//! Hand-rolled SQL for the `applications` table — single-table row
//! CRUD and state changes. Cross-table queries (JOIN with listings,
//! lockstep transitions) live in `applications_sync`. LinkedIn-
//! specific draft-review queries live in `linkedin`.

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::models::{Application, NewApplication};

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
                if db_err.kind() == sqlx::error::ErrorKind::UniqueViolation
                    && is_applications_pk_collision(&*db_err) =>
            {
                // Primary-key UUID collision — drop tx, retry with a
                // fresh id. Every other UNIQUE violation (e.g. a future
                // business-rule uniqueness on listing_id + hash) would
                // never succeed on retry, so we surface it immediately
                // instead of eating 3 attempts then returning NotFound.
                drop(tx);
                last_err = Some(DbError::Sqlx(sqlx::Error::Database(db_err)));
            }
            Err(e) => return Err(DbError::Sqlx(e)),
        }
    }
    Err(last_err.unwrap_or_else(|| DbError::NotFound("uuid collision retries exhausted".into())))
}

/// SQLite reports every UNIQUE violation with the message
/// `UNIQUE constraint failed: <table>.<column>[, ...]`. We only want to
/// retry when the violation is on the applications primary key — any
/// other unique constraint wouldn't be cleared by minting a fresh UUID.
fn is_applications_pk_collision(db_err: &dyn sqlx::error::DatabaseError) -> bool {
    let msg = db_err.message();
    msg.contains("applications.id")
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

/// List applications currently in the given state, newest first.
///
/// Mirrors `list_by_state` for listings but scopes to the `applications`
/// table; consumed by `careerai apply --all` (state = "rendered"/"prepared")
/// and `careerai applied` (state = "submitted"). The caller is responsible
/// for any cross-table filtering (e.g. by listing source) after the fact.
pub async fn list_applications_by_state(
    pool: &SqlitePool,
    state: &str,
    limit: i64,
) -> Result<Vec<Application>> {
    let rows = sqlx::query_as(
        "SELECT id, listing_id, state, profile_hash, prompt_version, llm_model,
                created_at, updated_at
         FROM applications WHERE state = ?
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(state)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
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

/// Whether any application exists for `company` (any state). Powers the
/// `apply_once_at_company` match gate — one application per company,
/// ported from AIHawk's work-preferences.
pub async fn has_application_for_company(pool: &SqlitePool, company: &str) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT a.id
         FROM applications a
         JOIN listings l ON l.id = a.listing_id
         WHERE l.company = ? COLLATE NOCASE
         LIMIT 1",
    )
    .bind(company)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::pool::pool_in_memory;
    use crate::queries::listings::insert_or_ignore;
    use crate::queries::test_support::{fixture, new_app};

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
    async fn list_applications_by_state_newest_first() {
        let pool = pool_in_memory().await.unwrap();
        let (listing_id, _) = insert_or_ignore(&pool, &fixture("greenhouse", "lbs"))
            .await
            .unwrap();
        let a = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let b = create_application(&pool, &new_app(&listing_id))
            .await
            .unwrap();

        // Both are created in the default state "tailored".
        set_application_state(&pool, &a.id, "rendered")
            .await
            .unwrap();
        set_application_state(&pool, &b.id, "rendered")
            .await
            .unwrap();

        let rows = list_applications_by_state(&pool, "rendered", 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        // Newest first (b was created after a).
        assert_eq!(rows[0].id, b.id);
        assert_eq!(rows[1].id, a.id);

        // Limit is honored.
        let one = list_applications_by_state(&pool, "rendered", 1)
            .await
            .unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, b.id);

        // Unrelated state returns empty.
        let subs = list_applications_by_state(&pool, "submitted", 10)
            .await
            .unwrap();
        assert!(subs.is_empty());
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
