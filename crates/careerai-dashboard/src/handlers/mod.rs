//! Dashboard HTTP handlers.

#![allow(clippy::cast_precision_loss, clippy::similar_names)]

pub mod config_gen;
pub mod listings;
pub mod outcomes;
pub mod profile_import;

pub use config_gen::{api_config_apply, api_config_generate, api_config_prompt};
pub use listings::{api_application_detail, api_download_artifact, api_force_shortlist};
pub use outcomes::{
    api_delete_outcome, api_dismiss_follow_up, api_handle_follow_up, api_list_outcomes,
    api_list_recent_outcomes, api_outcome_types, api_record_outcome, api_snooze_follow_up,
    api_update_follow_up_body,
};
pub use profile_import::{api_profile_import, api_profile_import_confirm};

use std::fmt::Write as _;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse},
    Json,
};
use careerai_core::state::ListingState;

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
    // Explorer rows are NOT fetched here — loading 10k listings blocks
    // the page render. Instead we fetch a cheap COUNT for the badge and
    // let client-side JS load the rows from /api/v1/explorer after the
    // page paints. The initial `tokio::join!` stays fast.
    let (
        snap_res,
        daemon_health,
        llm_health,
        events_res,
        config_res,
        actions_res,
        explorer_count_res,
        content_lib_res,
    ) = tokio::join!(
        data::snapshot(&state.pool),
        crate::daemon_health::probe(),
        crate::llm_health::probe(),
        details::fetch_recent_events(&state.pool, 50),
        details::fetch_config_view(&state.pool),
        details::fetch_action_center(&state.pool),
        details::fetch_explorer_count(&state.pool),
        data::content_library_summary(&state.pool),
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
        llm_strategy: "local".into(),
        llm_api_base: None,
        llm_timeout_seconds: 300,
    });
    let action_items = actions_res.unwrap_or_default();
    let explorer_total = explorer_count_res.unwrap_or(0);
    let content_library = content_lib_res.unwrap_or_default();
    let next_steps = next_steps::compute(&snap);
    let guided = crate::guided::compute(&snap, &config);
    let cli_groups = crate::cli_catalog::grouped();

    // LLM cost summary — reads costs.jsonl from the project root.
    let cwd = careerai_core::paths::resolve_root_env();
    let llm_cost = crate::llm_costs::cost_summary(&cwd).await;

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
        discovered_explorer: Vec::new(),
        explorer_total,
        explorer_lazy: true,
        llm_cost,
        content_library,
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
    pub strategy: Option<String>,
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
        payload.strategy.as_deref(),
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

