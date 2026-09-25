//! The `Source` trait + its shared DTO.
//!
//! One implementation per job board. `careerai-core` discovers listings by
//! iterating configured sources and calling `discover()` on each; it never
//! branches on concrete source types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("http {status}: {}", sanitize_error_body(.body))]
    HttpStatus { status: u16, body: String },
    #[error("parse: {0}")]
    Parse(String),
}

pub fn sanitize_error_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "empty response".to_string();
    }
    if trimmed.starts_with("<!DOCTYPE") || trimmed.starts_with("<html") || trimmed.contains("<body")
    {
        let clean = crate::util::html_to_text(trimmed);
        if clean.len() > 140 {
            format!("{}...", &clean[..137])
        } else if clean.is_empty() {
            "HTML error response".to_string()
        } else {
            clean
        }
    } else if trimmed.len() > 200 {
        format!("{}...", &trimmed[..197])
    } else {
        trimmed.to_string()
    }
}

/// A raw listing as returned by an adapter — pre-filter, pre-dedupe,
/// pre-persistence. `(source, external_id)` is the natural dedupe key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawListing {
    pub source: String,
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: String,
    /// Plain-text description. HTML is stripped at the adapter boundary.
    pub description: String,
    /// The raw JSON payload from the upstream board, for debugging.
    pub raw_json: Option<String>,
}

#[async_trait]
pub trait Source: Send + Sync {
    /// Stable identifier: `"greenhouse"`, `"lever"`, etc. Shared by all
    /// instances of the same board (a Greenhouse adapter for company A and
    /// company B both return `"greenhouse"`).
    fn name(&self) -> &'static str;

    /// Pull the latest listings. Implementations own their own pagination
    /// and rate-limit backoff.
    async fn discover(&self) -> Result<Vec<RawListing>, SourceError>;
}
