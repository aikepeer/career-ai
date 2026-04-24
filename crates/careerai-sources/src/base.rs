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
    #[error("http {status}: {body}")]
    HttpStatus { status: u16, body: String },
    #[error("parse: {0}")]
    Parse(String),
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
