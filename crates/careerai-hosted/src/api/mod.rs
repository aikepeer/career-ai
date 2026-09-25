//! Hosted API: Axum routes, middleware, and error handling (PR 6-7).
//!
//! API endpoints match the design doc's `/v1` route list. Session middleware
//! extracts tenant context and sets `SET LOCAL` per-request. Problem+json
//! error responses. Reauth middleware for sensitive endpoints.

pub mod action_handlers;
pub mod auth_handlers;
pub mod billing_handlers;
pub mod error;
pub mod middleware;
pub mod ops_handlers;
pub mod resource_handlers;
pub mod routes;
pub mod state;
pub mod workspace_handlers;

pub use error::{problem_json, ApiError};
pub use middleware::SessionAuth;
pub use state::AppState;
