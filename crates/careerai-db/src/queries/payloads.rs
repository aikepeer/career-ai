//! Hand-rolled SQL for `application_payloads` — the per-application
//! tailored resume JSON + cover-letter text that lets the render path
//! rehydrate without re-calling the LLM.

use sqlx::SqlitePool;

use crate::error::{DbError, Result};
use crate::models::ApplicationPayload;

/// Upsert the tailored payload for an application. Re-tailor overwrites.
pub async fn write_payload(
    pool: &SqlitePool,
    application_id: &str,
    resume_view_json: &str,
    cover_letter_text: &str,
    diff_json: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT OR REPLACE INTO application_payloads
            (application_id, resume_view_json, cover_letter_text, diff_json)
         VALUES (?, ?, ?, ?)",
    )
    .bind(application_id)
    .bind(resume_view_json)
    .bind(cover_letter_text)
    .bind(diff_json)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch the tailored payload for an application. Wave 4 addition so the CLI's
/// `render` path can rehydrate the `ResumeView` + cover letter without
/// re-calling the LLM.
pub async fn find_payload_by_application_id(
    pool: &SqlitePool,
    application_id: &str,
) -> Result<ApplicationPayload> {
    let row: Option<ApplicationPayload> = sqlx::query_as(
        "SELECT application_id, resume_view_json, cover_letter_text, diff_json, created_at
         FROM application_payloads WHERE application_id = ?",
    )
    .bind(application_id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| DbError::NotFound(application_id.to_string()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::applications::create_application;
    use crate::queries::listings::insert_or_ignore;
    use crate::queries::test_support::{fixture, new_app};

    #[tokio::test]
    async fn write_payload_upserts() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "app-3"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        write_payload(&pool, &app.id, r#"{"v":1}"#, "first body", r#"{"d":1}"#)
            .await
            .unwrap();
        write_payload(&pool, &app.id, r#"{"v":2}"#, "second body", r#"{"d":2}"#)
            .await
            .unwrap();

        let (resume, cover, diff): (String, String, String) = sqlx::query_as(
            "SELECT resume_view_json, cover_letter_text, diff_json
             FROM application_payloads WHERE application_id = ?",
        )
        .bind(&app.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(resume, r#"{"v":2}"#);
        assert_eq!(cover, "second body");
        assert_eq!(diff, r#"{"d":2}"#);
    }

    #[tokio::test]
    async fn find_payload_by_application_id_roundtrip() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "payload-1"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();
        write_payload(&pool, &app.id, r#"{"v":1}"#, "body", r#"{"d":1}"#)
            .await
            .unwrap();
        let got = find_payload_by_application_id(&pool, &app.id)
            .await
            .unwrap();
        assert_eq!(got.application_id, app.id);
        assert_eq!(got.resume_view_json, r#"{"v":1}"#);
        assert_eq!(got.cover_letter_text, "body");
        assert_eq!(got.diff_json, r#"{"d":1}"#);
    }

    #[tokio::test]
    async fn find_payload_by_application_id_missing_is_not_found() {
        let pool = pool_in_memory().await.unwrap();
        let err = find_payload_by_application_id(&pool, "nope")
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }
}
