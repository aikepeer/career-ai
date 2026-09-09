#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
//! F03: Unified application review queue.
//!
//! Shows applications awaiting explicit approval (state `rendered` or
//! `prepared`) with their JD, resume diff, cover letter, and artifacts
//! together. Supports approve, skip, and retry with content-version tracking
//! so stale or repeated clicks cannot send duplicates.
//!
//! Safety invariants:
//! - Approval identifies the exact application/content version via
//!   `profile_hash` + `prompt_version` + `updated_at`. A stale click whose
//!   content version no longer matches is rejected.
//! - Editing the resume or cover letter invalidates the current approval
//!   by bumping `updated_at` and clearing any pending approval flag.
//! - This module never sends submissions. Approval marks the application
//!   `approved`; the submit layer picks it up separately.

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::error::Result;
use crate::view::ApplicationDetail;

/// A single entry in the review queue.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReviewQueueEntry {
    pub application_id: String,
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub state: String,
    pub score: Option<f32>,
    pub profile_hash: String,
    pub prompt_version: String,
    pub llm_model: String,
    pub has_resume: bool,
    pub has_cover_letter: bool,
    pub artifact_count: usize,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// The content version that identifies an exact application snapshot.
/// Two approvals with the same version are idempotent; a version mismatch
/// means the content changed and the approval is stale.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ContentVersion {
    pub application_id: String,
    pub profile_hash: String,
    pub prompt_version: String,
    pub updated_at_rfc3339: String,
}

/// Request body for approve / skip / retry actions.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewAction {
    /// The content version the user saw when they clicked. If it no longer
    /// matches the current application, the action is rejected as stale.
    pub content_version: ContentVersion,
    pub note: Option<String>,
}

