//! API route definitions matching the design doc's `/v1` endpoint list.
//!
//! All routes are behind session middleware that extracts tenant context.
//! Sensitive endpoints (export, delete, approve) require reauth.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;

/// Build the hosted API router.
/// Routes match the design doc's `/v1` endpoint list.
pub fn router() -> Router {
    Router::new()
        // Auth
        .route("/v1/auth/start", post(start_login))
        .route("/v1/auth/callback", post(login_callback))
        .route("/v1/auth/logout", post(logout))
        .route("/v1/auth/recover", post(recover))
        // Workspaces
        .route("/v1/workspaces", get(list_workspaces))
        .route("/v1/workspaces/:id/switch", post(switch_workspace))
        // Profiles
        .route("/v1/profiles/imports", post(import_profile))
        // Preparation programs
        .route("/v1/preparation-programs", post(create_program))
        // Listings
        .route("/v1/listings", get(list_listings))
        // Applications
        .route("/v1/applications/:id/preview", post(preview_application))
        .route("/v1/applications/:id/approvals", post(approve_action))
        .route("/v1/applications/:id/actions", post(submit_action))
        // Exports
        .route("/v1/exports", post(create_export))
        // Account
        .route("/v1/account", axum::routing::delete(delete_account))
        // Billing
        .route("/v1/billing/checkout", post(billing_checkout))
        .route("/v1/billing/webhooks/:provider", post(billing_webhook))
        // Health
        .route("/v1/health", get(health))
}

async fn health() -> impl IntoResponse {
    StatusCode::OK
}

// Stub handlers — full implementation in PR 6+7
async fn start_login() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn login_callback() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn logout() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn recover() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn list_workspaces() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn switch_workspace() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn import_profile() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn create_program() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn list_listings() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn preview_application() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn approve_action() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn submit_action() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn create_export() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn delete_account() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn billing_checkout() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}
async fn billing_webhook() -> impl IntoResponse {
    StatusCode::NOT_IMPLEMENTED
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_endpoint_returns_200() {
        let app = router();
        let response = app
            .oneshot(Request::builder().uri("/v1/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_start_returns_501() {
        let app = router();
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/start")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let app = router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
