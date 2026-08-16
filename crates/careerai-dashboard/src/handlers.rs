use std::fmt::Write as _;
use std::sync::Arc;
use axum::extract::Multipart;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    Json,
};

use crate::data;
use crate::details;
use crate::next_steps;
use crate::view;
use crate::view::IndexView;
use crate::AppState;

pub async fn index(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match render_index(&state).await {
        Ok(body) => (StatusCode::OK, Html(body)).into_response(),
        Err(err) => {
            let mut chain = format!("{err}");
            let mut src = std::error::Error::source(&err);
            while let Some(s) = src {
                let _ = write!(chain, " :: {s}");
                src = s.source();
            }
            tracing::error!(error = %chain, "dashboard index render failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(fallback_error_page()),
            )
                .into_response()
        }
    }
}

async fn render_index(state: &AppState) -> crate::error::Result<String> {
    let (snap_res, daemon_health, llm_health, events_res, config_res, actions_res, explorer_res) = tokio::join!(
        data::snapshot(&state.pool),
        crate::daemon_health::probe(),
        crate::llm_health::probe(),
        details::fetch_recent_events(&state.pool, 50),
        details::fetch_config_view(&state.pool),
        details::fetch_action_center(&state.pool),
        details::fetch_discovered_explorer(&state.pool, 10000),
    );
    let snap = snap_res?;
    let recent_events = events_res.unwrap_or_default();
    let config = config_res.unwrap_or_else(|_| view::ConfigView {
        score_threshold: 0.70,
        must_include_skills: vec!["Rust".into(), "Python".into()],
        keywords: Vec::new(),
        sources: Vec::new(),
        llm_provider: "Anthropic / Claude Web".into(),
        llm_model: "claude-3-5-sonnet".into(),
        llm_status: "healthy".into(),
        rate_limit_per_min: 60,
        prompt_version: "v1.2.0".into(),
        profile: None,
    });
    let action_items = actions_res.unwrap_or_default();
    let discovered_explorer = explorer_res.unwrap_or_default();
    let next_steps = next_steps::compute(&snap);

    let view = IndexView {
        kpi: snap.kpi,
        columns: snap.columns,
        next_steps,
        daemon_health,
        llm_health,
        recent_events,
        config,
        action_items,
        discovered_explorer,
    };
    let mut ctx = tera::Context::new();
    ctx.insert("view", &view);
    ctx.insert("refresh_seconds", &state.refresh_seconds);
    ctx.insert("css", crate::STYLE_CSS);
    ctx.insert("build_version", crate::BUILD_VERSION);
    Ok(state.tera.render("index.tera", &ctx)?)
}

