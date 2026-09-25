//! F06: Persisted saved views for the explorer.
//!
//! Each row stores a named filter preset so the user can restore a
//! known-good filter combination across sessions. The `filter_json`
//! column is a denormalized copy for forward-compat when new filter
//! dimensions are added.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// One row in the `saved_views` table.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SavedView {
    pub id: i64,
    pub name: String,
    pub query: Option<String>,
    pub source: Option<String>,
    pub state: Option<String>,
    pub remote_only: bool,
    pub filter_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Create or update a saved view by name. Returns the row id.
pub async fn upsert_saved_view(
    pool: &SqlitePool,
    name: &str,
    query: Option<&str>,
    source: Option<&str>,
    state: Option<&str>,
    remote_only: bool,
    filter_json: &str,
) -> Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO saved_views (name, query, source, state, remote_only, filter_json)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET
            query = excluded.query,
            source = excluded.source,
            state = excluded.state,
            remote_only = excluded.remote_only,
            filter_json = excluded.filter_json,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         RETURNING id",
    )
    .bind(name)
    .bind(query)
    .bind(source)
    .bind(state)
    .bind(remote_only)
    .bind(filter_json)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// List all saved views, oldest first.
pub async fn list_saved_views(pool: &SqlitePool) -> Result<Vec<SavedView>> {
    let rows = sqlx::query_as(
        "SELECT id, name, query, source, state, remote_only, filter_json,
                created_at, updated_at
         FROM saved_views
         ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Delete a saved view by name. Returns true if a row was deleted.
pub async fn delete_saved_view(pool: &SqlitePool, name: &str) -> Result<bool> {
    let rows = sqlx::query("DELETE FROM saved_views WHERE name = ?")
        .bind(name)
        .execute(pool)
        .await?;
    Ok(rows.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .unwrap();
        sqlx::query(include_str!("../../migrations/0013_saved_views.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn upsert_creates_then_updates_saved_view() {
        let pool = setup_pool().await;
        let id = upsert_saved_view(&pool, "Rust Remote", Some("rust"), None, None, true, "{}")
            .await
            .unwrap();
        assert!(id > 0);

        // Update same name — should return the same id (ON CONFLICT DO UPDATE).
        let id2 = upsert_saved_view(
            &pool,
            "Rust Remote",
            Some("rust async"),
            None,
            None,
            true,
            "{}",
        )
        .await
        .unwrap();
        assert_eq!(id, id2);

        let views = list_saved_views(&pool).await.unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].query.as_deref(), Some("rust async"));
    }

    #[tokio::test]
    async fn delete_removes_named_view() {
        let pool = setup_pool().await;
        upsert_saved_view(&pool, "To Delete", None, None, None, false, "{}")
            .await
            .unwrap();
        upsert_saved_view(&pool, "Keep Me", None, None, None, false, "{}")
            .await
            .unwrap();

        let deleted = delete_saved_view(&pool, "To Delete").await.unwrap();
        assert!(deleted);

        let views = list_saved_views(&pool).await.unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, "Keep Me");
    }
}
