//! Profile import (multipart upload → draft) and confirm handlers.

use std::sync::Arc;

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use crate::AppState;

const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

/// Reduce a multipart `filename` to a single, non-traversal file name.
/// Returns `None` for empty names, `.`/`..`, or names that still carry a
/// path separator (defense in depth on top of `Path::file_name`).
fn sanitize_upload_name(name: &str) -> Option<String> {
    let base = std::path::Path::new(name).file_name()?.to_str()?;
    if base.is_empty() || base == "." || base == ".." || base.contains(['/', '\\']) {
        return None;
    }
    Some(base.to_string())
}

pub async fn api_profile_import(
    State(_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let upload_dir = std::env::temp_dir().join("careerai_uploads");
    if let Err(e) = tokio::fs::create_dir_all(&upload_dir).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("upload dir create failed: {e}") })),
        )
            .into_response();
    }
    let mut saved_paths = Vec::new();

    while let Some(mut field) = multipart.next_field().await.unwrap_or(None) {
        let Some(filename) = sanitize_upload_name(field.file_name().unwrap_or("upload")) else {
            continue;
        };
        // Stream the field in chunks and enforce the size cap so a hostile
        // multipart body can neither escape the upload dir nor exhaust memory.
        let mut data = bytes::BytesMut::new();
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if data.len() + chunk.len() > MAX_UPLOAD_BYTES {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            Json(serde_json::json!({ "error": "upload exceeds size limit" })),
                        )
                            .into_response();
                    }
                    data.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": format!("read upload failed: {e}") })),
                    )
                        .into_response();
                }
            }
        }
        if data.is_empty() {
            continue;
        }
        let path = upload_dir.join(&filename);
        if tokio::fs::write(&path, &data).await.is_ok() {
            saved_paths.push(path);
        }
    }

    if saved_paths.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "No files uploaded" })),
        )
            .into_response();
    }

    let refs: Vec<&std::path::Path> = saved_paths
        .iter()
        .map(std::path::PathBuf::as_path)
        .collect();
    match crate::profile_llm::import_paths_best_effort(&refs).await {
        Ok(profile) => {
            // Persist the full extracted profile (including experience /
            // education / projects, which the form does not surface) to a
            // *draft* file. Replacing the live `profile/profile.yaml` is a
            // separate, explicit confirmation step so an accidental or
            // stale upload can never clobber a hand-curated profile.
            let save_result = crate::profile_handler::save_profile_draft(&profile).await;
            let mut body = serde_json::json!({
                "status": "success",
                "profile": profile,
                "draft": true,
                "pending": true,
            });
            match save_result {
                Ok(path) => {
                    body["draft_path"] = serde_json::json!(path.display().to_string());
                }
                Err(e) => {
                    tracing::warn!(
                        target = "dashboard.profile_import",
                        error = %e,
                        "imported profile draft could not be persisted",
                    );
                    body["draft"] = serde_json::json!(false);
                    body["save_error"] = serde_json::json!(e);
                }
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Promote the pending import draft to the live profile. This is the only
/// place an imported profile can replace `profile/profile.yaml`, and it
/// always backs up the previous profile first.
pub async fn api_profile_import_confirm() -> impl IntoResponse {
    match crate::profile_handler::confirm_profile_import() {
        Ok(path) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!("Imported profile applied to {}", path.display()),
                "saved_to": path.display().to_string(),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}
