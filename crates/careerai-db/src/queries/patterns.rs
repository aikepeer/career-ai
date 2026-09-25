//! Pattern-analysis queries (ported from career-ops `analyze-patterns.mjs`
//! and `detect-reposts.mjs`).
//!
//! These aggregate the `events` and `listings` tables to surface:
//! - repost / ghost-job detection (same company + title reappearing)
//! - per-source funnel velocity (discovered → shortlisted → submitted → responded)
//! - per-source advance rate (submitted → responded)
//! - rejection patterns (time-to-rejection distribution)
//!
//! All queries are read-only and parameterised.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::Result;

/// A likely repost: same company + title seen again as a new listing after
/// a gap. `gap_days` is `null` when there is only one occurrence (the query
/// still returns it so the caller can report the company+title combo).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Repost {
    pub company: String,
    pub title: String,
    pub source: String,
    pub occurrences: i64,
    /// Days between the first and most recent sighting. `None` when only
    /// one occurrence exists (shouldn't happen given the HAVING clause,
    /// but kept for forward-compat).
    pub first_seen: String,
    pub last_seen: String,
}

/// Detect reposted listings: the same company + title appearing more than
/// once across distinct `external_id`s. Grouped by (company, title, source).
///
/// A repost is suspicious when the gap between sightings is short (days, not
/// months) — career-ops flags these as potential ghost jobs.
pub async fn detect_reposts(pool: &SqlitePool) -> Result<Vec<Repost>> {
    let rows = sqlx::query_as(
        "SELECT l.company AS company,
                l.title AS title,
                l.source AS source,
                COUNT(DISTINCT l.external_id) AS occurrences,
                MIN(l.created_at) AS first_seen,
                MAX(l.created_at) AS last_seen
         FROM listings l
         GROUP BY l.company, l.title, l.source
         HAVING occurrences > 1
         ORDER BY occurrences DESC, last_seen DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// One row per source in the funnel velocity report.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FunnelVelocity {
    pub source: String,
    pub discovered: i64,
    pub shortlisted: i64,
    pub submitted: i64,
    pub responded: i64,
    pub rejected: i64,
}

/// Per-source funnel: how many listings reached each stage. Counts listings
/// whose `state` is at or beyond the stage (cumulative), not just currently
/// in that state — this mirrors career-ops `stats.mjs`.
pub async fn funnel_velocity(pool: &SqlitePool) -> Result<Vec<FunnelVelocity>> {
    // R11: operational states (skipped/failed) are NOT employer outcomes.
    // A skipped listing was never submitted; a failed listing had a
    // submission attempt that failed. Neither represents an employer
    // rejection. `submitted` counts any listing that ever had a
    // `submitted` event (cumulative), regardless of its current terminal
    // state. `rejected` comes from the `employer_outcomes` table, not
    // from operational state.
    let rows = sqlx::query_as(
        "SELECT l.source AS source,
                COUNT(*) AS discovered,
                COUNT(CASE WHEN l.state IN ('shortlisted','tailored','rendered','prepared','submitted','responded','skipped','failed') THEN 1 END) AS shortlisted,
                COUNT(CASE WHEN EXISTS (
                    SELECT 1 FROM events e
                    WHERE e.listing_id = l.id AND e.to_state = 'submitted'
                ) THEN 1 END) AS submitted,
                COUNT(CASE WHEN l.state = 'responded' THEN 1 END) AS responded,
                COUNT(CASE WHEN EXISTS (
                    SELECT 1 FROM employer_outcomes eo
                    WHERE eo.listing_id = l.id AND eo.outcome_type = 'rejection'
                ) THEN 1 END) AS rejected
         FROM listings l
         GROUP BY l.source
         ORDER BY discovered DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Advance rate per source: what fraction of submitted applications got a
/// response (positive or negative). career-ops calls this "advance rate".
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AdvanceRate {
    pub source: String,
    pub submitted: i64,
    pub responded: i64,
    pub rate: f64,
}

/// Compute per-source advance rate from the events table. A listing that
/// has at least one `submitted` event and at least one `responded` event
/// counts as advanced.
pub async fn advance_rates(pool: &SqlitePool) -> Result<Vec<AdvanceRate>> {
    let rows = sqlx::query_as(
        "WITH src AS (
            SELECT l.source AS source, l.id AS listing_id
            FROM listings l
            JOIN events e ON e.listing_id = l.id AND e.to_state = 'submitted'
            GROUP BY l.source, l.id
        )
        SELECT s.source AS source,
               COUNT(*) AS submitted,
               COUNT(CASE WHEN EXISTS (
                   SELECT 1 FROM events e2
                   WHERE e2.listing_id = s.listing_id AND e2.to_state = 'responded'
               ) THEN 1 END) AS responded,
               CASE WHEN COUNT(*) = 0 THEN 0.0
                    ELSE CAST(COUNT(CASE WHEN EXISTS (
                        SELECT 1 FROM events e3
                        WHERE e3.listing_id = s.listing_id AND e3.to_state = 'responded'
                    ) THEN 1 END) AS REAL) / COUNT(*)
               END AS rate
        FROM src s
        GROUP BY s.source
        ORDER BY rate DESC, submitted DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// One row per rejection: how long from submission to rejection.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RejectionLatency {
    pub company: String,
    pub title: String,
    pub source: String,
    pub days_to_reject: f64,
}

/// List all rejections with the number of days from submission to the
/// rejection. Sorted by latency descending (slowest rejections first —
/// career-ops flags these as "ghosted then rejected").
///
/// R11: rejections are an employer outcome, not an operational state.
/// The previous version treated `skipped`/`failed` events as rejection
/// events — a skip is not a rejection and a failure is an operational
/// failure, not an employer rejection. Now uses the `employer_outcomes`
/// table to find real rejection events.
pub async fn rejection_latencies(pool: &SqlitePool) -> Result<Vec<RejectionLatency>> {
    let rows = sqlx::query_as(
        "WITH sub AS (
            SELECT l.id AS listing_id, l.company AS company, l.title AS title,
                   l.source AS source, MAX(e.created_at) AS submitted_at
            FROM events e JOIN listings l ON l.id = e.listing_id
            WHERE e.to_state = 'submitted'
            GROUP BY l.id
        ),
        rej AS (
            SELECT eo.listing_id AS listing_id, MIN(eo.occurred_at) AS rejected_at
            FROM employer_outcomes eo
            WHERE eo.outcome_type = 'rejection'
            GROUP BY eo.listing_id
        )
        SELECT sub.company AS company, sub.title AS title, sub.source AS source,
               CAST(julianday(rej.rejected_at) - julianday(sub.submitted_at) AS REAL) AS days_to_reject
        FROM sub JOIN rej ON rej.listing_id = sub.listing_id
        ORDER BY days_to_reject DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::test_support::fixture;
    use crate::queries::{listings, listings::insert_or_ignore};
    use careerai_core::state::ListingState;

    async fn seed(pool: &sqlx::SqlitePool) {
        // Same company + title, different external_id → repost.
        let (l1, _) = insert_or_ignore(pool, &fixture("greenhouse", "job-1-a"))
            .await
            .unwrap();
        let (l2, _) = insert_or_ignore(pool, &fixture("greenhouse", "job-1-b"))
            .await
            .unwrap();
        let (_l3, _) = insert_or_ignore(pool, &fixture("lever", "job-2"))
            .await
            .unwrap();

        // l1: discovered → shortlisted → submitted → responded
        listings::transition(pool, &l1, ListingState::Shortlisted, None)
            .await
            .unwrap();
        listings::transition(pool, &l1, ListingState::Submitted, None)
            .await
            .unwrap();
        listings::transition(pool, &l1, ListingState::Responded, None)
            .await
            .unwrap();

        // l2: discovered → shortlisted → submitted → skipped (operational skip,
        // NOT an employer rejection under R11 semantics)
        listings::transition(pool, &l2, ListingState::Shortlisted, None)
            .await
            .unwrap();
        listings::transition(pool, &l2, ListingState::Submitted, None)
            .await
            .unwrap();
        listings::transition(pool, &l2, ListingState::Skipped, None)
            .await
            .unwrap();
        // R11: l2 was later manually recorded as rejected by the employer.
        // This is the employer_outcomes row that rejection_latencies queries.
        // application_id is NULL — no formal application row in this seed.
        sqlx::query(
            "INSERT INTO employer_outcomes
                (application_id, listing_id, outcome_type, occurred_at, is_manual, note)
             VALUES (NULL, ?, 'rejection', datetime('now'), 1, 'rejected after screen')",
        )
        .bind(&l2)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reposts_group_by_company_title_source() {
        let pool = pool_in_memory().await.unwrap();
        seed(&pool).await;

        let reposts = detect_reposts(&pool).await.unwrap();
        assert_eq!(reposts.len(), 1);
        assert_eq!(reposts[0].company, "Acme Robotics");
        assert_eq!(reposts[0].occurrences, 2);
        assert_eq!(reposts[0].source, "greenhouse");
    }

    #[tokio::test]
    async fn funnel_velocity_counts_cumulative() {
        let pool = pool_in_memory().await.unwrap();
        seed(&pool).await;

        let funnel = funnel_velocity(&pool).await.unwrap();
        let gh = funnel.iter().find(|f| f.source == "greenhouse").unwrap();
        assert_eq!(gh.discovered, 2);
        assert_eq!(gh.shortlisted, 2);
        assert_eq!(gh.submitted, 2); // both l1 + l2 had submitted events
        assert_eq!(gh.responded, 1); // only l1 is in 'responded' state
        assert_eq!(gh.rejected, 1); // l2 has an employer_outcomes rejection
    }

    #[tokio::test]
    async fn advance_rate_one_of_two() {
        let pool = pool_in_memory().await.unwrap();
        seed(&pool).await;

        let rates = advance_rates(&pool).await.unwrap();
        let gh = rates.iter().find(|r| r.source == "greenhouse").unwrap();
        assert_eq!(gh.submitted, 2);
        assert_eq!(gh.responded, 1);
        assert!((gh.rate - 0.5).abs() < 0.001);
    }

    #[tokio::test]
    async fn rejection_latency_reports_days() {
        let pool = pool_in_memory().await.unwrap();
        seed(&pool).await;

        let latencies = rejection_latencies(&pool).await.unwrap();
        assert_eq!(latencies.len(), 1); // only l2 was rejected
        assert_eq!(latencies[0].company, "Acme Robotics");
        // days_to_reject may be slightly negative due to sub-second timing
        // (events use ms precision, employer_outcomes uses second precision).
        assert!(latencies[0].days_to_reject >= -1.0);
    }
}
