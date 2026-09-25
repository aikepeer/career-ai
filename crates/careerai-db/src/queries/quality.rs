//! Application quality score queries.
//!
//! Stores the sub-scores produced by `careerai_tailor::score_application_quality`
//! so the dashboard can surface application quality alongside the listing detail.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// One row in the `application_quality_scores` table.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct QualityScoreRow {
    pub id: i64,
    pub application_id: String,
    pub overall: f64,
    pub jd_relevance: f64,
    pub skill_coverage: f64,
    pub cover_letter_depth: f64,
    pub bullet_density: f64,
    pub recommendations: String,
    pub created_at: String,
}

/// Store a quality score for an application. Returns the inserted row id.
#[allow(clippy::too_many_arguments)]
pub async fn store_quality_score(
    pool: &SqlitePool,
    application_id: &str,
    overall: f32,
    jd_relevance: f32,
    skill_coverage: f32,
    cover_letter_depth: f32,
    bullet_density: f32,
    recommendations: &str,
) -> Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO application_quality_scores
            (application_id, overall, jd_relevance, skill_coverage,
             cover_letter_depth, bullet_density, recommendations)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(application_id)
    .bind(overall)
    .bind(jd_relevance)
    .bind(skill_coverage)
    .bind(cover_letter_depth)
    .bind(bullet_density)
    .bind(recommendations)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Fetch the most recent quality score for an application, if any.
pub async fn fetch_quality_score(
    pool: &SqlitePool,
    application_id: &str,
) -> Result<Option<QualityScoreRow>> {
    let row = sqlx::query_as(
        "SELECT id, application_id, overall, jd_relevance, skill_coverage,
                cover_letter_depth, bullet_density, recommendations, created_at
         FROM application_quality_scores
         WHERE application_id = ?
         ORDER BY created_at DESC
         LIMIT 1",
    )
    .bind(application_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
