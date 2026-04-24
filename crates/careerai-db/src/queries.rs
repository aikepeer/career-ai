//! Hand-rolled SQL queries. No `query!` macro so we avoid the
//! DATABASE_URL/.sqlx-cache compile-time dance for now. Switch to
//! compile-time-checked queries (M3+) once the schema stabilizes.

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use careerai_core::state::ListingState;

use crate::error::{DbError, Result};
use crate::models::{
    Application, Artifact, Event, Listing, NewApplication, NewArtifact, NewListing,
};

/// Insert a new listing or do nothing if `(source, external_id)` already
/// exists. Returns the row's id and whether it was newly inserted.
/// The listing insert and its initial `discovered` event are written
/// atomically so the audit log is always consistent.
pub async fn insert_or_ignore(pool: &SqlitePool, new: &NewListing) -> Result<(String, bool)> {
    let id = Uuid::now_v7().to_string();
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "INSERT OR IGNORE INTO listings
            (id, source, external_id, title, company, location, url, description, raw_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&new.source)
    .bind(&new.external_id)
    .bind(&new.title)
    .bind(&new.company)
    .bind(new.location.as_deref())
    .bind(&new.url)
    .bind(&new.description)
    .bind(new.raw_json.as_deref())
    .execute(&mut *tx)
    .await?;

    if res.rows_affected() == 1 {
        // New insertion: write the initial discovery event in the same tx so
        // both writes succeed or fail together.
        sqlx::query(
            "INSERT INTO events (listing_id, from_state, to_state, note) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind::<Option<&str>>(None)
        .bind(ListingState::Discovered.as_str())
        .bind::<Option<&str>>(None)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok((id, true));
    }

    // Already existed — nothing to write; drop the transaction and fetch
    // the stable id for the caller.
    drop(tx);
    let existing = find_by_external_id(pool, &new.source, &new.external_id).await?;
    Ok((existing.id, false))
}

pub async fn find_by_external_id(
    pool: &SqlitePool,
    source: &str,
    external_id: &str,
) -> Result<Listing> {
    let row: Option<Listing> = sqlx::query_as(
        "SELECT id, source, external_id, title, company, location, url, description,
                raw_json, state, score, created_at, updated_at
         FROM listings WHERE source = ? AND external_id = ?",
    )
    .bind(source)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| DbError::NotFound(format!("{source}/{external_id}")))
}

pub async fn find_by_id(pool: &SqlitePool, id: &str) -> Result<Listing> {
    let row: Option<Listing> = sqlx::query_as(
        "SELECT id, source, external_id, title, company, location, url, description,
                raw_json, state, score, created_at, updated_at
         FROM listings WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| DbError::NotFound(id.to_string()))
}

