//! Config generation preview/apply + prompt keyword extraction.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::AppState;

/// Derive the profile-backed config YAML from `profile/profile.yaml`.
/// Returns a `BAD_REQUEST`-shaped error string on unparseable profile data
/// so callers never overwrite `config/local.yaml` from a broken profile.
fn generated_config_for(cwd: &std::path::Path) -> Result<String, String> {
    let profile_path = careerai_core::paths::profile_path(cwd);
    let profile = if profile_path.exists() {
        let raw = std::fs::read_to_string(&profile_path)
            .map_err(|e| format!("read {} failed: {e}", profile_path.display()))?;
        serde_yaml::from_str(&raw)
            .map_err(|e| format!("parse {} failed: {e}", profile_path.display()))?
    } else {
        careerai_profile::schema::Profile::default()
    };
    Ok(careerai_profile::generate_config_yaml(&profile))
}

/// Resolve the shared preamble for the config preview/apply endpoints:
/// `config/local.yaml` plus the profile-derived generated config. Keeping
/// both error shapes here means preview and apply can never drift apart.
fn config_targets() -> Result<(PathBuf, String), (StatusCode, Json<serde_json::Value>)> {
    let cwd = careerai_core::paths::resolve_root_env();
    let local_cfg = cwd.join("config").join("local.yaml");
    let generated = generated_config_for(&cwd).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
    })?;
    Ok((local_cfg, generated))
}

/// Preview the merged config without writing. Returns the current file
/// (`before`) and the merged result (`after`) so the UI can show a
/// before/after diff and require an explicit "Apply" click.
pub async fn api_config_generate(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let (local_cfg, generated) = match config_targets() {
        Ok(targets) => targets,
        Err(resp) => return resp.into_response(),
    };
    let before = if local_cfg.exists() {
        std::fs::read_to_string(&local_cfg).unwrap_or_default()
    } else {
        String::new()
    };
    match crate::profile_handler::merge_generated_config_doc(&local_cfg, &generated) {
        Ok(after) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "preview",
                "pending": true,
                "before": before,
                "after": after,
                "generated": generated,
                "path": local_cfg.display().to_string(),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

/// Apply the previously previewed config generation: merge + backup +
/// atomic write. Kept separate from the preview endpoint so a preview
/// never mutates `config/local.yaml`.
pub async fn api_config_apply(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let (local_cfg, generated) = match config_targets() {
        Ok(targets) => targets,
        Err(resp) => return resp.into_response(),
    };
    match crate::profile_handler::merge_generated_config(&local_cfg, &generated) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!("Generated config merged into {}", local_cfg.display()),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct ConfigPromptRequest {
    pub prompt: String,
}

pub async fn api_config_prompt(Json(payload): Json<ConfigPromptRequest>) -> impl IntoResponse {
    let stopwords = [
        "focus", "on", "and", "for", "jobs", "in", "the", "a", "an", "with", "or", "that", "me",
        "my", "give", "show", "find", "want", "also", "add", "please", "more", "only", "just",
        "very", "need",
    ];
    let extracted: Vec<String> = payload
        .prompt
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
        .filter(|w| w.len() > 2 && !stopwords.contains(&w.as_str()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect();

    let additions = extracted.iter().fold(String::new(), |mut acc, k| {
        use std::fmt::Write;
        let _ = writeln!(acc, "    - \"{k}\"");
        acc
    });
    let response_text = format!(
        "Extracted {} keyword suggestions from prompt:\n{}",
        extracted.len(),
        additions
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "response": response_text,
            "extracted_keywords": extracted,
        })),
    )
        .into_response()
}
