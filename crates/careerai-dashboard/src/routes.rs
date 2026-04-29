use std::sync::Arc;

use axum::{routing::get, Router};

use crate::handlers;
use crate::AppState;

pub fn build(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(handlers::index))
        .route("/healthz", get(handlers::healthz))
        .with_state(state)
}
