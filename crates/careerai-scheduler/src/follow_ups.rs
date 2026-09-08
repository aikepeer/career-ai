//! Follow-up email checker — runs periodically to find applications
//! that haven't received a response within the configured threshold
//! and creates draft follow-up emails for them.

use careerai_db::queries;
use chrono::Utc;
use sqlx::SqlitePool;
use tracing::info;

/// Default days before sending a follow-up.
const DEFAULT_FOLLOW_UP_DAYS: i64 = 7;

/// Check for stale applications (submitted but no response) and create
/// draft follow-up entries in the `follow_ups` table. This is a pure
/// data operation — no emails are sent. The dashboard surfaces pending
/// follow-ups for the user to review and send manually.
///
/// # Errors
/// Returns a `DbError` if the database queries fail.
pub async fn check_and_create_follow_ups(
    pool: &SqlitePool,
) -> Result<usize, careerai_db::error::DbError> {
    let stale = queries::list_stale_submissions(pool, DEFAULT_FOLLOW_UP_DAYS).await?;
    let mut created = 0;
    let mut errors = 0;

    for submission in &stale {
        // R10: find the application by its listing, ordered by created_at
        // DESC so we follow up on the most recent application. This matches
        // the order the dashboard's review queue uses.
        let app: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM applications WHERE listing_id = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&submission.listing_id)
        .fetch_optional(pool)
        .await?;

        let Some((application_id,)) = app else {
            continue;
        };

        // R10: determine the cadence step — if a pending follow-up already
        // exists for this application at step 1, this is a second reminder
        // (step 2). The uniqueness constraint uq_follow_ups_app_step from
        // migration 0011 prevents duplicate drafts for the same step.
        let cadence_step: i64 = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(cadence_step), 0) FROM follow_ups WHERE application_id = ?",
        )
        .bind(&application_id)
        .fetch_one(pool)
        .await?
        .saturating_add(1);

        let scheduled_at = Utc::now().to_rfc3339();
        let body = format!(
            "Hi,\n\nI wanted to follow up on my application for the {} position at {}. \
             I submitted my application on {} and am very interested in the opportunity.\n\n\
             I'd be happy to provide any additional information or complete a technical assessment.\n\n\
             Best regards,",
            submission.title,
            submission.company,
            submission.submitted_at.format("%Y-%m-%d"),
        );

        // R09: propagate create_follow_up errors instead of `let _ =`.
        // Only increment `created` if the insert actually succeeded. The
        // uniqueness constraint on (application_id, cadence_step) WHERE
        // status='pending' makes this idempotent — a concurrent scheduler
        // that already inserted the same step will get a constraint violation,
        // which we treat as "already done" rather than an error.
        match queries::create_follow_up(
            pool,
            &application_id,
            &submission.listing_id,
            &scheduled_at,
            Some(&body),
            cadence_step,
        )
        .await
        {
            Ok(_) => created += 1,
            Err(e) => {
                // Check if this is a uniqueness violation — the follow-up
                // already exists, which is not a real error.
                if is_unique_violation(&e) {
                    continue;
                }
                tracing::warn!(
                    target: "follow_ups",
                    application_id = %application_id,
                    error = %e,
                    "create_follow_up failed",
                );
                errors += 1;
            }
        }
    }

    if created > 0 {
        info!(target: "follow_ups", created, "created {} follow-up drafts", created);
    }
    if errors > 0 {
        tracing::warn!(
            target: "follow_ups",
            errors,
            "encountered {} errors creating follow-up drafts",
            errors,
        );
    }

    Ok(created)
}

/// R09: check if a database error is a SQLite UNIQUE constraint violation.
/// Used to distinguish "follow-up already exists" (expected, idempotent)
/// from a real database failure.
fn is_unique_violation(err: &careerai_db::error::DbError) -> bool {
    match err {
        careerai_db::error::DbError::Sqlx(sqlx::Error::Database(db)) => {
            db.message().contains("UNIQUE constraint failed")
        }
        _ => false,
    }
}
