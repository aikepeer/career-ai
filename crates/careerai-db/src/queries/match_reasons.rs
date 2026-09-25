//! F02: Explainable match cards — persist and fetch structured match
//! reasons so the dashboard can surface *why* a listing was shortlisted
//! or filtered out.
//!
//! One row per listing, upserted on every match run. The `matched_keywords`
//! and `missing_keywords` columns store JSON arrays; the caller serializes
//! them before passing.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// One row in the `match_reasons` table.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct MatchReasonRow {
    pub id: i64,
    pub listing_id: String,
    pub score: f64,
    pub matched_keywords: String,
    pub missing_keywords: String,
    pub filter_reason: Option<String>,
    pub legitimacy_tier: Option<String>,
    pub legitimacy_score: Option<f64>,
    pub eligibility_note: Option<String>,
    pub matched_at: String,
}

/// Upsert a match-reasons row for a listing. Returns the row id.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_match_reasons(
    pool: &SqlitePool,
    listing_id: &str,
    score: f32,
    matched_keywords: &str,
    missing_keywords: &str,
    filter_reason: Option<&str>,
    legitimacy_tier: Option<&str>,
    legitimacy_score: Option<f32>,
    eligibility_note: Option<&str>,
) -> Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO match_reasons
            (listing_id, score, matched_keywords, missing_keywords,
             filter_reason, legitimacy_tier, legitimacy_score, eligibility_note)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(listing_id) DO UPDATE SET
            score = excluded.score,
            matched_keywords = excluded.matched_keywords,
            missing_keywords = excluded.missing_keywords,
            filter_reason = excluded.filter_reason,
            legitimacy_tier = excluded.legitimacy_tier,
            legitimacy_score = excluded.legitimacy_score,
            eligibility_note = excluded.eligibility_note,
            matched_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         RETURNING id",
    )
    .bind(listing_id)
    .bind(f64::from(score))
    .bind(matched_keywords)
    .bind(missing_keywords)
    .bind(filter_reason)
    .bind(legitimacy_tier)
    .bind(legitimacy_score.map(f64::from))
    .bind(eligibility_note)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Fetch the most recent match reasons for a listing, if any.
pub async fn fetch_match_reasons(
    pool: &SqlitePool,
    listing_id: &str,
) -> Result<Option<MatchReasonRow>> {
    let row = sqlx::query_as(
        "SELECT id, listing_id, score, matched_keywords, missing_keywords,
                filter_reason, legitimacy_tier, legitimacy_score,
                eligibility_note, matched_at
         FROM match_reasons
         WHERE listing_id = ?
         ORDER BY matched_at DESC
         LIMIT 1",
    )
    .bind(listing_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
