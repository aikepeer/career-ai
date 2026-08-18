//! Listing/application detail and manual shortlist handlers.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use careerai_core::state::ListingState;
use careerai_db::queries as db_queries;

use crate::details;
use crate::AppState;

pub async fn api_application_detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match details::fetch_application_detail(&state.pool, &id).await {
        Ok(Some(detail)) => (StatusCode::OK, Json(detail)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Application not found" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

pub async fn api_force_shortlist(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if id.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "listing id is required" })),
        )
            .into_response();
    }

    // Read the current state first so we never move a terminal listing
    // (submitted/failed/skipped/responded) backwards through the pipeline.
    let listing = match db_queries::find_by_id(&state.pool, &id).await {
        Ok(l) => l,
        Err(careerai_db::error::DbError::NotFound(_)) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "listing not found", "id": id })),
            )
                .into_response();
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    let current: ListingState = match listing.state.parse() {
        Ok(state) => state,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("listing has unknown state: {err}") })),
            )
                .into_response();
        }
    };

    if current == ListingState::Shortlisted {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "success", "id": id, "already_shortlisted": true })),
        )
            .into_response();
    }
    if !matches!(
        current,
        ListingState::Discovered | ListingState::FilteredOut
    ) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("cannot shortlist a listing in state `{current}`"),
                "id": id,
            })),
        )
            .into_response();
    }

    match db_queries::transition(
        &state.pool,
        &id,
        ListingState::Shortlisted,
        Some("manually shortlisted from dashboard"),
    )
    .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "success", "id": id })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}
