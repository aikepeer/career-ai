use std::sync::Arc;

use axum::{routing::{get, post}, Router};

use crate::handlers;
use crate::AppState;

pub fn build(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handlers::index))
        .route("/healthz", get(handlers::healthz))
        .route("/api/v1/snapshot", get(handlers::api_snapshot))
        .route("/api/v1/events", get(handlers::api_events))
        .route(
            "/api/v1/config",
            get(handlers::api_config).post(handlers::api_save_config),
        )
        .route(
            "/api/v1/applications/:id",
            get(handlers::api_application_detail),
        )
        .route("/api/v1/explorer", get(handlers::api_explorer))
        .route("/api/v1/config/generate", post(handlers::api_config_generate))
            .route("/api/v1/config/prompt", post(handlers::api_config_prompt))
        .route("/api/v1/profile/import", post(handlers::api_profile_import))
        .route("/api/v1/profile/save", post(crate::profile_handler::api_profile_save))
        .route("/api/v1/config/keywords", post(crate::profile_handler::api_config_keywords))
        .route("/api/v1/listings/:id/shortlist", post(handlers::api_force_shortlist))
        .route("/api/v1/chat", post(crate::chat::api_chat_agent))
        .with_state(state)
}