/// R17: Explorer with server-side filters and pagination. Query params:
/// - `q`: search title + company (case-insensitive)
/// - `source`: filter by source (e.g. "greenhouse")
/// - `state`: filter by listing state
/// - `remote`: "1" to show only remote listings
/// - `limit`: page size (default 100, max 500)
/// - `offset`: pagination offset
/// Returns `{ items: [...], total: N, limit, offset }` so the client can
/// distinguish loaded rows from total matches.
pub async fn api_explorer(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ExplorerQueryParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let offset = params.offset.unwrap_or(0);
    let filter = details::ExplorerFilter {
        query: params.q.filter(|q| !q.is_empty()),
        source: params.source.filter(|s| !s.is_empty()),
        state: params.state.filter(|s| !s.is_empty()),
        remote_only: params.remote.as_deref() == Some("1"),
        limit,
        offset,
    };

    let (items_result, count_result) = tokio::join!(
        details::fetch_explorer_filtered(&state.pool, &filter),
        details::fetch_explorer_filtered_count(&state.pool, &filter),
    );

    match (items_result, count_result) {
        (Ok(items), Ok(total)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "items": items,
                "total": total,
                "limit": limit,
                "offset": offset,
            })),
        )
            .into_response(),
        (Err(e), _) | (_, Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// R17: query parameters for the explorer endpoint.
#[derive(Debug, serde::Deserialize)]
pub struct ExplorerQueryParams {
    pub q: Option<String>,
    pub source: Option<String>,
    pub state: Option<String>,
    pub remote: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
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

#[derive(Debug, serde::Deserialize)]
pub struct SaveThresholdRequest {
    pub score_threshold: f32,
}

/// `POST /api/config/threshold` — update `matching.score_threshold` in
/// `config/local.yaml`. Preserves all other config keys.
pub async fn api_save_threshold(
    State(_state): State<Arc<AppState>>,
    Json(payload): Json<SaveThresholdRequest>,
) -> impl IntoResponse {
    let cwd = careerai_core::paths::resolve_root_env();
    let config_path = cwd.join("config").join("local.yaml");

    match crate::profile_handler::update_threshold_in_config(&config_path, payload.score_threshold)
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!(
                    "score_threshold set to {:.2} in {}",
                    payload.score_threshold,
                    config_path.display()
                ),
                "score_threshold": payload.score_threshold,
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

/// `POST /api/match/rematch-shortlisted` — re-score shortlisted listings
/// and demote those that fall below the current threshold to filtered_out.
pub async fn api_rematch_shortlisted(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let cwd = careerai_core::paths::resolve_root_env();
    let cfg = match careerai_core::config::CoreConfig::load(&cwd) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("load config: {e}") })),
            )
                .into_response();
        }
    };
    match careerai_pipeline::rematch_shortlisted(&cwd, &cfg).await {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!(
                    "Re-scored {} listings, demoted {}, promoted {} (threshold: {:.3})",
                    report.rescored, report.demoted, report.promoted, cfg.matching.score_threshold
                ),
                "rescored": report.rescored,
                "demoted": report.demoted,
                "promoted": report.promoted,
                "threshold": cfg.matching.score_threshold,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/analytics` — pattern analytics: reposts, funnel velocity,
/// advance rates, rejection latencies. All read-only SQL queries.
pub async fn api_analytics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let (reposts, funnel, advances, rejections) = tokio::join!(
        careerai_db::queries::detect_reposts(&state.pool),
        careerai_db::queries::funnel_velocity(&state.pool),
        careerai_db::queries::advance_rates(&state.pool),
        careerai_db::queries::rejection_latencies(&state.pool),
    );
    // R12: propagate DB errors instead of `.unwrap_or_default()`. A real
    // database failure should surface as a 500, not an empty array that
    // looks like "no data" to the dashboard UI.
    let reposts = match reposts {
        Ok(v) => v,
        Err(e) => return db_error_response(&e),
    };
    let funnel = match funnel {
        Ok(v) => v,
        Err(e) => return db_error_response(&e),
    };
    let advances = match advances {
        Ok(v) => v,
        Err(e) => return db_error_response(&e),
    };
    let rejections = match rejections {
        Ok(v) => v,
        Err(e) => return db_error_response(&e),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "reposts": reposts,
            "funnel_velocity": funnel,
            "advance_rates": advances,
            "rejection_latencies": rejections,
        })),
    )
        .into_response()
}

/// R12: render a DB error as a 500 JSON response.
fn db_error_response(e: &careerai_db::error::DbError) -> axum::response::Response {
    tracing::error!(error = %e, "analytics DB error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
        .into_response()
}

/// `GET /api/v1/content-library` — content library stats + domain breakdown.
pub async fn api_content_library(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match data::content_library_summary(&state.pool).await {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/llm-costs` — aggregate LLM cost + token usage from costs.jsonl.
pub async fn api_llm_costs(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let cwd = careerai_core::paths::resolve_root_env();
    let summary = crate::llm_costs::cost_summary(&cwd).await;
    (StatusCode::OK, Json(summary)).into_response()
}

/// `GET /api/v1/source-attribution` — per-source response rate ranking.
pub async fn api_source_attribution(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_db::queries::funnel_velocity(&state.pool).await {
        Ok(funnel) => {
            let views: Vec<view::SourceAttributionView> = funnel
                .into_iter()
                .map(|f| view::SourceAttributionView {
                    source: f.source,
                    discovered: f.discovered,
                    submitted: f.submitted,
                    responded: f.responded,
                    response_rate: if f.submitted > 0 {
                        f.responded as f64 / f.submitted as f64
                    } else {
                        0.0
                    },
                })
                .collect();
            (StatusCode::OK, Json(views)).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/follow-ups` — pending follow-up emails with company/title
/// context and full editable body (R18).
pub async fn api_follow_ups(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_db::queries::list_pending_follow_ups_with_listing(&state.pool).await {
        Ok(pending) => {
            let views: Vec<view::FollowUpView> = pending
                .into_iter()
                .map(|f| view::FollowUpView {
                    id: f.id,
                    application_id: f.application_id,
                    listing_id: f.listing_id,
                    company: f.company,
                    title: f.title,
                    status: f.status,
                    scheduled_at: f.scheduled_at,
                    body: f.body.unwrap_or_default(),
                    cadence_step: f.cadence_step,
                })
                .collect();
            (StatusCode::OK, Json(views)).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/variants` — A/B variant tracking with outcomes.
pub async fn api_variants(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_db::queries::list_variants_with_outcome(&state.pool).await {
        Ok(variants) => (StatusCode::OK, Json(variants)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/timing` — application timing stats (best days to submit).
pub async fn api_timing(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_db::queries::submission_timing_stats(&state.pool).await {
        Ok(stats) => (StatusCode::OK, Json(stats)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/salary-ranges` — extracted salary ranges.
pub async fn api_salary_ranges(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_db::queries::list_salary_ranges(&state.pool, 100).await {
        Ok(ranges) => (StatusCode::OK, Json(ranges)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// F02: `GET /api/v1/match-reasons/:listing_id` — return the structured
/// match reasons (matched/missing keywords, filter reason, legitimacy tier)
/// for a single listing. Returns 404 with `{ "match_reasons": null }` if
/// the listing has no recorded match reasons yet.
pub async fn api_match_reasons(
    State(state): State<Arc<AppState>>,
    Path(listing_id): Path<String>,
) -> impl IntoResponse {
    match careerai_db::queries::fetch_match_reasons(&state.pool, &listing_id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(row)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "match_reasons": null })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/market-pulse` — weekly market summary.
pub async fn api_market_pulse(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_db::queries::weekly_market_summary(&state.pool).await {
        Ok(pulse) => (StatusCode::OK, Json(pulse)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `GET /api/v1/interview-feedback` — logged interview feedback.
pub async fn api_interview_feedback(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_db::queries::list_feedback(&state.pool, 50).await {
        Ok(feedback) => (StatusCode::OK, Json(feedback)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/v1/interview-feedback` — save interview feedback.
#[derive(Debug, serde::Deserialize)]
pub struct SaveFeedbackRequest {
    pub application_id: String,
    pub listing_id: String,
    pub company: String,
    pub title: String,
    pub questions_text: Option<String>,
    pub rating: i64,
    pub went_well: Option<String>,
    pub could_improve: Option<String>,
}

pub async fn api_save_interview_feedback(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SaveFeedbackRequest>,
) -> impl IntoResponse {
    match careerai_db::queries::save_feedback(
        &state.pool,
        &payload.application_id,
        &payload.listing_id,
        &payload.company,
        &payload.title,
        payload.questions_text.as_deref(),
        payload.rating,
        payload.went_well.as_deref(),
        payload.could_improve.as_deref(),
    )
    .await
    {
        Ok(id) => (
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

/// `GET /api/v1/referrals` — referral opportunities matching listings
/// against the user's past employers from the profile.
pub async fn api_referrals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cwd = careerai_core::paths::resolve_root_env();
    let profile_path = careerai_core::paths::profile_path(&cwd);
    let Some(profile) = std::fs::read_to_string(&profile_path)
        .ok()
        .and_then(|raw| careerai_profile::schema::Profile::from_yaml(&raw).ok())
    else {
        return (StatusCode::OK, Json(Vec::<view::ReferralView>::new())).into_response();
    };

    let past = careerai_profile::extract_past_companies(&profile.experience);

    // Fetch listings in active pipeline states for referral matching.
    let states = [
        ListingState::Shortlisted,
        ListingState::Tailored,
        ListingState::Rendered,
        ListingState::Submitted,
    ];
    let mut all_listings: Vec<careerai_db::models::Listing> = Vec::new();
    for s in &states {
        if let Ok(rows) = careerai_db::queries::list_by_state(&state.pool, *s, 500).await {
            all_listings.extend(rows);
        }
    }

    let refs: Vec<careerai_profile::ListingRef<'_>> = all_listings
        .iter()
        .map(|l| careerai_profile::ListingRef {
            id: &l.id,
            company: &l.company,
            title: &l.title,
            url: &l.url,
        })
        .collect();

    let matches = careerai_profile::find_referral_opportunities(&refs, &past);
    let views: Vec<view::ReferralView> = matches
        .into_iter()
        .map(|m| view::ReferralView {
            listing_id: m.listing_id,
            company: m.company,
            title: m.title,
            url: m.url,
            connection_source: m.connection_source,
        })
        .collect();

    (StatusCode::OK, Json(views)).into_response()
}

/// `POST /api/v1/follow-ups/check` — manually trigger follow-up creation.
pub async fn api_check_follow_ups(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match careerai_scheduler::follow_ups::check_and_create_follow_ups(&state.pool).await {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "created": count,
                "message": format!("Created {} follow-up drafts", count),
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

/// `GET /api/v1/snapshot` — pipeline snapshot with KPI strip, funnel
/// columns, and state counts. Used by the dashboard's auto-refresh.
pub async fn api_snapshot(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match data::snapshot(&state.pool).await {
        Ok(snap) => (StatusCode::OK, Json(snap)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/v1/pipeline/discover` — trigger discovery via the CLI.
pub async fn api_pipeline_discover(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let req = crate::profile_handler::pipeline_types::CliRunRequest {
        command: "discover".into(),
        args: crate::profile_handler::pipeline_types::CliRunArgs::default(),
    };
    crate::profile_handler::run_cli_request(&req).await
}

/// `POST /api/v1/pipeline/match` — trigger matching via the CLI.
pub async fn api_pipeline_match(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let req = crate::profile_handler::pipeline_types::CliRunRequest {
        command: "match".into(),
        args: crate::profile_handler::pipeline_types::CliRunArgs::default(),
    };
    crate::profile_handler::run_cli_request(&req).await
}
