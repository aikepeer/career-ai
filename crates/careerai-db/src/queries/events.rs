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
pub async fn list_recent_events(
    pool: &SqlitePool,
    limit: u32,
    offset: u32,
) -> Result<Vec<Event>> {
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

#[cfg(test)]
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

