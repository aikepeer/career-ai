//! Dashboard HTTP handlers.

pub mod config_gen;
pub mod listings;
pub mod profile_import;

pub use config_gen::{api_config_apply, api_config_generate, api_config_prompt};
pub use listings::{api_application_detail, api_force_shortlist};
pub use profile_import::{api_profile_import, api_profile_import_confirm};

use std::fmt::Write as _;
use std::sync::Arc;

use axum::{
    extract::State,
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
        llm_backend: "auto".into(),
        llm_api_base: None,
        llm_timeout_seconds: 300,
    });
    let action_items = actions_res.unwrap_or_default();
    let discovered_explorer = explorer_res.unwrap_or_default();
    let next_steps = next_steps::compute(&snap);
    let guided = crate::guided::compute(&snap, &config);
    let cli_groups = crate::cli_catalog::grouped();

    let view = IndexView {
        kpi: snap.kpi,
        columns: snap.columns,
        state_counts: snap.state_counts,
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
    ctx.insert("guided", &guided);
    ctx.insert("cli_groups", &cli_groups);
    ctx.insert("refresh_seconds", &state.refresh_seconds);
    ctx.insert("css", crate::STYLE_CSS);
    ctx.insert("build_version", crate::BUILD_VERSION);
    Ok(state.tera.render("index.html", &ctx)?)
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
    let cwd = careerai_core::paths::resolve_root_env();
    let config_path = cwd.join("config").join("local.yaml");

    // Persist to the layered config file instead of mutating process env.
    // Env vars are lost on restart, visible to same-user processes via
    // /proc/<pid>/environ, and the previous code wrote one key to five
    // provider-specific variables, silently mislabeling e.g. a DeepSeek key
    // as an Anthropic key.
    match crate::profile_handler::update_llm_settings_in_config(
        &config_path,
        &payload.backend,
        payload.provider.as_deref(),
        payload.model.as_deref(),
        payload.api_base_url.as_deref(),
        payload.api_key.as_deref(),
        payload.timeout_seconds,
    ) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!("Configuration updated at {}", config_path.display()),
                "backend": payload.backend,
                "provider": payload.provider,
                "model": payload.model,
                "api_base_url": payload.api_base_url,
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

pub async fn healthz() -> &'static str {
    "ok\n"
}

fn fallback_error_page() -> String {
    "<!doctype html><html><body><h1>career-ai</h1>\
     <p>Data temporarily unavailable. Check daemon logs.</p>\
     </body></html>"
        .to_string()
}
