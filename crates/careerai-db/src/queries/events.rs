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

/// One submitted application that has gone quiet: no `responded`
/// transition, and its latest submission is older than the follow-up
/// window. Powers `careerai followups` (ported from career-ops
/// `followup-cadence.mjs`).
#[derive(Debug, Clone)]
pub struct StaleSubmission {
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub source: String,
    pub url: String,
    pub submitted_at: chrono::DateTime<chrono::Utc>,
}

/// List submitted applications that have gone quiet for `min_days` or
/// more and never received a `responded` transition. Oldest first.
pub async fn list_stale_submissions(
    pool: &SqlitePool,
    min_days: i64,
) -> Result<Vec<StaleSubmission>> {
    let cutoff = format!("-{min_days} days");
    let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
        "SELECT l.id, l.title, l.company, l.source, l.url, MAX(e.created_at) AS submitted_at
         FROM events e
         JOIN listings l ON l.id = e.listing_id
         WHERE e.to_state = 'submitted'
           AND l.id NOT IN (
               SELECT DISTINCT listing_id FROM events WHERE to_state = 'responded'
           )
         GROUP BY l.id
         HAVING julianday(MAX(e.created_at)) <= julianday('now', ?1)
         ORDER BY submitted_at ASC",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(listing_id, title, company, source, url, ts)| {
            let submitted_at = chrono::DateTime::parse_from_rfc3339(&ts)
                .map_or_else(|_| chrono::Utc::now(), |t| t.with_timezone(&chrono::Utc));
            Ok(StaleSubmission {
                listing_id,
                title,
                company,
                source,
                url,
                submitted_at,
            })
        })
        .collect()
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

    #[tokio::test]
    async fn stale_submissions_exclude_responded_and_recent() {
        // Backdated events: insert directly (the transition helper stamps
        // `now`, which is what the real pipeline does). `events.id` is an
        // INTEGER AUTOINCREMENT column — bind numeric ids.
        async fn backdated_submit(pool: &SqlitePool, id: &str, days_ago: i64, ev_id: i64) {
            let ts = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
            sqlx::query(
                "INSERT INTO events (id, listing_id, from_state, to_state, note, created_at)
                 VALUES (?, ?, 'prepared', 'submitted', 'test', ?)",
            )
            .bind(ev_id)
            .bind(id)
            .bind(ts)
            .execute(pool)
            .await
            .unwrap();
        }

        let pool = pool_in_memory().await.unwrap();
        // A: submitted 15 days ago, never responded → stale.
        let (a, _) = insert_or_ignore(&pool, &fixture("greenhouse", "stale-a"))
            .await
            .unwrap();
        // B: submitted 3 days ago → too fresh.
        let (b, _) = insert_or_ignore(&pool, &fixture("greenhouse", "stale-b"))
            .await
            .unwrap();
        // C: submitted 15 days ago but responded 10 days ago → excluded.
        let (c, _) = insert_or_ignore(&pool, &fixture("greenhouse", "stale-c"))
            .await
            .unwrap();

        let mut ev_id = 1000i64;
        backdated_submit(&pool, &a, 15, ev_id).await;
        ev_id += 1;
        backdated_submit(&pool, &b, 3, ev_id).await;
        ev_id += 1;
        backdated_submit(&pool, &c, 15, ev_id).await;
        ev_id += 1;
        // C's response.
        sqlx::query(
            "INSERT INTO events (id, listing_id, from_state, to_state, note, created_at)
             VALUES (?, ?, 'submitted', 'responded', 'test', ?)",
        )
        .bind(ev_id)
        .bind(&c)
        .bind((chrono::Utc::now() - chrono::Duration::days(10)).to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();

        let stale = list_stale_submissions(&pool, 10).await.unwrap();
        assert_eq!(stale.len(), 1, "expected only A: {stale:?}");
        assert_eq!(stale[0].listing_id, a);
        assert_eq!(stale[0].company, "Acme Robotics");

        // A tighter window (3 days) admits A and B (15 and 3 days old)
        // but never C (responded).
        let tight = list_stale_submissions(&pool, 3).await.unwrap();
        let ids: Vec<&str> = tight.iter().map(|s| s.listing_id.as_str()).collect();
        assert!(ids.contains(&a.as_str()) && ids.contains(&b.as_str()));
        assert!(!ids.contains(&c.as_str()));
        // A looser window (20 days) admits nobody — everything is fresher
        // than the cutoff.
        assert!(list_stale_submissions(&pool, 20).await.unwrap().is_empty());
    }
}