pub async fn list_by_state(
    pool: &SqlitePool,
    state: ListingState,
    limit: i64,
) -> Result<Vec<Listing>> {
    let rows = sqlx::query_as(
        "SELECT id, source, external_id, title, company, location, url, description,
                raw_json, state, score, created_at, updated_at
         FROM listings WHERE state = ?
         ORDER BY COALESCE(score, 0) DESC, created_at DESC
         LIMIT ?",
    )
    .bind(state.as_str())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Atomically transition a listing's state and append an event.
pub async fn transition(
    pool: &SqlitePool,
    id: &str,
    to: ListingState,
    note: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let prev: Option<(String,)> = sqlx::query_as("SELECT state FROM listings WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let from = prev.ok_or_else(|| DbError::NotFound(id.to_string()))?.0;
    let now = Utc::now();
    sqlx::query("UPDATE listings SET state = ?, updated_at = ? WHERE id = ?")
        .bind(to.as_str())
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO events (listing_id, from_state, to_state, note) VALUES (?, ?, ?, ?)")
        .bind(id)
        .bind(&from)
        .bind(to.as_str())
        .bind(note)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_score(pool: &SqlitePool, id: &str, score: f64) -> Result<()> {
    let res = sqlx::query("UPDATE listings SET score = ?, updated_at = ? WHERE id = ?")
        .bind(score)
        .bind(Utc::now())
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(id.to_string()));
    }
    Ok(())
}

pub async fn events_for(pool: &SqlitePool, listing_id: &str) -> Result<Vec<Event>> {
    let rows = sqlx::query_as(
        "SELECT id, listing_id, from_state, to_state, note, created_at
         FROM events WHERE listing_id = ? ORDER BY created_at ASC",
    )
    .bind(listing_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// --- applications / payloads / artifacts (M3) -------------------------------

/// Create a new application row. The listing insert pattern (tx + retry on
/// UUID collision) is mirrored here; UUIDv7 has sub-ms monotonicity so a
/// collision is vanishingly unlikely but we retry defensively.
pub async fn create_application(pool: &SqlitePool, new: &NewApplication) -> Result<Application> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut last_err: Option<DbError> = None;
    for _ in 0..MAX_ATTEMPTS {
        let id = Uuid::now_v7().to_string();
        let mut tx = pool.begin().await?;
        let res = sqlx::query(
            "INSERT INTO applications
                (id, listing_id, profile_hash, prompt_version, llm_model)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.listing_id)
        .bind(&new.profile_hash)
        .bind(&new.prompt_version)
        .bind(&new.llm_model)
        .execute(&mut *tx)
        .await;

        match res {
            Ok(_) => {
                let row: Application = sqlx::query_as(
                    "SELECT id, listing_id, state, profile_hash, prompt_version, llm_model,
                            created_at, updated_at
                     FROM applications WHERE id = ?",
                )
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
                tx.commit().await?;
                return Ok(row);
            }
            Err(sqlx::Error::Database(db_err))
                if db_err.kind() == sqlx::error::ErrorKind::UniqueViolation =>
            {
                // UUID collision — drop tx, retry with a fresh id.
                drop(tx);
                last_err = Some(DbError::Sqlx(sqlx::Error::Database(db_err)));
                continue;
            }
            Err(e) => return Err(DbError::Sqlx(e)),
        }
    }
    Err(last_err.unwrap_or_else(|| DbError::NotFound("uuid collision retries exhausted".into())))
}

pub async fn find_application_by_id(pool: &SqlitePool, id: &str) -> Result<Application> {
    let row: Option<Application> = sqlx::query_as(
        "SELECT id, listing_id, state, profile_hash, prompt_version, llm_model,
                created_at, updated_at
         FROM applications WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.ok_or_else(|| DbError::NotFound(id.to_string()))
}

/// Return the most recent application for `listing_id`, if any.
pub async fn find_latest_application_for_listing(
    pool: &SqlitePool,
    listing_id: &str,
) -> Result<Option<Application>> {
    let row: Option<Application> = sqlx::query_as(
        "SELECT id, listing_id, state, profile_hash, prompt_version, llm_model,
                created_at, updated_at
         FROM applications WHERE listing_id = ?
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(listing_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn set_application_state(pool: &SqlitePool, id: &str, state: &str) -> Result<()> {
    let res = sqlx::query("UPDATE applications SET state = ?, updated_at = ? WHERE id = ?")
        .bind(state)
        .bind(Utc::now())
        .bind(id)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(id.to_string()));
    }
    Ok(())
}

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

/// Attach an artifact row, overwriting any existing same-kind row for the
/// application (idempotent re-render). Uses `RETURNING *` to avoid a second
/// round-trip; SQLite >= 3.35 supports this.
pub async fn attach_artifact(
    pool: &SqlitePool,
    application_id: &str,
    a: &NewArtifact,
) -> Result<Artifact> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM artifacts WHERE application_id = ? AND kind = ?")
        .bind(application_id)
        .bind(&a.kind)
        .execute(&mut *tx)
        .await?;
    let row: Artifact = sqlx::query_as(
        "INSERT INTO artifacts (application_id, kind, path, bytes)
         VALUES (?, ?, ?, ?)
         RETURNING id, application_id, kind, path, bytes, created_at",
    )
    .bind(application_id)
    .bind(&a.kind)
    .bind(&a.path)
    .bind(a.bytes)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn list_artifacts(pool: &SqlitePool, application_id: &str) -> Result<Vec<Artifact>> {
    let rows = sqlx::query_as(
        "SELECT id, application_id, kind, path, bytes, created_at
         FROM artifacts WHERE application_id = ? ORDER BY kind ASC",
    )
    .bind(application_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;

    fn fixture(source: &str, ext: &str) -> NewListing {
        NewListing {
            source: source.into(),
            external_id: ext.into(),
            title: "Senior ML Engineer".into(),
            company: "Acme Robotics".into(),
            location: Some("Remote".into()),
            url: format!("https://example.com/{ext}"),
            description: "Build embedded LLM perception systems.".into(),
            raw_json: None,
        }
    }

    #[tokio::test]
    async fn insert_then_dedupe() {
        let pool = pool_in_memory().await.unwrap();
        let (id1, inserted1) = insert_or_ignore(&pool, &fixture("greenhouse", "1"))
            .await
            .unwrap();
        let (id2, inserted2) = insert_or_ignore(&pool, &fixture("greenhouse", "1"))
            .await
            .unwrap();
        assert!(inserted1);
        assert!(!inserted2);
        assert_eq!(id1, id2);

        let listing = find_by_id(&pool, &id1).await.unwrap();
        assert_eq!(listing.state, "discovered");
        assert_eq!(listing.typed_state().unwrap(), ListingState::Discovered);
    }

    #[tokio::test]
    async fn transition_writes_event_and_updates_state() {
        let pool = pool_in_memory().await.unwrap();
        let (id, _) = insert_or_ignore(&pool, &fixture("lever", "abc"))
            .await
            .unwrap();
        transition(&pool, &id, ListingState::Shortlisted, Some("score=0.8"))
            .await
            .unwrap();
        let listing = find_by_id(&pool, &id).await.unwrap();
        assert_eq!(listing.typed_state().unwrap(), ListingState::Shortlisted);

        let events = events_for(&pool, &id).await.unwrap();
        // discovered (from insert) + shortlisted (from transition)
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].from_state.as_deref(), Some("discovered"));
        assert_eq!(events[1].to_state, "shortlisted");
        assert_eq!(events[1].note.as_deref(), Some("score=0.8"));
    }

    #[tokio::test]
    async fn list_by_state_orders_by_score_desc() {
        let pool = pool_in_memory().await.unwrap();
        let (a, _) = insert_or_ignore(&pool, &fixture("greenhouse", "a"))
            .await
            .unwrap();
        let (b, _) = insert_or_ignore(&pool, &fixture("greenhouse", "b"))
            .await
            .unwrap();
        let (c, _) = insert_or_ignore(&pool, &fixture("greenhouse", "c"))
            .await
            .unwrap();

        for id in [&a, &b, &c] {
            transition(&pool, id, ListingState::Shortlisted, None)
                .await
                .unwrap();
        }
        set_score(&pool, &a, 0.50).await.unwrap();
        set_score(&pool, &b, 0.85).await.unwrap();
        set_score(&pool, &c, 0.72).await.unwrap();

        let rows = list_by_state(&pool, ListingState::Shortlisted, 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].id, b);
        assert_eq!(rows[1].id, c);
        assert_eq!(rows[2].id, a);
    }

    #[tokio::test]
    async fn transition_unknown_id_is_not_found() {
        let pool = pool_in_memory().await.unwrap();
        let err = transition(&pool, "nope", ListingState::Failed, None)
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    fn new_app(listing_id: &str) -> NewApplication {
        NewApplication {
            listing_id: listing_id.into(),
            profile_hash: "sha256:abc".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "claude-3-5-sonnet-20241022".into(),
        }
    }

    #[tokio::test]
    async fn create_application_then_find_by_id() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "app-1"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();
        assert_eq!(app.state, "tailored");
        assert_eq!(app.profile_hash, "sha256:abc");
        assert_eq!(app.listing_id, listing_id);

        let fetched = find_application_by_id(&pool, &app.id).await.unwrap();
        assert_eq!(fetched.id, app.id);
        assert_eq!(fetched.profile_hash, "sha256:abc");
        assert_eq!(fetched.prompt_version, "tailor.v1");
    }

    #[tokio::test]
    async fn set_application_state_updates_row_and_timestamp() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "app-2"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();
        let before = app.updated_at;

        // SQLite stores ms precision; sleep a moment so the timestamp advances.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        set_application_state(&pool, &app.id, "rendered")
            .await
            .unwrap();

        let after = find_application_by_id(&pool, &app.id).await.unwrap();
        assert_eq!(after.state, "rendered");
        assert!(after.updated_at >= before);
    }

    #[tokio::test]
    async fn set_application_state_unknown_id_is_not_found() {
        let pool = pool_in_memory().await.unwrap();
        let err = set_application_state(&pool, "nope", "rendered")
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

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
    async fn attach_artifact_overwrites_same_kind() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "app-4"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        attach_artifact(
            &pool,
            &app.id,
            &NewArtifact {
                kind: "resume_docx".into(),
                path: "a".into(),
                bytes: 10,
            },
        )
        .await
        .unwrap();
        attach_artifact(
            &pool,
            &app.id,
            &NewArtifact {
                kind: "resume_docx".into(),
                path: "b".into(),
                bytes: 20,
            },
        )
        .await
        .unwrap();

        let rows = list_artifacts(&pool, &app.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "b");
        assert_eq!(rows[0].bytes, 20);
    }

    #[tokio::test]
    async fn list_artifacts_orders_by_kind() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "app-5"))
            .await
            .unwrap();
        let app = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        for kind in ["resume_pdf", "cover_md", "resume_md"] {
            attach_artifact(
                &pool,
                &app.id,
                &NewArtifact {
                    kind: kind.into(),
                    path: format!("/tmp/{kind}"),
                    bytes: 1,
                },
            )
            .await
            .unwrap();
        }

        let rows = list_artifacts(&pool, &app.id).await.unwrap();
        let kinds: Vec<&str> = rows.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(kinds, vec!["cover_md", "resume_md", "resume_pdf"]);
    }

    #[tokio::test]
    async fn find_latest_application_for_listing_returns_most_recent() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "app-6"))
            .await
            .unwrap();
        let none = find_latest_application_for_listing(&pool, &listing_id)
            .await
            .unwrap();
        assert!(none.is_none());

        let _first = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let second = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        let latest = find_latest_application_for_listing(&pool, &listing_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.id, second.id);
    }

    #[tokio::test]
    async fn create_application_rejects_missing_listing() {
        let pool = pool_in_memory().await.unwrap();
        let err = create_application(&pool, &new_app("no-such-listing"))
            .await
            .unwrap_err();
        assert!(matches!(err, DbError::Sqlx(_)), "got {err:?}");
    }
}
