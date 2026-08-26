//! Read-side queries against the `events` audit log. The write-side
//! lives next to its trigger — `listings::transition` and
//! `applications::transition_application_and_listing` — because both
//! insert an `events` row inside a transaction with the corresponding
//! state update.

use sqlx::SqlitePool;

use crate::error::Result;
use crate::models::Event;

pub async fn events_for(pool: &SqlitePool, listing_id: &str) -> Result<Vec<Event>> {
    let rows = sqlx::query_as(
        "SELECT id, listing_id, from_state, to_state, note, created_at
         FROM events WHERE listing_id = ? ORDER BY created_at ASC",
    )
    .bind(listing_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Fetch recent audit events ordered by created_at DESC with limit and offset.
pub async fn list_recent_events(pool: &SqlitePool, limit: u32, offset: u32) -> Result<Vec<Event>> {
    let rows = sqlx::query_as(
        "SELECT id, listing_id, from_state, to_state, note, created_at
         FROM events ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(i64::from(limit))
    .bind(i64::from(offset))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Count submissions for a given source since a specific UTC timestamp.
pub async fn count_submissions_since(
    pool: &SqlitePool,
    source: &str,
    since: chrono::DateTime<chrono::Utc>,
) -> Result<u32> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events e
         JOIN listings l ON e.listing_id = l.id
         WHERE l.source = ? AND e.to_state = 'submitted' AND e.created_at >= ?",
    )
    .bind(source)
    .bind(since)
    .fetch_one(pool)
    .await?;
    Ok(u32::try_from(row.0).unwrap_or(u32::MAX))
}

/// Fetch the most recent submission event timestamp for a given source.
pub async fn latest_submission_time(
    pool: &SqlitePool,
    source: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    let row: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(
        "SELECT e.created_at FROM events e
         JOIN listings l ON e.listing_id = l.id
         WHERE l.source = ? AND e.to_state = 'submitted'
         ORDER BY e.created_at DESC LIMIT 1",
    )
    .bind(source)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::listings::{insert_or_ignore, transition};
    use crate::queries::test_support::fixture;
    use careerai_core::state::ListingState;

    #[tokio::test]
    async fn test_list_recent_events() {
        let pool = pool_in_memory().await.unwrap();
        let (id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "ev1"))
            .await
            .unwrap();

        transition(&pool, &id, ListingState::Shortlisted, Some("scored 0.85"))
            .await
            .unwrap();

        let evs = list_recent_events(&pool, 10, 0).await.unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0].listing_id, id);
        assert_eq!(evs[0].to_state, "shortlisted");
        assert_eq!(evs[1].to_state, "discovered");
    }
}
