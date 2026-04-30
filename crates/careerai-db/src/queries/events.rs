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
