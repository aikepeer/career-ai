//! Submission timing queries.
//!
//! Aggregates the `events` table to surface which days of the week and
//! which hour windows have the best response rates. The scheduler uses
//! `best_submission_windows` to bias submission timing toward windows
//! that historically convert.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// Response-rate stats grouped by day of week (0 = Sunday … 6 = Saturday).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DayOfWeekStats {
    pub day_of_week: i64,
    pub submitted: i64,
    pub responded: i64,
    pub response_rate: f64,
}

/// Response-rate stats grouped by hour-of-day bucket (0–23).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TimeWindowStats {
    pub hour_bucket: i64,
    pub submitted: i64,
    pub responded: i64,
    pub response_rate: f64,
}

/// Per-day-of-week submission stats with response rate. Mirrors the
/// `advance_rates` CTE pattern: a listing counts as responded when it
/// has at least one `responded` event after its `submitted` event.
pub async fn submission_timing_stats(pool: &SqlitePool) -> Result<Vec<DayOfWeekStats>> {
    let rows = sqlx::query_as(
        "WITH sub AS (
            SELECT e.listing_id AS listing_id,
                   CAST(strftime('%w', e.created_at) AS INTEGER) AS day_of_week
            FROM events e
            WHERE e.to_state = 'submitted'
        )
        SELECT sub.day_of_week AS day_of_week,
               COUNT(*) AS submitted,
               COUNT(CASE WHEN EXISTS (
                   SELECT 1 FROM events e2
                   WHERE e2.listing_id = sub.listing_id AND e2.to_state = 'responded'
               ) THEN 1 END) AS responded,
               CASE WHEN COUNT(*) = 0 THEN 0.0
                    ELSE CAST(COUNT(CASE WHEN EXISTS (
                        SELECT 1 FROM events e3
                        WHERE e3.listing_id = sub.listing_id AND e3.to_state = 'responded'
                    ) THEN 1 END) AS REAL) / COUNT(*)
               END AS response_rate
        FROM sub
        GROUP BY sub.day_of_week
        ORDER BY sub.day_of_week ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Per-hour submission stats with response rate, ordered by response
/// rate descending so the best-converting windows surface first.
pub async fn best_submission_windows(pool: &SqlitePool) -> Result<Vec<TimeWindowStats>> {
    let rows = sqlx::query_as(
        "WITH sub AS (
            SELECT e.listing_id AS listing_id,
                   CAST(strftime('%H', e.created_at) AS INTEGER) AS hour_bucket
            FROM events e
            WHERE e.to_state = 'submitted'
        )
        SELECT sub.hour_bucket AS hour_bucket,
               COUNT(*) AS submitted,
               COUNT(CASE WHEN EXISTS (
                   SELECT 1 FROM events e2
                   WHERE e2.listing_id = sub.listing_id AND e2.to_state = 'responded'
               ) THEN 1 END) AS responded,
               CASE WHEN COUNT(*) = 0 THEN 0.0
                    ELSE CAST(COUNT(CASE WHEN EXISTS (
                        SELECT 1 FROM events e3
                        WHERE e3.listing_id = sub.listing_id AND e3.to_state = 'responded'
                    ) THEN 1 END) AS REAL) / COUNT(*)
               END AS response_rate
        FROM sub
        GROUP BY sub.hour_bucket
        ORDER BY response_rate DESC, submitted DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
