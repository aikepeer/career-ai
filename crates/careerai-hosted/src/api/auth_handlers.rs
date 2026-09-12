//! Auth route handlers: start_login, login_callback, logout, recover.
//!
//! Flow: start_login → magic link emailed (dev: returned in response) →
//! login_callback verifies link → first call enrolls TOTP → second call
//! (new link + TOTP code) creates session + workspace + recovery codes.
//! Recovery: verify recovery code → create session.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::error::ApiError;
use crate::api::middleware::SessionAuth;
use crate::api::state::AppState;
use crate::auth::magic_link::{DeviceContext, MagicLinkToken};
use crate::auth::rate_limit::{RateLimitConfig, RateLimitDecision};
use crate::auth::session::SessionManager;
use crate::auth::totp::Totp;
use crate::auth::workspace::{Workspace, WorkspaceMembership};

// ── Request / Response types ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StartLoginRequest {
    pub email: String,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartLoginResponse {
    pub login_tx_id: String,
    pub expires_at: chrono::DateTime<Utc>,
    /// Dev-mode: the raw magic link token (emailed in production).
    pub magic_link_token: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginCallbackRequest {
    pub token: String,
    pub totp_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status")]
pub enum LoginCallbackResponse {
    /// TOTP enrollment required (first login).
    #[serde(rename = "totp_enrollment")]
    TotpEnrollment {
        base32_secret: String,
        qr_uri: String,
    },
    /// Fully authenticated session created.
    #[serde(rename = "authenticated")]
    Authenticated {
        session_token: String,
        workspace_id: String,
        role: String,
        /// Recovery codes (shown only on first login).
        recovery_codes: Option<Vec<String>>,
    },
}

#[derive(Debug, Deserialize)]
pub struct RecoverRequest {
    pub email: String,
    recovery_code: String,
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Extract device context from request headers.
fn device_context(headers: &HeaderMap) -> DeviceContext {
    let ua = headers
        .get("user-agent")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown");
    let ip = headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|h| h.to_str().ok())
        .unwrap_or("127.0.0.1");
    DeviceContext::from_request(ua, ip)
}

/// Extract client IP for rate limiting.
fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|h| h.to_str().ok())
        .unwrap_or("127.0.0.1")
        .to_string()
}

/// Create workspace + membership + session for a newly authenticated user.
async fn create_user_session(
    state: &AppState,
    email: &str,
    workspace_id: Option<String>,
) -> (String, String, Vec<String>) {
    let user_id = Uuid::new_v4().to_string();
    let tenant_id = Uuid::new_v4().to_string();

    // Create personal workspace
    let ws = Workspace::new(&tenant_id, email, &user_id);
    let ws_id = workspace_id.unwrap_or_else(|| ws.id.clone());
    let membership = WorkspaceMembership::owner(&ws_id, &user_id);

    state.workspaces.write().await.insert(ws_id.clone(), ws);
    state
        .memberships
        .write()
        .await
        .insert(user_id.clone(), vec![membership]);

    // Generate recovery codes
    let (raw_codes, code_set) = crate::auth::recovery::RecoveryCodeSet::generate();
    state
        .recovery_codes
        .write()
        .await
        .insert(email.to_string(), code_set);

    // Create session
    let (raw_token, record) = SessionManager::create(&tenant_id, &user_id, &ws_id, "owner");
    state
        .sessions
        .write()
        .await
        .insert(record.token_hash.clone(), record);

    (raw_token, ws_id, raw_codes)
}

// ── Handlers ─────────────────────────────────────────────────────────

pub async fn start_login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<StartLoginRequest>,
) -> Result<Json<StartLoginResponse>, ApiError> {
    // Rate limit
    let ip = client_ip(&headers);
    let rl_key = format!("{ip}:login");
    {
        let mut rl = state.rate_limiter.write().await;
        if rl.check(&rl_key, &RateLimitConfig::login()) == RateLimitDecision::Deny {
            return Err(ApiError::rate_limited());
        }
    }

    let ctx = device_context(&headers);
    let login_tx_id = Uuid::new_v4().to_string();

    let (raw_token, record) =
        MagicLinkToken::generate(&req.email, ctx, &login_tx_id, req.workspace_id);
    let expires_at = record.expires_at;

    // Store the token record and dev-mode raw token
    state
        .magic_links
        .write()
        .await
        .insert(record.token_hash.clone(), record);
    state
        .dev_tokens
        .write()
        .await
        .insert(login_tx_id.clone(), raw_token.clone());

    Ok(Json(StartLoginResponse {
        login_tx_id,
        expires_at,
        magic_link_token: raw_token,
    }))
}

