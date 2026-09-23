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

    for submission in &stale {
        // Only the newest application for a listing may receive a reminder.
        // The CTE prevents an older submitted attempt from creating a draft
        // after a newer application was prepared but not submitted.
        let app: Option<(String, i64)> = sqlx::query_as(
            "WITH latest_application AS (
                 SELECT id, state
                 FROM applications
                 WHERE listing_id = ?
                 ORDER BY created_at DESC, id DESC
                 LIMIT 1
             )
             SELECT a.id, sa.id
             FROM latest_application a
             JOIN submission_attempts sa ON sa.application_id = a.id
             WHERE a.state = 'submitted'
               AND sa.status = 'submitted'
               AND sa.submitted_at IS NOT NULL
               AND julianday(sa.submitted_at) <=
                   julianday('now', ?)
             ORDER BY sa.submitted_at DESC, sa.id DESC
             LIMIT 1",
        )
        .bind(&submission.listing_id)
        .bind(format!("-{DEFAULT_FOLLOW_UP_DAYS} days"))
        .fetch_optional(pool)
        .await?;

        let Some((application_id, submitted_attempt_id)) = app else {
            continue;
        };

        // A reminder is associated with one submitted attempt. Existing
        // rows in any terminal/pending status suppress another step until a
        // future cadence policy explicitly permits it.
        let already_scheduled: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM follow_ups
                 WHERE submitted_attempt_id = ? AND cadence_step = 1
             )",
        )
        .bind(submitted_attempt_id)
        .fetch_one(pool)
        .await?;
        if already_scheduled {
            continue;
        }

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

        match queries::create_follow_up(
            pool,
            &application_id,
            &submission.listing_id,
            &scheduled_at,
            Some(&body),
            1,
            submitted_attempt_id,
        )
        .await
        {
            Ok(_) => created += 1,
            Err(e) if is_unique_violation(&e) => {
                // Another scheduler won the attempt-level claim.
            }
            Err(e) => {
                tracing::error!(
                    target: "follow_ups",
                    application_id = %application_id,
                    submitted_attempt_id,
                    error = %e,
                    "create_follow_up failed",
                );
                return Err(e);
            }
        }
    }

    if created > 0 {
        info!(target: "follow_ups", created, "created {} follow-up drafts", created);
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
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_db::models::NewListing;
    use careerai_db::pool::pool_in_memory;
    use careerai_db::queries::insert_or_ignore;
    use chrono::Duration;

    #[tokio::test]
    async fn insert_failure_is_returned_to_the_caller() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(
            &pool,
            &NewListing {
                source: "greenhouse".into(),
                external_id: "follow-up-error".into(),
                title: "Rust Engineer".into(),
                company: "Acme".into(),
                location: Some("Remote".into()),
                url: "https://example.com/follow-up-error".into(),
                description: "Rust systems work".into(),
                raw_json: None,
            },
        )
        .await
        .unwrap();
        let application_id = "app-follow-up-error";
        sqlx::query(
            "INSERT INTO applications
                (id, listing_id, state, profile_hash, prompt_version, llm_model)
             VALUES (?, ?, 'submitted', 'hash', 'v1', 'test')",
        )
        .bind(application_id)
        .bind(&listing_id)
        .execute(&pool)
        .await
        .unwrap();
        let submitted_at = (Utc::now() - Duration::days(8)).to_rfc3339();
        sqlx::query(
            "INSERT INTO submission_attempts
                (application_id, attempt_no, status, pre_submit_state, submitted_at, resolved_at)
             VALUES (?, 1, 'submitted', 'prepared', ?, ?)",
        )
        .bind(application_id)
        .bind(&submitted_at)
        .bind(&submitted_at)
        .execute(&pool)
        .await
        .unwrap();
        let old = (Utc::now() - Duration::days(8)).to_rfc3339();
        sqlx::query(
            "INSERT INTO events (listing_id, from_state, to_state, created_at)
             VALUES (?, 'prepared', 'submitted', ?)",
        )
        .bind(&listing_id)
        .bind(&old)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_follow_up
             BEFORE INSERT ON follow_ups
             BEGIN SELECT RAISE(ABORT, 'follow-up insert blocked'); END",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = check_and_create_follow_ups(&pool).await;

        let error = result.expect_err("insert failure must not be reported as success");
        assert!(error.to_string().contains("follow-up insert blocked"));
    }
    #[tokio::test]
    async fn latest_unsubmitted_application_blocks_old_attempt_and_repeat_is_idempotent() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(
            &pool,
            &NewListing {
                source: "greenhouse".into(),
                external_id: "follow-up-latest".into(),
                title: "Platform Engineer".into(),
                company: "Acme".into(),
                location: Some("Remote".into()),
                url: "https://example.com/follow-up-latest".into(),
                description: "Platform systems work".into(),
                raw_json: None,
            },
        )
        .await
        .unwrap();
        let old_app = "app-follow-up-old";
        let new_app = "app-follow-up-new";
        let now = Utc::now();
        let old_created = (now - Duration::days(2)).to_rfc3339();
        let new_created = (now - Duration::days(1)).to_rfc3339();
        for (id, state, created_at) in [
            (old_app, "submitted", old_created.as_str()),
            (new_app, "tailored", new_created.as_str()),
        ] {
            sqlx::query(
                "INSERT INTO applications
                    (id, listing_id, state, profile_hash, prompt_version, llm_model,
                     created_at, updated_at)
                 VALUES (?, ?, ?, 'hash', 'v1', 'test', ?, ?)",
            )
            .bind(id)
            .bind(&listing_id)
            .bind(state)
            .bind(created_at)
            .bind(created_at)
            .execute(&pool)
            .await
            .unwrap();
        }
        let submitted_at = (now - Duration::days(8)).to_rfc3339();
        sqlx::query(
            "INSERT INTO submission_attempts
                (application_id, attempt_no, status, pre_submit_state, submitted_at, resolved_at)
             VALUES (?, 1, 'submitted', 'prepared', ?, ?)",
        )
        .bind(old_app)
        .bind(&submitted_at)
        .bind(&submitted_at)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO events (listing_id, from_state, to_state, created_at)
             VALUES (?, 'prepared', 'submitted', ?)",
        )
        .bind(&listing_id)
        .bind(&submitted_at)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(check_and_create_follow_ups(&pool).await.unwrap(), 0);

        sqlx::query("UPDATE applications SET state = 'submitted' WHERE id = ?")
            .bind(new_app)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO submission_attempts
                (application_id, attempt_no, status, pre_submit_state, submitted_at, resolved_at)
             VALUES (?, 1, 'submitted', 'prepared', ?, ?)",
        )
        .bind(new_app)
        .bind(&submitted_at)
        .bind(&submitted_at)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(check_and_create_follow_ups(&pool).await.unwrap(), 1);
        assert_eq!(check_and_create_follow_ups(&pool).await.unwrap(), 0);
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM follow_ups")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }
}
