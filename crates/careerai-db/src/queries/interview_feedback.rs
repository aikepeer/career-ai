//! Interview feedback loop queries.
//!
//! Logs questions asked in interviews and a self-assessment of how the
//! interview went. The tailor module reads historical feedback to adjust
//! talking points and the cover letter emphasis for similar roles.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// One row in the `interview_feedback` table.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct InterviewFeedback {
    pub id: i64,
    pub application_id: String,
    pub listing_id: String,
    pub company: String,
    pub title: String,
    pub questions_text: Option<String>,
    pub rating: i64,
    pub went_well: Option<String>,
    pub could_improve: Option<String>,
    pub created_at: String,
}

/// Save interview feedback. Returns the inserted row id.
#[allow(clippy::too_many_arguments)]
pub async fn save_feedback(
    pool: &SqlitePool,
    application_id: &str,
    listing_id: &str,
    company: &str,
    title: &str,
    questions_text: Option<&str>,
    rating: i64,
    went_well: Option<&str>,
    could_improve: Option<&str>,
) -> Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO interview_feedback
            (application_id, listing_id, company, title, questions_text,
             rating, went_well, could_improve)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(application_id)
    .bind(listing_id)
    .bind(company)
    .bind(title)
    .bind(questions_text)
    .bind(rating)
    .bind(went_well)
    .bind(could_improve)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// List recent interview feedback, newest first.
pub async fn list_feedback(pool: &SqlitePool, limit: u32) -> Result<Vec<InterviewFeedback>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, listing_id, company, title, questions_text,
                rating, went_well, could_improve, created_at
         FROM interview_feedback
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List all interview feedback for a given listing.
pub async fn feedback_for_listing(
    pool: &SqlitePool,
    listing_id: &str,
) -> Result<Vec<InterviewFeedback>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, listing_id, company, title, questions_text,
                rating, went_well, could_improve, created_at
         FROM interview_feedback
         WHERE listing_id = ?
         ORDER BY created_at DESC",
    )
    .bind(listing_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
