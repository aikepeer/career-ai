//! Hand-rolled SQL for the `listings` table + their `state`
//! transitions. The transition path also writes an `events` row in the
//! same transaction, so the audit log is always consistent.

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use careerai_core::state::ListingState;

use crate::error::{DbError, Result};
use crate::models::{Listing, NewListing};

/// Insert a new listing or do nothing if `(source, external_id)` already
/// exists. Returns the row's id and whether it was newly inserted.
/// The listing insert and its initial `discovered` event are written
/// atomically so the audit log is always consistent.
pub async fn insert_or_ignore(pool: &SqlitePool, new: &NewListing) -> Result<(String, bool)> {
    let id = Uuid::now_v7().to_string();
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "INSERT OR IGNORE INTO listings
            (id, source, external_id, title, company, location, url, description, raw_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&new.source)
    .bind(&new.external_id)
    .bind(&new.title)
    .bind(&new.company)
    .bind(new.location.as_deref())
    .bind(&new.url)
    .bind(&new.description)
    .bind(new.raw_json.as_deref())
    .execute(&mut *tx)
    .await?;

    if res.rows_affected() == 1 {
        // New insertion: write the initial discovery event in the same tx so
        // both writes succeed or fail together.
        sqlx::query(
            "INSERT INTO events (listing_id, from_state, to_state, note) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind::<Option<&str>>(None)
        .bind(ListingState::Discovered.as_str())
        .bind::<Option<&str>>(None)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok((id, true));
    }

    // Already existed — nothing to write; drop the transaction and fetch
    // the stable id for the caller.
    drop(tx);
    let existing = find_by_external_id(pool, &new.source, &new.external_id).await?;
    Ok((existing.id, false))
}

pub async fn find_by_external_id(
    pool: &SqlitePool,
    source: &str,
    external_id: &str,
) -> Result<Listing> {
    let row: Option<Listing> = sqlx::query_as(
        "SELECT id, source, external_id, title, company, location, url, description,
                raw_json, state, score, created_at, updated_at
         FROM listings WHERE source = ? AND external_id = ?",
    )
    .bind(source)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| DbError::NotFound(format!("{source}/{external_id}")))
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Listing> {
    let row: Option<Listing> = sqlx::query_as(
        "SELECT id, source, external_id, title, company, location, url, description,
                raw_json, state, score, created_at, updated_at
         FROM listings WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| DbError::NotFound(id.to_string()))
}

pub async fn list_by_state(
    pool: &SqlitePool,
    state: ListingState,
    limit: i64,
) -> Result<Vec<Listing>> {
    let rows = sqlx::query_as(
        "SELECT id, source, external_id, title, company, location, url, description,
                raw_json, state, score, created_at, updated_at
         FROM listings WHERE state = ?
         ORDER BY COALESCE(score, 0) DESC, created_at DESC
         LIMIT ?",
    )
    .bind(state.as_str())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Atomically transition a listing's state and append an event.
pub async fn transition(
    pool: &SqlitePool,
    id: &str,
    to: ListingState,
    note: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let prev: Option<(String,)> = sqlx::query_as("SELECT state FROM listings WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let from = prev.ok_or_else(|| DbError::NotFound(id.to_string()))?.0;
    let now = Utc::now();
    sqlx::query("UPDATE listings SET state = ?, updated_at = ? WHERE id = ?")
        .bind(to.as_str())
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO events (listing_id, from_state, to_state, note) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(&from)
        .bind(to.as_str())
        .bind(note)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_score(pool: &SqlitePool, id: &str, score: f64) -> Result<()> {
    let res = sqlx::query("UPDATE listings SET score = ?, updated_at = ? WHERE id = ?")
        .bind(score)
        .bind(Utc::now())
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(id.to_string()));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::events::events_for;
    use crate::queries::test_support::fixture;

    #[tokio::test]
    async fn insert_then_dedupe() {
        let pool = pool_in_memory().await.unwrap();
        let (id1, inserted1) = insert_or_ignore(&pool, &fixture("greenhouse", "1"))
            .await
            .unwrap();
        let (id2, inserted2) = insert_or_ignore(&pool, &fixture("greenhouse", "1"))
            .await
            .unwrap();
        assert!(inserted1);
        assert!(!inserted2);
        assert_eq!(id1, id2);

        let listing = find_by_id(&pool, &id1).await.unwrap();
        assert_eq!(listing.state, "discovered");
        assert_eq!(listing.typed_state().unwrap(), ListingState::Discovered);
    }

    #[tokio::test]
    async fn transition_writes_event_and_updates_state() {
        let pool = pool_in_memory().await.unwrap();
        let (id, _) = insert_or_ignore(&pool, &fixture("lever", "abc"))
            .await
            .unwrap();
        transition(&pool, &id, ListingState::Shortlisted, Some("score=0.8"))
            .await
            .unwrap();
        let listing = find_by_id(&pool, &id).await.unwrap();
        assert_eq!(listing.typed_state().unwrap(), ListingState::Shortlisted);

        let events = events_for(&pool, &id).await.unwrap();
        // discovered (from insert) + shortlisted (from transition)
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].from_state.as_deref(), Some("discovered"));
        assert_eq!(events[1].to_state, "shortlisted");
        assert_eq!(events[1].note.as_deref(), Some("score=0.8"));
    }

    #[tokio::test]
    async fn list_by_state_orders_by_score_desc() {
        let pool = pool_in_memory().await.unwrap();
        let (a, _) = insert_or_ignore(&pool, &fixture("greenhouse", "a"))
            .await
            .unwrap();
        let (b, _) = insert_or_ignore(&pool, &fixture("greenhouse", "b"))
            .await
            .unwrap();
        let (c, _) = insert_or_ignore(&pool, &fixture("greenhouse", "c"))
            .await
            .unwrap();

        for id in [&a, &b, &c] {
            transition(&pool, id, ListingState::Shortlisted, None)
                .await
                .unwrap();
        }
        set_score(&pool, &a, 0.50).await.unwrap();
        set_score(&pool, &b, 0.85).await.unwrap();
        set_score(&pool, &c, 0.72).await.unwrap();

        let rows = list_by_state(&pool, ListingState::Shortlisted, 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, b);
        assert_eq!(rows[1].id, c);
        assert_eq!(rows[2].id, a);
    }

    #[tokio::test]
    async fn transition_unknown_id_is_not_found() {
        let pool = pool_in_memory().await.unwrap();
        let err = transition(&pool, "nope", ListingState::Failed, None)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }
}
