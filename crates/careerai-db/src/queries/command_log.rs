//! Write + read queries for the `command_log` audit table.
//!
//! Every command executed via the dashboard's `/api/v1/cli/run` endpoint
//! is recorded here — success, failure, and timeout — so the Events tab
//! can surface tailor/render/apply failures alongside pipeline state
//! transitions. The CLI's own `events` table is reserved for listing
//! state transitions; `command_log` captures operator-triggered actions
//! that don't always move a listing through a state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// One row in the `command_log` table.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CommandLogEntry {
    pub id: i64,
    pub command: String,
    pub listing_id: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Record a dashboard-spawned CLI command result.
pub async fn log_command(
    pool: &SqlitePool,
    command: &str,
    listing_id: Option<&str>,
    status: &str,
    exit_code: Option<i32>,
    message: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO command_log (command, listing_id, status, exit_code, message)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(command)
    .bind(listing_id)
    .bind(status)
    .bind(exit_code)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch recent command-log entries, newest first.
pub async fn list_recent_commands(pool: &SqlitePool, limit: u32) -> Result<Vec<CommandLogEntry>> {
    let rows = sqlx::query_as(
        "SELECT id, command, listing_id, status, exit_code, message, created_at
         FROM command_log ORDER BY created_at DESC LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;

    #[tokio::test]
    async fn log_and_list_commands() {
        let pool = pool_in_memory().await.unwrap();
        log_command(
            &pool,
            "tailor",
            Some("lst-1"),
            "success",
            Some(0),
            Some("ok"),
        )
        .await
        .unwrap();
        log_command(
            &pool,
            "tailor",
            Some("lst-2"),
            "failed",
            Some(1),
            Some("LLM backend error"),
        )
        .await
        .unwrap();
        log_command(&pool, "discover", None, "timeout", None, None)
            .await
            .unwrap();

        let entries = list_recent_commands(&pool, 10).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Newest first — discover (timeout) is last inserted.
        assert_eq!(entries[0].command, "discover");
        assert_eq!(entries[0].status, "timeout");
        assert!(entries[0].listing_id.is_none());
        // Failed tailor.
        let failed = entries.iter().find(|e| e.status == "failed").unwrap();
        assert_eq!(failed.command, "tailor");
        assert_eq!(failed.listing_id.as_deref(), Some("lst-2"));
        assert_eq!(failed.exit_code, Some(1));
        assert!(failed.message.as_deref().unwrap().contains("LLM backend"));
    }
}
