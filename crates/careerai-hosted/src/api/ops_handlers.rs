//! Ops route handlers: metrics, readiness, artifact proxy.
//!
//! These endpoints serve the operational needs of the hosted beta:
//! Prometheus metrics exposition, health/readiness checks, and
//! capability-mediated artifact downloads.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::api::error::ApiError;
use crate::api::middleware::SessionAuth;
use crate::api::state::AppState;
use crate::ops::metrics::BetaExitSnapshot;

/// Readiness check — returns 200 if the service is ready to accept traffic.
pub async fn readiness(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Beta: always ready if the state exists.
    // In production: check DB connectivity, queue depth, etc.
    let _ = state;
    StatusCode::OK
}

/// Prometheus metrics endpoint.
pub async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let text = state.metrics.render_prometheus();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        text,
    )
}

/// Beta exit criteria snapshot.
pub async fn beta_exit_snapshot(
    State(state): State<Arc<AppState>>,
    _session: SessionAuth,
) -> Json<BetaExitSnapshot> {
    Json(state.metrics.beta_exit_snapshot())
}

/// Download an artifact by version ID with a capability token.
///
/// The capability is passed as a query parameter `?cap=<token>`.
/// The handler validates the capability, checks tenant scope, and
/// returns the object bytes.
pub async fn download_artifact(
    State(state): State<Arc<AppState>>,
    Path(version_id): Path<String>,
    axum::extract::Query(params): axum::extract::Query<DownloadParams>,
) -> Result<impl IntoResponse, ApiError> {
    let cap_str = params.cap.as_deref().unwrap_or("");
    let cap: crate::ops::object_store::Capability =
        serde_json::from_str(cap_str).map_err(|_| ApiError::bad_request("invalid capability"))?;

    let (metadata, bytes) = state
        .artifacts
        .download(&cap)
        .await
        .map_err(|_| ApiError::forbidden("artifact download"))?;

    // Verify the URL path matches the capability scope
    if metadata.version_id != version_id {
        return Err(ApiError::forbidden("capability scope mismatch"));
    }

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, metadata.content_type),
            (header::CONTENT_LENGTH, metadata.size.to_string()),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    ))
}

#[derive(Debug, serde::Deserialize)]
pub struct DownloadParams {
    pub cap: Option<String>,
}

/// List artifacts for the current tenant.
pub async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
) -> Json<Vec<crate::ops::object_store::ObjectVersion>> {
    let artifacts = state.artifacts.list_for_tenant(session.tenant_id()).await;
    Json(artifacts)
}

#[derive(Debug, Serialize)]
pub struct CapabilityResponse {
    pub capability: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Issue a download capability for an artifact.
pub async fn issue_artifact_capability(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
    Path(version_id): Path<String>,
) -> Result<Json<CapabilityResponse>, ApiError> {
    let cap = state
        .artifacts
        .issue_capability(
            session.tenant_id(),
            &version_id,
            session.user_id(),
            "download",
            300, // 5-minute TTL per design doc
        )
        .await
        .map_err(|_| ApiError::not_found("artifact"))?;

    let cap_json = serde_json::to_string(&cap)
        .map_err(|_| ApiError::bad_request("capability serialization"))?;

    Ok(Json(CapabilityResponse {
        capability: cap_json,
        expires_at: cap.expires_at,
    }))
}
