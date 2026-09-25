use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("invalid state value in row: {0}")]
    InvalidState(#[from] careerai_core::state::ListingStateParseError),

    #[error("listing not found: {0}")]
    NotFound(String),

    /// R14: the supplied IDs do not refer to the same application/listing
    /// pair, or a conditional update matched zero rows because the
    /// expected state no longer holds.
    #[error("conflict: {0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, DbError>;
