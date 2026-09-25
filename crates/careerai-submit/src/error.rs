//! Error type for the submit layer.
//!
//! Variants are deliberately actionable: a caller can tell at a glance
//! whether a failure is transient (network), structural (wrong state),
//! or policy (source disabled / dry-run).

use careerai_db::error::DbError;

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("db: {0}")]
    Db(#[from] DbError),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("http {status}: {body_tail}")]
    HttpStatus { status: u16, body_tail: String },
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("application is in state '{state}'; expected 'rendered', 'prepared', or 'drafted'")]
    BadState { state: String },
    #[error("unknown source: {0}")]
    UnknownSource(String),
    #[error("missing required submission data: {0}")]
    MissingData(String),
    #[error("source '{0}' is disabled in submit.per_source config")]
    SourceDisabled(String),
    /// The per-source rate policy refused a live submission attempt
    /// (day cap or quiet hours). Distinct from `SourceDisabled` (a
    /// config `enabled: false`) so audit logs can tell "operator turned
    /// this off" from "rate limiter held it back".
    #[error("rate-limited: {0}")]
    RateLimited(String),
    /// The submitter exists but its click/network flow has not been
    /// built yet. Distinct from `SourceDisabled` (config) so audit logs
    /// can tell "operator turned this off" from "engineer hasn't
    /// shipped this".
    #[error("not implemented: {0}")]
    NotImplemented(String),
}

pub type Result<T> = std::result::Result<T, SubmitError>;
