use std::sync::Arc;

use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Router,
};

use crate::handlers;
use crate::AppState;

/// Serve the PWA manifest JSON.
async fn serve_manifest() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("Content-Type", "application/manifest+json")],
        include_str!("../static/manifest.json"),
    )
}

/// Serve the PWA service worker script.
async fn serve_sw() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("Content-Type", "application/javascript")],
        include_str!("../static/sw.js"),
    )
}

/// Route registration is a flat declarative list; splitting adds indirection
/// without behavioral benefit. Follows the pattern in `careerai-scheduler`.
#[allow(clippy::too_many_lines)]
pub fn build(state: Arc<AppState>) -> Router {
    let auth_state = state.clone();
    Router::new()
        .route("/", get(handlers::index))
        .route("/healthz", get(handlers::healthz))
        .route("/sw-manifest", get(serve_manifest))
        .route("/sw.js", get(serve_sw))
        .route(
            "/static/workspace.js",
            get(|| async {
                (
                    [
                        ("Content-Type", "application/javascript"),
                        ("Cache-Control", "no-cache"),
                    ],
                    include_str!("../static/workspace.js"),
                )
            }),
        )
        .route(
            "/static/workspace.css",
            get(|| async {
                (
                    [("Content-Type", "text/css"), ("Cache-Control", "no-cache")],
                    include_str!("../static/workspace.css"),
                )
            }),
        )
        .route("/api/v1/activity", get(crate::activity::api_activity))
        .route("/api/v1/events", get(handlers::api_events))
        .route(
            "/api/v1/config",
            get(handlers::api_config).post(handlers::api_save_config),
        )
        .route(
            "/api/v1/applications/:id",
            get(handlers::api_application_detail),
        )
        .route(
            "/api/v1/artifacts/download",
            get(handlers::api_download_artifact),
        )
        .route("/api/v1/explorer", get(handlers::api_explorer))
        .route(
            "/api/v1/config/generate",
            post(handlers::api_config_generate),
        )
        .route("/api/v1/config/apply", post(handlers::api_config_apply))
        .route("/api/v1/config/prompt", post(handlers::api_config_prompt))
        .route("/api/v1/profile/import", post(handlers::api_profile_import))
        .route(
            "/api/v1/profile/import/confirm",
            post(handlers::api_profile_import_confirm),
        )
        .route(
            "/api/v1/profile/save",
            post(crate::profile_handler::api_profile_save),
        )
        .route(
            "/api/v1/config/keywords",
            post(crate::profile_handler::api_config_keywords),
        )
        .route("/api/v1/cli/run", post(crate::profile_handler::api_cli_run))
        .route(
            "/api/v1/listings/:id/shortlist",
            post(handlers::api_force_shortlist),
        )
        .route("/api/v1/chat", post(crate::chat::api_chat_agent))
        .route("/api/v1/analytics", get(handlers::api_analytics))
        .route(
            "/api/v1/content-library",
            get(handlers::api_content_library),
        )
        .route("/api/v1/llm-costs", get(handlers::api_llm_costs))
        .route(
            "/api/v1/source-attribution",
            get(handlers::api_source_attribution),
        )
        .route("/api/v1/follow-ups", get(handlers::api_follow_ups))
        .route("/api/v1/referrals", get(handlers::api_referrals))
        .route(
            "/api/v1/follow-ups/check",
            post(handlers::api_check_follow_ups),
        )
        .route("/api/v1/variants", get(handlers::api_variants))
        .route("/api/v1/outcomes", post(handlers::api_record_outcome))
        .route("/api/v1/outcomes", get(handlers::api_list_recent_outcomes))
        .route("/api/v1/outcomes/types", get(handlers::api_outcome_types))
        .route(
            "/api/v1/outcomes/by-listing/:listing_id",
            get(handlers::api_list_outcomes),
        )
        .route(
            "/api/v1/outcomes/by-id/:id",
            delete(handlers::api_delete_outcome),
        )
        .route(
            "/api/v1/follow-ups/:id/snooze",
            post(handlers::api_snooze_follow_up),
        )
        .route(
            "/api/v1/follow-ups/:id/dismiss",
            post(handlers::api_dismiss_follow_up),
        )
        .route(
            "/api/v1/follow-ups/:id/handle",
            post(handlers::api_handle_follow_up),
        )
        .route(
            "/api/v1/follow-ups/:id/body",
            post(handlers::api_update_follow_up_body),
        )
        .route("/api/v1/timing", get(handlers::api_timing))
        .route("/api/v1/salary-ranges", get(handlers::api_salary_ranges))
        .route(
            "/api/v1/match-reasons/:listing_id",
            get(handlers::api_match_reasons),
        )
        .route("/api/v1/insights", get(handlers::api_insights))
        .route("/api/v1/market-pulse", get(handlers::api_market_pulse))
        .route(
            "/api/v1/interview-feedback",
            get(handlers::api_interview_feedback).post(handlers::api_save_interview_feedback),
        )
        .route("/api/config/threshold", post(handlers::api_save_threshold))
        .route(
            "/api/match/rematch-shortlisted",
            post(handlers::api_rematch_shortlisted),
        )
        .route("/api/v1/snapshot", get(handlers::api_snapshot))
        .route(
            "/api/v1/pipeline/discover",
            post(handlers::api_pipeline_discover),
        )
        .route("/api/v1/pipeline/match", post(handlers::api_pipeline_match))
        .route("/api/v1/onboarding", get(handlers::api_onboarding))
        .route("/api/v1/review-queue", get(handlers::api_review_queue))
        .route("/api/v1/review-queue/:id", get(handlers::api_review_detail))
        .route(
            "/api/v1/review-queue/:id/approve",
            post(handlers::api_review_approve),
        )
        .route(
            "/api/v1/review-queue/:id/skip",
            post(handlers::api_review_skip),
        )
        .route(
            "/api/v1/review-queue/:id/retry",
            post(handlers::api_review_retry),
        )
        .route(
            "/api/v1/saved-views",
            get(handlers::api_list_saved_views).post(handlers::api_save_saved_view),
        )
        .route(
            "/api/v1/saved-views/:name",
            delete(handlers::api_delete_saved_view),
        )
        .layer(axum::middleware::from_fn(
            crate::security::enforce_same_origin,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            crate::security::enforce_auth_token,
        ))
        .with_state(state)
}
