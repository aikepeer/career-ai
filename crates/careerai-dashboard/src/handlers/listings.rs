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

    // R13: the read-check below is for UX (404 / already-shortlisted /
    // conflict). The actual state change uses `transition_if` so the
    // conditional UPDATE + event INSERT are atomic — a concurrent worker
    // cannot advance the listing between this check and the write. If
    // `transition_if` returns false the state changed under us; report
    // conflict rather than silently regressing it.
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

    match db_queries::transition_if(
        &state.pool,
        &id,
        &[ListingState::Discovered, ListingState::FilteredOut],
        ListingState::Shortlisted,
        Some("manually shortlisted from dashboard"),
    )
    .await
    {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "success", "id": id })),
        )
            .into_response(),
        Ok(false) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "listing state changed before shortlist; refresh and retry",
                "id": id,
            })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct DownloadArtifactQuery {
    pub path: String,
}

pub async fn api_download_artifact(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<DownloadArtifactQuery>,
) -> impl IntoResponse {
    let root = careerai_core::paths::resolve_root_env();
    let file_path = if std::path::Path::new(&q.path).is_absolute() {
        std::path::PathBuf::from(&q.path)
    } else {
        root.join(&q.path)
    };

    let canonical_root = match root.canonicalize() {
        Ok(r) => r,
        Err(_) => root.clone(),
    };
    let Ok(canonical_file) = file_path.canonicalize() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "File not found" })),
        )
            .into_response();
    };

    // Layer 1: the file must live under the career-ai root (defeats
    // `../../etc/passwd` and absolute-path escapes, including symlinks,
    // because `canonicalize` resolves them).
    if !canonical_file.starts_with(&canonical_root) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access denied" })),
        )
            .into_response();
    }

    // Layer 2: only files registered by the render pipeline may be
    // served. `data/credentials.json` and config files live under root
    // too, but are never registered as artifacts — this blocks
    // credential exfiltration through the download endpoint.
    let registered = match db_queries::all_artifact_paths(&state.pool).await {
        Ok(paths) => paths,
        Err(e) => {
            tracing::warn!(error = %e, "download: failed to list artifact paths");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Artifact registry unavailable" })),
            )
                .into_response();
        }
    };
    let requested = canonical_file.to_string_lossy().to_string();
    if !registered.iter().any(|p| p == &requested) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Not a registered artifact" })),
        )
            .into_response();
    }

    let Ok(contents) = tokio::fs::read(&canonical_file).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "File not found" })),
        )
            .into_response();
    };

    let filename = canonical_file.file_name().map_or_else(
        || "artifact".to_string(),
        |n| n.to_string_lossy().to_string(),
    );

    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let mime = match ext.to_ascii_lowercase().as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "md" => "text/markdown; charset=utf-8",
        _ => "application/octet-stream",
    };

    // Filenames derive from user profile / listing text; strip
    // characters that could break out of the quoted header value.
    let safe_filename: String = filename
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\r' | '\n' => '-',
            c => c,
        })
        .collect();

    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (
                axum::http::header::CONTENT_DISPOSITION,
                &format!("inline; filename=\"{safe_filename}\""),
            ),
        ],
        contents,
    )
        .into_response()
}