pub async fn api_snapshot(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match data::snapshot(&state.pool).await {
        Ok(snap) => (StatusCode::OK, Json(snap)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

pub async fn api_events(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match details::fetch_recent_events(&state.pool, 50).await {
        Ok(events) => (StatusCode::OK, Json(events)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

pub async fn api_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match details::fetch_config_view(&state.pool).await {
        Ok(cfg) => (StatusCode::OK, Json(cfg)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct SaveConfigRequest {
    pub backend: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_base_url: Option<String>,
    pub api_key: Option<String>,
    pub timeout_seconds: Option<u64>,
}

pub async fn api_save_config(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<SaveConfigRequest>,
) -> impl IntoResponse {
    if let Some(ref key) = payload.api_key {
        if !key.trim().is_empty() {
            std::env::set_var("LLM_API_KEY", key.trim());
            std::env::set_var("CAREERAI_LLM_API_KEY", key.trim());
            std::env::set_var("DEEPSEEK_API_KEY", key.trim());
            std::env::set_var("OPENAI_API_KEY", key.trim());
            std::env::set_var("ANTHROPIC_API_KEY", key.trim());
        }
    }
    if let Some(ref base_url) = payload.api_base_url {
        if !base_url.trim().is_empty() {
            std::env::set_var("LLM_API_BASE_URL", base_url.trim());
            std::env::set_var("CAREERAI_LLM_API_BASE_URL", base_url.trim());
            std::env::set_var("DEEPSEEK_API_BASE_URL", base_url.trim());
            std::env::set_var("OPENAI_API_BASE_URL", base_url.trim());
        }
    }
    if let Some(ref model) = payload.model {
        if !model.trim().is_empty() {
            std::env::set_var("LLM_MODEL", model.trim());
            std::env::set_var("CAREERAI_LLM_MODEL", model.trim());
        }
    }
    if let Some(ref provider) = payload.provider {
        if !provider.trim().is_empty() {
            std::env::set_var("LLM_PROVIDER", provider.trim());
            std::env::set_var("CAREERAI_LLM_PROVIDER", provider.trim());
        }
    }
    if let Some(timeout) = payload.timeout_seconds {
        if timeout > 0 {
            let s = timeout.to_string();
            std::env::set_var("LLM_TIMEOUT_SECONDS", &s);
            std::env::set_var("CAREERAI_LLM_TIMEOUT", &s);
        }
    }
    if !payload.backend.trim().is_empty() {
        std::env::set_var("CAREERAI_LLM_BACKEND", payload.backend.trim());
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": "Configuration updated successfully",
            "backend": payload.backend,
            "provider": payload.provider,
            "model": payload.model,
            "api_base_url": payload.api_base_url,
        })),
    )
        .into_response()
}

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

pub async fn healthz() -> &'static str {
    "ok\n"
}

pub async fn api_explorer(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match details::fetch_discovered_explorer(&state.pool, 10000).await {
        Ok(items) => (StatusCode::OK, Json(items)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

pub async fn api_config_generate(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("cwd error: {e}") })),
        ).into_response(),
    };
    let profile_path = cwd.join("profile").join("profile.yaml");
    let local_cfg = cwd.join("config").join("local.yaml");
    let profile = if profile_path.exists() {
        let raw = std::fs::read_to_string(&profile_path).unwrap_or_default();
        serde_yaml::from_str(&raw).unwrap_or_default()
    } else {
        careerai_profile::schema::Profile::default()
    };
    let generated = careerai_profile::generate_config_yaml(&profile);
    match std::fs::write(&local_cfg, &generated) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({
            "status": "success",
            "message": format!("Generated config at {}", local_cfg.display()),
            "lines": generated.lines().count(),
        }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": format!("write failed: {e}")
        }))).into_response(),
    }
}

pub async fn api_profile_import(
    State(_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let upload_dir = std::env::temp_dir().join("careerai_uploads");
    let _ = std::fs::create_dir_all(&upload_dir);
    let mut saved_paths = Vec::new();

    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let filename = field.file_name().unwrap_or("upload").to_owned();
        if filename.is_empty() {
            continue;
        }
        let data = field.bytes().await.unwrap_or_else(|_| bytes::Bytes::new());
        if data.is_empty() {
            continue;
        }
        let path = upload_dir.join(&filename);
        if std::fs::write(&path, &data).is_ok() {
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

    let refs: Vec<&std::path::Path> = saved_paths.iter().map(|p| p.as_path()).collect();
    match careerai_profile::import_paths(&refs) {
        Ok(profile) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "success", "profile": profile })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}


pub async fn api_force_shortlist(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match sqlx::query("UPDATE listings SET state = 'shortlisted' WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
    {
        Ok(_) => (
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

#[derive(Debug, serde::Deserialize)]
pub struct ConfigPromptRequest {
    pub prompt: String,
}

pub async fn api_config_prompt(
    Json(payload): Json<ConfigPromptRequest>,
) -> impl IntoResponse {
    let stopwords = [
        "focus", "on", "and", "for", "jobs", "in", "the", "a", "an", "with", "or", "that",
        "me", "my", "give", "show", "find", "want", "also", "add", "please", "more", "only",
        "just", "very", "need",
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

    let additions: String = extracted
        .iter()
        .map(|k| format!("    - \"{k}\"\n"))
        .collect();
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

fn fallback_error_page() -> String {
    "<!doctype html><html><body><h1>career-ai</h1>\
     <p>Data temporarily unavailable. Check daemon logs.</p>\
     </body></html>"
        .to_string()
}
