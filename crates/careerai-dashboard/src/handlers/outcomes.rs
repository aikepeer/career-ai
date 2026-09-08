//! F04 + F05: employer-outcome timeline and follow-up inbox handlers.
//!
//! These endpoints let the user record manual employer outcomes (applied,
//! replied, interview, rejection, offer, withdrawal) and manage the
//! follow-up inbox lifecycle (snooze, dismiss, handle, edit body) — all
//! without automatically sending any email.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::AppState;

// ─── F04: Employer outcomes ───────────────────────────────────────────

/// Request body for recording a manual employer outcome.
#[derive(Debug, Deserialize)]
pub struct RecordOutcomeRequest {
    pub application_id: Option<String>,
    pub listing_id: String,
    pub attempt_id: Option<i64>,
    pub outcome_type: String,
    pub occurred_at: String,
    pub note: Option<String>,
}

/// `POST /api/v1/outcomes` — record a manual employer outcome.
pub async fn api_record_outcome(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RecordOutcomeRequest>,
) -> impl IntoResponse {
    match careerai_db::queries::record_outcome(
        &state.pool,
        req.application_id.as_deref(),
        &req.listing_id,
        req.attempt_id,
        &req.outcome_type,
        &req.occurred_at,
        req.note.as_deref(),
    )
    .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "id": id })),
        )
            .into_response(),
        Err(e) => outcome_error_response(&e),
    }
}

/// `GET /api/v1/outcomes/:listing_id` — list outcomes for a listing.
pub async fn api_list_outcomes(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
) -> impl IntoResponse {
    match careerai_db::queries::outcomes_for_listing(&state.pool, &listing_id).await {
        Ok(outcomes) => (StatusCode::OK, Json(outcomes)).into_response(),
        Err(e) => outcome_error_response(&e),
    }
}

/// `GET /api/v1/outcomes` — list recent outcomes across all applications.
pub async fn api_list_recent_outcomes(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match careerai_db::queries::list_recent_outcomes(&state.pool, 50).await {
        Ok(outcomes) => (StatusCode::OK, Json(outcomes)).into_response(),
        Err(e) => outcome_error_response(&e),
    }
}

/// `DELETE /api/v1/outcomes/:id` — delete a misrecorded outcome.
pub async fn api_delete_outcome(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match careerai_db::queries::delete_outcome(&state.pool, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => outcome_error_response(&e),
    }
}

/// `GET /api/v1/outcomes/types` — list allowed outcome types.
pub async fn api_outcome_types() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "types": careerai_db::queries::OUTCOME_TYPES
        })),
    )
}

fn outcome_error_response(e: &careerai_db::DbError) -> axum::response::Response {
    let status = match e {
        careerai_db::DbError::NotFound(_) => StatusCode::NOT_FOUND,
        careerai_db::DbError::Conflict(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
        .into_response()
}

// ─── F05: Follow-up inbox lifecycle ──────────────────────────────────

/// Request body for snoozing a follow-up.
#[derive(Debug, Deserialize)]
pub struct SnoozeFollowUpRequest {
    pub new_scheduled_at: String,
}

/// Request body for editing a follow-up body.
#[derive(Debug, Deserialize)]
pub struct UpdateFollowUpBodyRequest {
    pub body: String,
}

/// `POST /api/v1/follow-ups/:id/snooze` — reschedule a pending follow-up.
pub async fn api_snooze_follow_up(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<SnoozeFollowUpRequest>,
) -> impl IntoResponse {
    match careerai_db::queries::snooze_follow_up(&state.pool, id, &req.new_scheduled_at).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => follow_up_error_response(&e),
    }
}

/// `POST /api/v1/follow-ups/:id/dismiss` — dismiss a pending follow-up.
pub async fn api_dismiss_follow_up(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match careerai_db::queries::dismiss_follow_up(&state.pool, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => follow_up_error_response(&e),
    }
}

/// `POST /api/v1/follow-ups/:id/handle` — mark a follow-up as handled.
pub async fn api_handle_follow_up(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    match careerai_db::queries::mark_follow_up_handled(&state.pool, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => follow_up_error_response(&e),
    }
}

/// `PUT /api/v1/follow-ups/:id/body` — edit the follow-up email body.
pub async fn api_update_follow_up_body(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateFollowUpBodyRequest>,
) -> impl IntoResponse {
    match careerai_db::queries::update_follow_up_body(&state.pool, id, &req.body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => follow_up_error_response(&e),
    }
}

fn follow_up_error_response(e: &careerai_db::DbError) -> axum::response::Response {
    let status = match e {
        careerai_db::DbError::NotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
        .into_response()
}

/// F05: a follow-up card with full editable body and company/title context.
/// This is the enriched view used by the follow-up inbox.
#[derive(Debug, Serialize)]
pub struct FollowUpInboxItem {
    pub id: i64,
    pub application_id: String,
    pub listing_id: String,
    pub company: String,
    pub title: String,
    pub status: String,
    pub scheduled_at: String,
    pub body: String,
    pub cadence_step: i64,
}