pub async fn login_callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<LoginCallbackRequest>,
) -> Result<Json<LoginCallbackResponse>, ApiError> {
    let ctx = device_context(&headers);

    // Hash the provided token and look up the magic link record
    let token_hash = SessionManager::hash_token(&req.token);

    let mut magic_links = state.magic_links.write().await;
    let record = magic_links
        .get_mut(&token_hash)
        .ok_or_else(ApiError::unauthorized)?;

    // Verify the token (checks hash, expiry, single-use, device context)
    record
        .verify(&req.token, &ctx, Utc::now())
        .map_err(|_| ApiError::unauthorized())?;
    record.mark_used();
    let email = record.email.clone();
    drop(magic_links);

    // Check if TOTP is enrolled for this email
    let totp_enrolled = state.totp_secrets.read().await.contains_key(&email);

    if !totp_enrolled {
        // First login: enroll TOTP, return secret (no session yet)
        let (b32_secret, totp) = Totp::generate();
        state.totp_secrets.write().await.insert(email.clone(), totp);

        let qr_uri =
            format!("otpauth://totp/career-ai:{email}?secret={b32_secret}&issuer=career-ai");

        return Ok(Json(LoginCallbackResponse::TotpEnrollment {
            base32_secret: b32_secret,
            qr_uri,
        }));
    }

    // TOTP enrolled: verify the code
    let totp_code = req
        .totp_code
        .ok_or_else(|| ApiError::bad_request("TOTP code required"))?;

    let totp_secrets = state.totp_secrets.read().await;
    let totp = totp_secrets
        .get(&email)
        .ok_or_else(ApiError::unauthorized)?;
    totp.verify_now(&totp_code)
        .map_err(|_| ApiError::bad_request("Invalid TOTP code"))?;
    drop(totp_secrets);

    // Create session + workspace + recovery codes
    let (session_token, workspace_id, recovery_codes) =
        create_user_session(&state, &email, None).await;

    Ok(Json(LoginCallbackResponse::Authenticated {
        session_token,
        workspace_id,
        role: "owner".to_string(),
        recovery_codes: Some(recovery_codes),
    }))
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
) -> Result<axum::http::StatusCode, ApiError> {
    let token_hash = SessionManager::hash_token(&session.raw_token);
    let mut sessions = state.sessions.write().await;
    if let Some(record) = sessions.get_mut(&token_hash) {
        record.revoked = true;
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn recover(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<RecoverRequest>,
) -> Result<Json<LoginCallbackResponse>, ApiError> {
    // Rate limit (more aggressive)
    let ip = client_ip(&headers);
    let rl_key = format!("{ip}:recovery");
    {
        let mut rl = state.rate_limiter.write().await;
        if rl.check(&rl_key, &RateLimitConfig::recovery()) == RateLimitDecision::Deny {
            return Err(ApiError::rate_limited());
        }
    }

    // Verify recovery code
    let mut recovery_codes = state.recovery_codes.write().await;
    let code_set = recovery_codes
        .get_mut(&req.email)
        .ok_or_else(|| ApiError::bad_request("No recovery codes for this email"))?;
    code_set
        .verify(&req.recovery_code)
        .map_err(|_| ApiError::bad_request("Invalid recovery code"))?;
    drop(recovery_codes);

    // Create session
    let (session_token, workspace_id, recovery_codes) =
        create_user_session(&state, &req.email, None).await;

    Ok(Json(LoginCallbackResponse::Authenticated {
        session_token,
        workspace_id,
        role: "owner".to_string(),
        recovery_codes: Some(recovery_codes),
    }))
}