/// Fetch applications awaiting review (state `rendered` or `prepared`).
pub async fn fetch_review_queue(pool: &SqlitePool) -> Result<Vec<ReviewQueueEntry>> {
    let rows = sqlx::query(
        "SELECT a.id, a.listing_id, a.profile_hash, a.prompt_version, a.llm_model, a.updated_at, \
                l.title, l.company, l.state, l.score \
         FROM applications a \
         JOIN listings l ON l.id = a.listing_id \
         WHERE a.state IN ('rendered', 'prepared') \
         ORDER BY a.updated_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(careerai_db::DbError::from)?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let application_id: String = row.try_get("id").unwrap_or_default();
        if application_id.is_empty() {
            continue;
        }
        let listing_id: String = row.try_get("listing_id").unwrap_or_default();
        let has_resume = has_payload_field(pool, &application_id, "resume_view_json").await?;
        let has_cover_letter = has_payload_field(pool, &application_id, "cover_letter_text").await?;
        let artifact_count = count_artifacts(pool, &application_id).await?;
        let score_f64: Option<f64> = row.try_get("score").ok();
        out.push(ReviewQueueEntry {
            application_id,
            listing_id,
            title: row.try_get("title").unwrap_or_default(),
            company: row.try_get("company").unwrap_or_default(),
            state: row.try_get("state").unwrap_or_default(),
            score: score_f64.map(|s| s as f32),
            profile_hash: row.try_get("profile_hash").unwrap_or_default(),
            prompt_version: row.try_get("prompt_version").unwrap_or_default(),
            llm_model: row.try_get("llm_model").unwrap_or_default(),
            has_resume,
            has_cover_letter,
            artifact_count,
            updated_at: row.try_get("updated_at").unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Fetch the full review detail for a single application (JD + resume +
/// cover letter + artifacts + timeline).
pub async fn fetch_review_detail(pool: &SqlitePool, application_id: &str) -> Result<Option<ApplicationDetail>> {
    crate::details::fetch_application_detail(pool, application_id).await
}

/// Approve an application for submission. Idempotent: if the application is
/// already `approved` with the same content version, returns Ok without
/// changing state. Returns `Err` if the content version is stale.
pub async fn approve(pool: &SqlitePool, action: &ReviewAction) -> Result<ApproveOutcome> {
    let mut tx = pool.begin().await.map_err(careerai_db::DbError::from)?;
    let row = sqlx::query("SELECT id, state, profile_hash, prompt_version, updated_at FROM applications WHERE id = ?")
        .bind(&action.content_version.application_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(careerai_db::DbError::from)?;
    let Some(row) = row else {
        tx.rollback().await.ok();
        return Ok(ApproveOutcome::NotFound);
    };
    let state: String = row.try_get("state").unwrap_or_default();
    let profile_hash: String = row.try_get("profile_hash").unwrap_or_default();
    let prompt_version: String = row.try_get("prompt_version").unwrap_or_default();
    let updated_at: chrono::DateTime<chrono::Utc> = row.try_get("updated_at").unwrap_or_default();
    let current_version = ContentVersion {
        application_id: action.content_version.application_id.clone(),
        profile_hash: profile_hash.clone(),
        prompt_version: prompt_version.clone(),
        updated_at_rfc3339: updated_at.to_rfc3339(),
    };
    if current_version != action.content_version {
        tx.rollback().await.ok();
        return Ok(ApproveOutcome::StaleContent);
    }
    if state == "approved" {
        tx.rollback().await.ok();
        return Ok(ApproveOutcome::AlreadyApproved);
    }
    sqlx::query("UPDATE applications SET state = 'approved', updated_at = ? WHERE id = ?")
        .bind(chrono::Utc::now())
        .bind(&action.content_version.application_id)
        .execute(&mut *tx)
        .await
        .map_err(careerai_db::DbError::from)?;
    tx.commit().await.map_err(careerai_db::DbError::from)?;
    Ok(ApproveOutcome::Approved)
}

/// Skip an application — moves it to `skipped` state. Idempotent.
pub async fn skip(pool: &SqlitePool, action: &ReviewAction) -> Result<SkipOutcome> {
    let mut tx = pool.begin().await.map_err(careerai_db::DbError::from)?;
    let row = sqlx::query("SELECT id, state, profile_hash, prompt_version, updated_at FROM applications WHERE id = ?")
        .bind(&action.content_version.application_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(careerai_db::DbError::from)?;
    let Some(row) = row else {
        tx.rollback().await.ok();
        return Ok(SkipOutcome::NotFound);
    };
    let state: String = row.try_get("state").unwrap_or_default();
    if state == "skipped" {
        tx.rollback().await.ok();
        return Ok(SkipOutcome::AlreadySkipped);
    }
    sqlx::query("UPDATE applications SET state = 'skipped', updated_at = ? WHERE id = ?")
        .bind(chrono::Utc::now())
        .bind(&action.content_version.application_id)
        .execute(&mut *tx)
        .await
        .map_err(careerai_db::DbError::from)?;
    tx.commit().await.map_err(careerai_db::DbError::from)?;
    Ok(SkipOutcome::Skipped)
}

/// Retry a failed application — moves it back to `rendered` for re-review.
pub async fn retry(pool: &SqlitePool, action: &ReviewAction) -> Result<RetryOutcome> {
    let mut tx = pool.begin().await.map_err(careerai_db::DbError::from)?;
    let row = sqlx::query("SELECT id, state FROM applications WHERE id = ?")
        .bind(&action.content_version.application_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(careerai_db::DbError::from)?;
    let Some(row) = row else {
        tx.rollback().await.ok();
        return Ok(RetryOutcome::NotFound);
    };
    let state: String = row.try_get("state").unwrap_or_default();
    if state != "failed" {
        tx.rollback().await.ok();
        return Ok(RetryOutcome::NotFailed);
    }
    sqlx::query("UPDATE applications SET state = 'rendered', updated_at = ? WHERE id = ?")
        .bind(chrono::Utc::now())
        .bind(&action.content_version.application_id)
        .execute(&mut *tx)
        .await
        .map_err(careerai_db::DbError::from)?;
    tx.commit().await.map_err(careerai_db::DbError::from)?;
    Ok(RetryOutcome::Retried)
}

/// Invalidate an application's approval after an edit. Bumps `updated_at`
/// and moves back to `rendered` so it re-enters the review queue.
pub async fn invalidate_after_edit(pool: &SqlitePool, application_id: &str) -> Result<()> {
    sqlx::query("UPDATE applications SET state = 'rendered', updated_at = ? WHERE id = ?")
        .bind(chrono::Utc::now())
        .bind(application_id)
        .execute(pool)
        .await
        .map_err(careerai_db::DbError::from)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApproveOutcome {
    Approved,
    AlreadyApproved,
    StaleContent,
    NotFound,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipOutcome {
    Skipped,
    AlreadySkipped,
    NotFound,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetryOutcome {
    Retried,
    NotFailed,
    NotFound,
}

async fn has_payload_field(pool: &SqlitePool, app_id: &str, field: &str) -> Result<bool> {
    let sql = format!(
        "SELECT {field} FROM application_payloads WHERE application_id = ?"
    );
    let row = sqlx::query(&sql)
        .bind(app_id)
        .fetch_optional(pool)
        .await
        .map_err(careerai_db::DbError::from)?;
    Ok(match row {
        Some(r) => {
            let val: Option<String> = r.try_get(field).unwrap_or_default();
            val.is_some_and(|v| !v.is_empty())
        }
        None => false,
    })
}

async fn count_artifacts(pool: &SqlitePool, app_id: &str) -> Result<usize> {
    let row = sqlx::query("SELECT COUNT(*) as n FROM artifacts WHERE application_id = ?")
        .bind(app_id)
        .fetch_one(pool)
        .await
        .map_err(careerai_db::DbError::from)?;
    let n: i64 = row.try_get("n").unwrap_or(0);
    Ok(usize::try_from(n).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_version_equality_detects_stale_clicks() {
        let v1 = ContentVersion {
            application_id: "app-1".into(),
            profile_hash: "sha256:abc".into(),
            prompt_version: "v1".into(),
            updated_at_rfc3339: "2026-09-09T10:00:00Z".into(),
        };
        let v2 = ContentVersion {
            application_id: "app-1".into(),
            profile_hash: "sha256:abc".into(),
            prompt_version: "v1".into(),
            updated_at_rfc3339: "2026-09-09T10:00:00Z".into(),
        };
        let v3 = ContentVersion {
            application_id: "app-1".into(),
            profile_hash: "sha256:abc".into(),
            prompt_version: "v1".into(),
            updated_at_rfc3339: "2026-09-09T11:00:00Z".into(), // edited later
        };
        assert_eq!(v1, v2, "same content version must be equal");
        assert_ne!(v1, v3, "different updated_at must be stale");
    }
}
