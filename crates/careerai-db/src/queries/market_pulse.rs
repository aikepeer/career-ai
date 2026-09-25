//! Market-pulse summary queries.
//!
//! Assembles a weekly dashboard snapshot from multiple read-only
//! sub-queries: new listings/companies in the last 7 days, top hiring
//! companies, most-frequent skills mentioned in JDs, average match score
//! of shortlisted listings, and submission/response counts. Mirrors the
//! multi-query assembly pattern used by `content_library::library_stats`.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// A company and how many listings it has posted.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CompanyHiring {
    pub company: String,
    pub count: i64,
}

/// A skill keyword and how many JDs mention it.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SkillFrequency {
    pub skill: String,
    pub count: i64,
}

/// Weekly market snapshot for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketPulse {
    pub new_listings_7d: i64,
    pub new_companies_7d: i64,
    pub top_hiring_companies: Vec<CompanyHiring>,
    pub top_skills_in_jds: Vec<SkillFrequency>,
    pub avg_score_shortlisted: f64,
    pub submitted_7d: i64,
    pub responded_7d: i64,
}

/// Assemble the weekly market summary from several sub-queries.
#[allow(clippy::cast_possible_truncation)]
pub async fn weekly_market_summary(pool: &SqlitePool) -> Result<MarketPulse> {
    let (new_listings_7d,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM listings
         WHERE datetime(created_at) >= datetime('now', '-7 days')",
    )
    .fetch_one(pool)
    .await?;

    let (new_companies_7d,): (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT company) FROM listings
         WHERE datetime(created_at) >= datetime('now', '-7 days')",
    )
    .fetch_one(pool)
    .await?;

    let top_hiring_companies: Vec<CompanyHiring> = sqlx::query_as(
        "SELECT company AS company, COUNT(*) AS count
         FROM listings
         GROUP BY company
         ORDER BY count DESC
         LIMIT 10",
    )
    .fetch_all(pool)
    .await?;

    // Count how many JD descriptions mention each known skill keyword.
    // Case-insensitive LIKE; skills with zero mentions are excluded.
    let top_skills_in_jds: Vec<SkillFrequency> = sqlx::query_as(
        "WITH skills(skill) AS (
            VALUES ('Python'), ('Rust'), ('TypeScript'), ('PyTorch'), ('TensorFlow'),
                   ('LLM'), ('machine learning'), ('deep learning'), ('NLP'),
                   ('CUDA'), ('embedded'), ('RTOS'), ('C++'), ('Linux'), ('Yocto'),
                   ('Kubernetes'), ('Docker'), ('AWS'), ('GCP'), ('Triton')
        )
        SELECT s.skill AS skill, COUNT(*) AS count
        FROM skills s
        JOIN listings l ON LOWER(l.description) LIKE '%' || LOWER(s.skill) || '%'
        GROUP BY s.skill
        ORDER BY count DESC
        LIMIT 10",
    )
    .fetch_all(pool)
    .await?;

    let (avg_score_shortlisted,): (f64,) = sqlx::query_as(
        "SELECT COALESCE(AVG(score), 0.0)
         FROM listings
         WHERE score IS NOT NULL
           AND state IN ('shortlisted','tailored','rendered','prepared',
                         'submitted','responded','skipped','failed')",
    )
    .fetch_one(pool)
    .await?;

    let (submitted_7d,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events
         WHERE to_state = 'submitted'
           AND datetime(created_at) >= datetime('now', '-7 days')",
    )
    .fetch_one(pool)
    .await?;

    let (responded_7d,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events
         WHERE to_state = 'responded'
           AND datetime(created_at) >= datetime('now', '-7 days')",
    )
    .fetch_one(pool)
    .await?;

    Ok(MarketPulse {
        new_listings_7d,
        new_companies_7d,
        top_hiring_companies,
        top_skills_in_jds,
        avg_score_shortlisted,
        submitted_7d,
        responded_7d,
    })
}
