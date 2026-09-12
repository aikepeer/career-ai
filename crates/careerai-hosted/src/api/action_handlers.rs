//! Action route handlers: approve_action, submit_action,
//! create_export, delete_account.
//!
//! All sensitive operations require reauthentication (session active
//! within the last 5 minutes) and role-based authorization.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::action::ActionState;
use crate::api::error::ApiError;
use crate::api::middleware::SessionAuth;
use crate::api::state::AppState;
use crate::auth::roles::{Permission, Role};
use crate::auth::session::SessionManager;
use crate::export::{check_reauth, encrypt_export, ExportBundle};

/// Check reauth + role permission for a sensitive operation.
fn authorize_sensitive(session: &SessionAuth, permission: Permission) -> Result<Role, ApiError> {
    let role = match session.role_str() {
        "owner" => Role::Owner,
        "support_readonly" => Role::SupportReadonly,
        _ => return Err(ApiError::forbidden("unknown role")),
    };
    let decision = role.check(permission);
    if !decision.allowed {
        return Err(ApiError::forbidden("insufficient role"));
    }
    if decision.requires_reauth
        && SessionManager::requires_reauth(&session.record, chrono::Utc::now())
    {
        return Err(ApiError::reauth_required());
    }
    Ok(role)
}

// ── Approve action ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub action_id: String,
    pub state: ActionState,
    pub payload_digest: String,
}

pub async fn approve_action(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
    Path(action_id): Path<String>,
) -> Result<Json<ActionResponse>, ApiError> {
    authorize_sensitive(&session, Permission::ApproveSideEffect)?;

    let mut actions = state.actions.write().await;
    let action = actions
        .get_mut(&action_id)
        .ok_or_else(|| ApiError::not_found("action"))?;
    action
        .approve(session.user_id(), chrono::Utc::now())
        .map_err(|e| ApiError::bad_request(&format!("approve failed: {e}")))?;

    Ok(Json(ActionResponse {
        action_id: action.action_id.clone(),
        state: action.state,
        payload_digest: action.payload_digest.clone(),
    }))
}

// ── Submit action ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SubmitResponse {
    pub action_id: String,
    pub state: ActionState,
    pub message: String,
}

pub async fn submit_action(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
    Path(action_id): Path<String>,
) -> Result<Json<SubmitResponse>, ApiError> {
    authorize_sensitive(&session, Permission::ApproveSideEffect)?;

    let mut actions = state.actions.write().await;
    let action = actions
        .get_mut(&action_id)
        .ok_or_else(|| ApiError::not_found("action"))?;

    // Lease + begin execution
    let _lease_token = action
        .lease("worker-1", chrono::Utc::now())
        .map_err(|e| ApiError::bad_request(&format!("lease failed: {e}")))?;
    action
        .begin_execution(1, 1, "worker-1", chrono::Utc::now())
        .map_err(|e| ApiError::bad_request(&format!("begin failed: {e}")))?;

    // In the beta, submission is dry-run only — no live network write.
    // The action succeeds with a "would_submit" outcome.
    action
        .succeed("worker-1", chrono::Utc::now())
        .map_err(|e| ApiError::bad_request(&format!("succeed failed: {e}")))?;

    Ok(Json(SubmitResponse {
        action_id: action.action_id.clone(),
        state: action.state,
        message: "dry-run: would_submit (no network write)".to_string(),
    }))
}

// ── Export ───────────────────────────────────────────────────────────

pub async fn create_export(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
) -> Result<Json<ExportBundle>, ApiError> {
    authorize_sensitive(&session, Permission::ExportDelete)?;

    // Reauth check via export module
    check_reauth(&session.record, chrono::Utc::now()).map_err(|_| ApiError::reauth_required())?;

    // Export tenant data (beta: export session metadata)
    let export_data = serde_json::json!({
        "tenant_id": session.tenant_id(),
        "user_id": session.user_id(),
        "workspace_id": session.workspace_id(),
        "exported_at": chrono::Utc::now(),
    });
    let plaintext = serde_json::to_vec(&export_data)
        .map_err(|e| ApiError::bad_request(&format!("serialization: {e}")))?;

    let bundle = encrypt_export(&plaintext, &state.master_key, session.tenant_id())
        .map_err(|_| ApiError::bad_request("encryption failed"))?;

    Ok(Json(bundle))
}

// ── Delete account ──────────────────────────────────────────────────

pub async fn delete_account(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
) -> Result<StatusCode, ApiError> {
    authorize_sensitive(&session, Permission::ExportDelete)?;

    // Revoke session
    let token_hash = SessionManager::hash_token(&session.raw_token);
    let mut sessions = state.sessions.write().await;
    if let Some(record) = sessions.get_mut(&token_hash) {
        record.revoked = true;
    }
    // In production: mark account for deletion with grace period.
    // Beta: just revoke the session.
    drop(sessions);

    Ok(StatusCode::NO_CONTENT)
}
