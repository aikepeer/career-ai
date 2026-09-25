//! Hand-rolled SQL for `artifacts` — paths to rendered DOCX/PDF
//! files associated with an application.

use sqlx::SqlitePool;

use crate::error::Result;
use crate::models::{Artifact, NewArtifact};

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

/// All artifact file paths ever registered by the render pipeline.
/// Used by the dashboard download endpoint to serve only files the
/// pipeline itself produced (never `credentials.json` or config).
pub async fn all_artifact_paths(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM artifacts")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(p,)| p).collect())
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
}
