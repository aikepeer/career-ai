//! Workspace route handlers: list_workspaces, switch_workspace.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;

use crate::api::error::ApiError;
use crate::api::middleware::SessionAuth;
use crate::api::state::AppState;
use crate::auth::session::SessionManager;
use crate::auth::workspace::validate_switch;

#[derive(Debug, Serialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub display_name: String,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct SwitchWorkspaceResponse {
    pub session_token: String,
    pub workspace_id: String,
}

pub async fn list_workspaces(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
) -> Result<Json<Vec<WorkspaceInfo>>, ApiError> {
    let memberships = state.memberships.read().await;
    let user_memberships = memberships
        .get(session.user_id())
        .cloned()
        .unwrap_or_default();
    drop(memberships);

    let workspaces = state.workspaces.read().await;
    let result: Vec<WorkspaceInfo> = user_memberships
        .iter()
        .filter_map(|m| {
            workspaces.get(&m.workspace_id).map(|ws| WorkspaceInfo {
                id: ws.id.clone(),
                display_name: ws.display_name.clone(),
                role: format!("{:?}", m.role).to_lowercase(),
            })
        })
        .collect();

    Ok(Json(result))
}

pub async fn switch_workspace(
    State(state): State<Arc<AppState>>,
    session: SessionAuth,
    Path(target_id): Path<String>,
) -> Result<Json<SwitchWorkspaceResponse>, ApiError> {
    let memberships = state.memberships.read().await;
    let user_memberships = memberships
        .get(session.user_id())
        .cloned()
        .unwrap_or_default();
    drop(memberships);

    let _membership = validate_switch(&user_memberships, &target_id)
        .map_err(|_| ApiError::forbidden("workspace switch"))?;

    // Rotate session with new workspace
    let (new_token, new_record) = SessionManager::rotate(&session.record);
    let mut new_record = new_record;
    new_record.workspace_id = target_id.clone();

    // Update the session store: remove old, insert new
    let old_hash = SessionManager::hash_token(&session.raw_token);
    let mut sessions = state.sessions.write().await;
    sessions.remove(&old_hash);
    sessions.insert(new_record.token_hash.clone(), new_record);
    drop(sessions);

    Ok(Json(SwitchWorkspaceResponse {
        session_token: new_token,
        workspace_id: target_id,
    }))
}
