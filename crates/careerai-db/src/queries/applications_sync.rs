//! Cross-table queries that touch `applications` AND `listings` (or
//! `events`) together. Splits out from `applications.rs` to keep both
//! files under the 300-LOC cap and to make the cross-table boundary
//! visible in the file structure.

use chrono::Utc;
use sqlx::SqlitePool;

use careerai_core::state::ListingState;

use crate::error::{DbError, Result};
use crate::models::Application;

/// List applications by state, optionally filtered by their linked
/// listing's source. The single JOIN replaces a per-row `find_by_id`
/// fetch loop in the CLI's `apply_all` / `applied_show` paths
/// (previously O(N) round-trips when a `--source` filter was set).
pub async fn list_applications_by_state_and_source(
    pool: &SqlitePool,
    state: &str,
    source: Option<&str>,
    limit: i64,
) -> Result<Vec<Application>> {
    let rows = if let Some(src) = source {
        sqlx::query_as::<_, Application>(
            "SELECT a.id, a.listing_id, a.state, a.profile_hash, a.prompt_version, a.llm_model,
                    a.created_at, a.updated_at
             FROM applications a
             JOIN listings l ON l.id = a.listing_id
             WHERE a.state = ? AND l.source = ?
             ORDER BY a.created_at DESC
             LIMIT ?",
        )
        .bind(state)
        .bind(src)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Application>(
            "SELECT id, listing_id, state, profile_hash, prompt_version, llm_model,
                    created_at, updated_at
             FROM applications
             WHERE state = ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(state)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

/// Atomically transition both an application and its linked listing in
/// a single transaction. Used by `careerai-submit` to keep `applications.
/// state` and `listings.state` in lockstep across submit success / skip /
/// failure — previously two unrelated round-trips with a partial-failure
/// window.
///
/// Also appends an `events` row keyed off the listing for audit, so the
/// listing's `from_state → to_state` transition is preserved alongside
/// the application state change.
pub async fn transition_application_and_listing(
    pool: &SqlitePool,
    application_id: &str,
    listing_id: &str,
    application_state: &str,
    listing_state: ListingState,
    note: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();

    // Application state.
    let app_res = sqlx::query("UPDATE applications SET state = ?, updated_at = ? WHERE id = ?")
        .bind(application_state)
        .bind(now)
        .bind(application_id)
        .execute(&mut *tx)
        .await?;
    if app_res.rows_affected() == 0 {
        return Err(DbError::NotFound(application_id.to_string()));
    }

    // Listing state + events row.
    let prev: Option<(String,)> = sqlx::query_as("SELECT state FROM listings WHERE id = ?")
        .bind(listing_id)
        .fetch_optional(&mut *tx)
        .await?;
    let from = prev
        .ok_or_else(|| DbError::NotFound(listing_id.to_string()))?
        .0;
    sqlx::query("UPDATE listings SET state = ?, updated_at = ? WHERE id = ?")
        .bind(listing_state.as_str())
        .bind(now)
        .bind(listing_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO events (listing_id, from_state, to_state, note) VALUES (?, ?, ?, ?)")
        .bind(listing_id)
        .bind(&from)
        .bind(listing_state.as_str())
        .bind(note)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}
