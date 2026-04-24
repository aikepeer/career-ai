//! Pool construction and migration runner.

use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tracing::info;

use crate::error::Result;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Open or create a SQLite database at `path`, run migrations, return a pool.
pub async fn pool_from_path(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
        }
    }
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    MIGRATOR.run(&pool).await?;
    info!(path = %path.display(), "db: migrations applied");
    Ok(pool)
}

/// In-memory pool for tests. Each call gives a fresh DB.
pub async fn pool_in_memory() -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1) // ":memory:" is not shared across connections
        .connect_with(opts)
        .await?;
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}
