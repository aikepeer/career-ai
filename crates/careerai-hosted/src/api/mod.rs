//! Hosted API: Axum routes, middleware, and error handling (PR 6-7).
//!
//! API endpoints match the design doc's `/v1` route list. Session middleware
//! extracts tenant context and sets `SET LOCAL` per-request. Problem+json
//! error responses. Reauth middleware for sensitive endpoints.

pub mod error;
pub mod routes;

pub use error::{ApiError, problem_json};
