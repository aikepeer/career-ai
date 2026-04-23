//! The `Source` trait. All discovery adapters implement this.
//!
//! Intentionally minimal at M0 — signature firms up when M2 lands.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("http error: {0}")]
    Http(String),
    #[error("parse error: {0}")]
    Parse(String),
}

/// A raw listing as returned by a source adapter, pre-normalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawListing {
    pub source: String,
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: String,
    pub description: String,
}

#[async_trait]
pub trait Source: Send + Sync {
    /// Stable identifier for this source (e.g. `"greenhouse"`).
    fn name(&self) -> &'static str;

    /// Pull the latest listings. Implementations own their own pagination
    /// and rate limiting.
    async fn discover(&self) -> Result<Vec<RawListing>, SourceError>;
}
