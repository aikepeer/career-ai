//! API router assembly and integration tests.
//!
//! All routes use `Arc<AppState>` as shared state. Session middleware
//! extracts the Bearer token and verifies the session.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

use crate::api::state::AppState;

/// Build the hosted API router with the given shared state.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Auth
        .route("/v1/auth/start", post(crate::api::auth_handlers::start_login))
        .route("/v1/auth/callback", post(crate::api::auth_handlers::login_callback))
        .route("/v1/auth/logout", post(crate::api::auth_handlers::logout))
        .route("/v1/auth/recover", post(crate::api::auth_handlers::recover))
        // Workspaces
        .route("/v1/workspaces", get(crate::api::workspace_handlers::list_workspaces))
        .route("/v1/workspaces/:id/switch", post(crate::api::workspace_handlers::switch_workspace))
        // Profiles
        .route("/v1/profiles/imports", post(crate::api::resource_handlers::import_profile))
        // Preparation programs
        .route("/v1/preparation-programs", post(crate::api::resource_handlers::create_program))
        // Listings
        .route("/v1/listings", get(crate::api::resource_handlers::list_listings))
        // Applications
        .route("/v1/applications/:id/preview", post(crate::api::resource_handlers::preview_application))
        .route("/v1/applications/:id/approvals", post(crate::api::action_handlers::approve_action))
        .route("/v1/applications/:id/actions", post(crate::api::action_handlers::submit_action))
        // Exports
        .route("/v1/exports", post(crate::api::action_handlers::create_export))
        // Account
        .route("/v1/account", axum::routing::delete(crate::api::action_handlers::delete_account))
        // Billing
        .route("/v1/billing/checkout", post(crate::api::billing_handlers::billing_checkout))
        .route("/v1/billing/webhooks/:provider", post(crate::api::billing_handlers::billing_webhook))
        // Health
        .route("/v1/health", get(health))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    StatusCode::OK
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "routes_tests.rs"]
mod tests;
