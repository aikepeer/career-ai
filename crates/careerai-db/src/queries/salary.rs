//! Salary range queries.
//!
//! Stores salary ranges extracted from job descriptions and aggregates
//! them by role so the dashboard can show market-rate benchmarks. The
//! `salary_stats_by_role` query groups by title to surface average
//! min/max across postings for the same role.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// One row in the `salary_ranges` table.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SalaryRange {
    pub id: i64,
    pub listing_id: String,
    pub company: String,
    pub title: String,
    pub min_salary: Option<i64>,
    pub max_salary: Option<i64>,
    pub currency: String,
    pub period: String,
    pub raw_text: Option<String>,
    pub created_at: String,
}

/// Aggregated salary stats grouped by role (title pattern).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SalaryStats {
    pub title_pattern: String,
    pub min_avg: f64,
    pub max_avg: f64,
    pub count: i64,
}

/// Store a salary range extracted from a JD. Returns the inserted row id.
#[allow(clippy::too_many_arguments)]
pub async fn store_salary_range(
    pool: &SqlitePool,
    listing_id: &str,
    company: &str,
    title: &str,
    min_salary: Option<i64>,
    max_salary: Option<i64>,
    currency: &str,
    period: &str,
    raw_text: Option<&str>,
) -> Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO salary_ranges
            (listing_id, company, title, min_salary, max_salary, currency, period, raw_text)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(listing_id)
    .bind(company)
    .bind(title)
    .bind(min_salary)
    .bind(max_salary)
    .bind(currency)
    .bind(period)
    .bind(raw_text)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// List recent salary ranges, newest first.
pub async fn list_salary_ranges(pool: &SqlitePool, limit: u32) -> Result<Vec<SalaryRange>> {
    let rows = sqlx::query_as(
        "SELECT id, listing_id, company, title, min_salary, max_salary,
                currency, period, raw_text, created_at
         FROM salary_ranges
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Aggregate salary stats grouped by title. `AVG` ignores NULL salary
/// columns, so rows with only a min or only a max still contribute to
/// the respective average.
pub async fn salary_stats_by_role(pool: &SqlitePool) -> Result<Vec<SalaryStats>> {
    let rows = sqlx::query_as(
        "SELECT title                       AS title_pattern,
                COALESCE(AVG(min_salary), 0.0) AS min_avg,
                COALESCE(AVG(max_salary), 0.0) AS max_avg,
                COUNT(*)                     AS count
         FROM salary_ranges
         GROUP BY title
         ORDER BY count DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
