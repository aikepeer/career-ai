//! Employer-outcome timeline queries (F04).
//!
//! Records externally applied, replied, interview, rejection, offer, and
//! withdrawal events separately from technical pipeline state. Each row
//! references a submission attempt when one exists so the outcome is tied
//! to a real submission, not an operational skip or failure.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::{DbError, Result};

/// One row in `employer_outcomes`.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct EmployerOutcome {
    pub id: i64,
    pub application_id: Option<String>,
    pub listing_id: String,
    pub attempt_id: Option<i64>,
    pub outcome_type: String,
    pub occurred_at: String,
    pub is_manual: bool,
    pub note: Option<String>,
    pub created_at: String,
}

/// Record a manual employer outcome. Returns the inserted row id.
/// `application_id` is optional — an outcome may be recorded for a listing
/// before a formal application row exists.
pub async fn record_outcome(
    pool: &SqlitePool,
    application_id: Option<&str>,
    listing_id: &str,
    attempt_id: Option<i64>,
    outcome_type: &str,
    occurred_at: &str,
    note: Option<&str>,
) -> Result<i64> {
    validate_outcome_type(outcome_type)?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO employer_outcomes
            (application_id, listing_id, attempt_id, outcome_type, occurred_at, is_manual, note)
         VALUES (?, ?, ?, ?, ?, 1, ?)
         RETURNING id",
    )
    .bind(application_id)
    .bind(listing_id)
    .bind(attempt_id)
    .bind(outcome_type)
    .bind(occurred_at)
    .bind(note)
    .fetch_one(pool)
    .await?;
    Ok(id)
}


/// List all outcomes for an application, newest first by occurrence.
pub async fn outcomes_for_application(
    pool: &SqlitePool,
    application_id: &str,
) -> Result<Vec<EmployerOutcome>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, listing_id, attempt_id, outcome_type, occurred_at,
                is_manual, note, created_at
         FROM employer_outcomes WHERE application_id = ?
         ORDER BY occurred_at DESC, id DESC",
    )
    .bind(application_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List all outcomes for a listing, newest first by occurrence.
pub async fn outcomes_for_listing(
    pool: &SqlitePool,
    listing_id: &str,
) -> Result<Vec<EmployerOutcome>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, listing_id, attempt_id, outcome_type, occurred_at,
                is_manual, note, created_at
         FROM employer_outcomes WHERE listing_id = ?
         ORDER BY occurred_at DESC, id DESC",
    )
    .bind(listing_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// List recent outcomes across all applications, newest first.
pub async fn list_recent_outcomes(pool: &SqlitePool, limit: u32) -> Result<Vec<EmployerOutcome>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, listing_id, attempt_id, outcome_type, occurred_at,
                is_manual, note, created_at
         FROM employer_outcomes
         ORDER BY occurred_at DESC, id DESC
         LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Delete an outcome by id. Used when a user corrects a misrecorded entry.
pub async fn delete_outcome(pool: &SqlitePool, id: i64) -> Result<()> {
    let res = sqlx::query("DELETE FROM employer_outcomes WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("employer_outcome {id}")));
    }
    Ok(())
}

/// Allowed outcome types.
pub const OUTCOME_TYPES: &[&str] = &[
    "applied",
    "replied",
    "interview",
    "rejection",
    "offer",
    "withdrawal",
];

fn validate_outcome_type(t: &str) -> Result<()> {
    if OUTCOME_TYPES.contains(&t) {
        Ok(())
    } else {
        Err(DbError::Conflict(format!(
            "invalid outcome_type '{t}'; expected one of: {}",
            OUTCOME_TYPES.join(", ")
        )))
    }
}
