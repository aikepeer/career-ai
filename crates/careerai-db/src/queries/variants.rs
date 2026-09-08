//! A/B variant tracking queries.
//!
//! Records which resume variant was submitted for each application and
//! whether it received a response. The dashboard's experiment tab uses
//! `list_variants_with_outcome` to show conversion rates per variant
//! label so the tailor prompt can learn which phrasing patterns convert.

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::error::Result;

/// A variant row joined with its application + listing to surface the
/// company and title alongside the outcome.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct VariantWithOutcome {
    pub id: i64,
    pub application_id: String,
    pub variant_label: String,
    pub variant_metadata: Option<String>,
    pub submitted_at: Option<String>,
    pub response_status: String,
    pub response_type: Option<String>,
    pub company: String,
    pub title: String,
    /// R16: content version hash (resume diff hash) for provenance.
    #[sqlx(default)]
    pub content_version: Option<String>,
}

/// Record a new A/B variant for an application. Returns the inserted row id.
///
/// R16: populates `submitted_at` at record time (the variant is recorded
/// *after* submission succeeds, so the submission timestamp is known).
/// Accepts `content_version` — a hash of the resume diff — so outcomes
/// can be attributed to a specific resume treatment, not just an A/B label.
/// Uses `INSERT ... ON CONFLICT DO UPDATE` to prevent duplicate records
/// for one application+label assignment (the unique index
/// `uq_variants_app_label` enforces this).
pub async fn record_variant(
    pool: &SqlitePool,
    application_id: &str,
    variant_label: &str,
    variant_metadata: Option<&str>,
    content_version: Option<&str>,
) -> Result<i64> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO application_variants
            (application_id, variant_label, variant_metadata, content_version, submitted_at)
         VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(application_id, variant_label) DO UPDATE SET
            variant_metadata = excluded.variant_metadata,
            content_version = excluded.content_version,
            submitted_at = excluded.submitted_at
         RETURNING id",
    )
    .bind(application_id)
    .bind(variant_label)
    .bind(variant_metadata)
    .bind(content_version)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn list_variants_with_outcome(pool: &SqlitePool) -> Result<Vec<VariantWithOutcome>> {
    let rows = sqlx::query_as(
        "SELECT av.id              AS id,
                av.application_id  AS application_id,
                av.variant_label   AS variant_label,
                av.variant_metadata AS variant_metadata,
                av.submitted_at    AS submitted_at,
                av.response_status AS response_status,
                av.response_type   AS response_type,
                av.content_version AS content_version,
                l.company          AS company,
                l.title            AS title
         FROM application_variants av
         JOIN applications a ON a.id = av.application_id
         JOIN listings l ON l.id = a.listing_id
         ORDER BY av.created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Update a variant's response status and type after a response is
/// received (or after a timeout marks it ghosted).
pub async fn update_variant_response(
    pool: &SqlitePool,
    id: i64,
    response_status: &str,
    response_type: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE application_variants
         SET response_status = ?, response_type = ?, response_at = datetime('now')
         WHERE id = ?",
    )
    .bind(response_status)
    .bind(response_type)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
