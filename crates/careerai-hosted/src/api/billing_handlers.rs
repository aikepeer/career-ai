//! Billing route handlers: billing_checkout, billing_webhook.
//!
//! Webhook handler verifies HMAC-SHA256 signatures, deduplicates by
//! (provider, event_id), and processes events in order.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::api::middleware::SessionAuth;
use crate::api::state::AppState;
use crate::entitlement::billing::{process_event, verify_signature, BillingEvent, BillingEventStatus};

// ── Checkout ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    pub plan_id: String,
}

#[derive(Debug, Serialize)]
pub struct CheckoutResponse {
    pub checkout_url: String,
    pub session_id: String,
}

pub async fn billing_checkout(
    State(_state): State<Arc<AppState>>,
    _session: SessionAuth,
    Json(req): Json<CheckoutRequest>,
) -> Result<Json<CheckoutResponse>, ApiError> {
    // Beta: return a mock checkout URL.
    // In production this creates a real provider checkout session.
    let session_id = uuid::Uuid::new_v4().to_string();
    let checkout_url = format!(
        "https://checkout.provider.example.com/c/{}?plan={}",
        session_id, req.plan_id
    );
    Ok(Json(CheckoutResponse {
        checkout_url,
        session_id,
    }))
}

// ── Webhook ─────────────────────────────────────────────────────────

pub async fn billing_webhook(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, ApiError> {
    // Extract signature from header
    let signature = headers
        .get("x-webhook-signature")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(ApiError::unauthorized)?;

    // Look up webhook secret for this provider
    let secret = {
        let secrets = state.webhook_secrets.read().await;
        secrets
            .get(&provider)
            .cloned()
            .ok_or_else(ApiError::unauthorized)?
    };

    // Verify signature
    verify_signature(&body, signature, &secret)
        .map_err(|_| ApiError::unauthorized())?;

    // Parse the event
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| ApiError::bad_request(&format!("invalid JSON: {e}")))?;

    let event_id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("missing event id"))?
        .to_string();

    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let event_version = payload
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .map_or(1, |v| u32::try_from(v).unwrap_or(1));

    let customer_id = payload
        .get("customer")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let subscription_id = payload
        .get("subscription")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Dedup check
    {
        let mut dedup = state.webhook_dedup.write().await;
        dedup
            .check(&provider, &event_id)
            .map_err(|_| ApiError::conflict("duplicate webhook event"))?;
    }

    // Process event
    let mut event = BillingEvent {
        provider: provider.clone(),
        event_id,
        event_type,
        event_version,
        customer_id,
        subscription_id,
        amount_cents: 0,
        currency: "usd".to_string(),
        occurred_at: chrono::Utc::now(),
        received_at: chrono::Utc::now(),
        status: BillingEventStatus::Pending,
        raw_payload: body,
    };

    process_event(&mut event, 0)
        .map_err(|e| ApiError::bad_request(&format!("processing: {e}")))?;

    Ok(StatusCode::OK)
}
