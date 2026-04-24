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
    #[error("application is in state '{state}'; expected 'rendered' or 'prepared'")]
    BadState { state: String },
    #[error("unknown source: {0}")]
    UnknownSource(String),
    #[error("source '{0}' is disabled in submit.per_source config")]
    SourceDisabled(String),
    #[error("dry-run mode; nothing sent")]
    DryRun,
}

pub type Result<T> = std::result::Result<T, SubmitError>;
