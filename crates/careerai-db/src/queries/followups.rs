//! Follow-up email scheduler queries.
//!
//! Tracks pending and sent follow-ups for applications that haven't
//! received a response within N days. The follow-ups daemon (M7) polls
//! `list_pending_follow_ups` on its cron cadence and marks rows sent as
//! it dispatches them.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// One row in the `follow_ups` table.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct FollowUp {
    pub id: i64,
    pub application_id: String,
    pub listing_id: String,
    pub scheduled_at: String,
    pub sent_at: Option<String>,
    pub status: String,
    pub body: Option<String>,
    /// R10: cadence step (1 = first reminder, 2 = second, etc.).
    #[sqlx(default)]
    pub cadence_step: i64,
    /// R10: links this follow-up to the submission attempt it concerns.
    #[sqlx(default)]
    pub submitted_attempt_id: Option<String>,
}

/// Schedule a new follow-up. Returns the inserted row id.
///
/// R10: `cadence_step` distinguishes first reminders (step 1) from
/// subsequent reminders (step 2+). The uniqueness constraint
/// `uq_follow_ups_app_step` on `(application_id, cadence_step) WHERE
/// status='pending'` prevents concurrent schedulers from creating
/// duplicate drafts for the same logical follow-up.
pub async fn create_follow_up(
    pool: &SqlitePool,
    application_id: &str,
    listing_id: &str,
    scheduled_at: &str,
    body: Option<&str>,
    cadence_step: i64,
) -> Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO follow_ups (application_id, listing_id, scheduled_at, body, cadence_step)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(application_id)
    .bind(listing_id)
    .bind(scheduled_at)
    .bind(body)
    .bind(cadence_step)
    .fetch_one(pool)
    .await?;
    Ok(id)
}
pub async fn list_pending_follow_ups(pool: &SqlitePool) -> Result<Vec<FollowUp>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, listing_id, scheduled_at, sent_at, status, body,
                cadence_step, submitted_attempt_id
         FROM follow_ups
         WHERE status = 'pending'
         ORDER BY scheduled_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
/// R18: a pending follow-up joined with its listing's company + title,
/// so the dashboard card can show which job the reminder is about
/// without a second round-trip. Includes the full body and cadence step.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct FollowUpWithListing {
    pub id: i64,
    pub application_id: String,
    pub listing_id: String,
    pub scheduled_at: String,
    pub sent_at: Option<String>,
    pub status: String,
    pub body: Option<String>,
    #[sqlx(default)]
    pub cadence_step: i64,
    #[sqlx(default)]
    pub submitted_attempt_id: Option<String>,
    pub company: String,
    pub title: String,
}

/// List all pending follow-ups joined with their listing's company + title.
pub async fn list_pending_follow_ups_with_listing(
    pool: &SqlitePool,
) -> Result<Vec<FollowUpWithListing>> {
    let rows = sqlx::query_as(
        "SELECT f.id, f.application_id, f.listing_id, f.scheduled_at, f.sent_at,
                f.status, f.body, f.cadence_step, f.submitted_attempt_id,
                l.company AS company, l.title AS title
         FROM follow_ups f
         JOIN listings l ON l.id = f.listing_id
         WHERE f.status = 'pending'
         ORDER BY f.scheduled_at ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Mark a follow-up as sent: set `status = 'sent'` and `sent_at` to now.
pub async fn mark_follow_up_sent(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query(
        "UPDATE follow_ups
         SET status = 'sent', sent_at = datetime('now')
         WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
/// F05: snooze a follow-up by rescheduling it to a later time. The
/// follow-up stays `pending` so it reappears in the inbox at the new time.
pub async fn snooze_follow_up(pool: &SqlitePool, id: i64, new_scheduled_at: &str) -> Result<()> {
    let res = sqlx::query(
        "UPDATE follow_ups SET scheduled_at = ?
         WHERE id = ? AND status = 'pending'",
    )
    .bind(new_scheduled_at)
    .bind(id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(crate::error::DbError::NotFound(format!("follow_up {id}")));
    }
    Ok(())
}

/// F05: dismiss a follow-up by marking it `dismissed`. The row is
/// preserved for audit but no longer appears in the pending inbox.
/// Reviewing or handling a reminder does NOT automatically send email.
pub async fn dismiss_follow_up(pool: &SqlitePool, id: i64) -> Result<()> {
    let res = sqlx::query(
        "UPDATE follow_ups SET status = 'dismissed', sent_at = datetime('now')
         WHERE id = ? AND status = 'pending'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(crate::error::DbError::NotFound(format!("follow_up {id}")));
    }
    Ok(())
}

/// F05: mark a follow-up as handled — the user has dealt with it
/// (e.g. sent the email manually outside the tool). Sets status to
/// `handled` and records the time. Does NOT send any email.
pub async fn mark_follow_up_handled(pool: &SqlitePool, id: i64) -> Result<()> {
    let res = sqlx::query(
        "UPDATE follow_ups SET status = 'handled', sent_at = datetime('now')
         WHERE id = ? AND status = 'pending'",
    )
    .bind(id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(crate::error::DbError::NotFound(format!("follow_up {id}")));
    }
    Ok(())
}

/// F05: update the body of a pending follow-up draft so the user can
/// edit the email text before manually sending it.
pub async fn update_follow_up_body(pool: &SqlitePool, id: i64, body: &str) -> Result<()> {
    let res = sqlx::query("UPDATE follow_ups SET body = ? WHERE id = ? AND status = 'pending'")
        .bind(body)
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(crate::error::DbError::NotFound(format!("follow_up {id}")));
    }
    Ok(())
}
