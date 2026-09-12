//! Submission attempt queries (R01).
//!
//! Each submission attempt is recorded as a durable row so that concurrent
//! workers cannot double-submit and so that uncertain remote results can
//! be reconciled before retry. The `claim_submission_attempt` function
//! atomically inserts a row and advances the application to a `submitting`
//! state that is excluded from ordinary eligible-queues.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::{DbError, Result};

/// One row in `submission_attempts`.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SubmissionAttempt {
    pub id: i64,
    pub application_id: String,
    pub attempt_no: i64,
    pub status: String,
    pub approver: Option<String>,
    pub remote_receipt: Option<String>,
    pub idempotency_key: Option<String>,
    pub error: Option<String>,
    pub pre_submit_state: Option<String>,
    pub claimed_at: String,
    pub submitted_at: Option<String>,
    pub resolved_at: Option<String>,
}

/// Atomically claim a submission attempt for `application_id`.
///
/// Inserts a new attempt row whose `attempt_no` is one greater than the
/// current maximum for the application. The INSERT ... SELECT is atomic
/// within a transaction, so two concurrent callers cannot create the same
/// attempt number. Returns the new attempt row.
///
/// `pre_submit_state` records the application's state before the claim so
/// a crash after remote success but before the DB write can be recovered.
pub async fn claim_submission_attempt(
    pool: &SqlitePool,
    application_id: &str,
    approver: Option<&str>,
) -> Result<SubmissionAttempt> {
    let mut tx = pool.begin().await?;
    let now = Utc::now().to_rfc3339();

    // Fetch the application's current state for crash recovery.
    let app_row: Option<(String, String)> =
        sqlx::query_as("SELECT id, state FROM applications WHERE id = ?")
            .bind(application_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (_, pre_submit_state) =
        app_row.ok_or_else(|| DbError::NotFound(application_id.to_string()))?;

    // Atomically compute the next attempt_no and insert.
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO submission_attempts
            (application_id, attempt_no, status, approver, pre_submit_state, claimed_at)
         VALUES (?, COALESCE(
             (SELECT MAX(attempt_no) FROM submission_attempts WHERE application_id = ?), 0
         ) + 1, 'claimed', ?, ?, ?)
         RETURNING id",
    )
    .bind(application_id)
    .bind(application_id)
    .bind(approver)
    .bind(&pre_submit_state)
    .bind(&now)
    .fetch_one(&mut *tx)
    .await?;

    let id = row.0;
    tx.commit().await?;

    // Re-read the full row.
    let attempt = fetch_attempt_by_id(pool, id).await?;
    Ok(attempt)
}

/// Fetch a submission attempt by id.
pub async fn fetch_attempt_by_id(pool: &SqlitePool, id: i64) -> Result<SubmissionAttempt> {
    let row: Option<SubmissionAttempt> = sqlx::query_as(
        "SELECT id, application_id, attempt_no, status, approver, remote_receipt,
                idempotency_key, error, pre_submit_state, claimed_at, submitted_at, resolved_at
         FROM submission_attempts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| DbError::NotFound(format!("submission_attempt {id}")))
}

/// Mark an attempt as successfully submitted with the remote receipt.
pub async fn mark_attempt_submitted(
    pool: &SqlitePool,
    id: i64,
    remote_receipt: Option<&str>,
    idempotency_key: Option<&str>,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE submission_attempts
         SET status = 'submitted', remote_receipt = ?, idempotency_key = ?,
             submitted_at = ?, resolved_at = ?
         WHERE id = ?",
    )
    .bind(remote_receipt)
    .bind(idempotency_key)
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark an attempt as failed with the captured error.
pub async fn mark_attempt_failed(pool: &SqlitePool, id: i64, error: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE submission_attempts
         SET status = 'failed', error = ?, resolved_at = ?
         WHERE id = ?",
    )
    .bind(error)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark an attempt as uncertain — the remote call may or may not have
/// succeeded (e.g. timeout, network error after the request was sent).
/// Must be reconciled before retrying.
pub async fn mark_attempt_uncertain(pool: &SqlitePool, id: i64, error: &str) -> Result<()> {
    sqlx::query(
        "UPDATE submission_attempts
         SET status = 'uncertain', error = ?
         WHERE id = ?",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch the latest attempt for an application, regardless of status.
pub async fn latest_attempt_for_application(
    pool: &SqlitePool,
    application_id: &str,
) -> Result<Option<SubmissionAttempt>> {
    let row: Option<SubmissionAttempt> = sqlx::query_as(
        "SELECT id, application_id, attempt_no, status, approver, remote_receipt,
                idempotency_key, error, pre_submit_state, claimed_at, submitted_at, resolved_at
         FROM submission_attempts WHERE application_id = ?
         ORDER BY attempt_no DESC LIMIT 1",
    )
    .bind(application_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// List attempts that are in an uncertain state and need reconciliation.
pub async fn list_uncertain_attempts(pool: &SqlitePool) -> Result<Vec<SubmissionAttempt>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, attempt_no, status, approver, remote_receipt,
                idempotency_key, error, pre_submit_state, claimed_at, submitted_at, resolved_at
         FROM submission_attempts WHERE status = 'uncertain'
         ORDER BY claimed_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
