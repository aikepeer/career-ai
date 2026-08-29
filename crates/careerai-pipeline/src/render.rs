//! Render stage — emit DOCX + PDF artifacts for a tailored
//! application via Tera → Markdown → pandoc, attach artifact rows,
//! and transition both listing and application to `rendered`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::models::NewArtifact;
use careerai_db::queries;
use careerai_render::render_application;
use careerai_tailor::model::{CoverLetter, ResumeView};

use crate::{load_profile, open_pool};

#[derive(Debug)]
pub struct RenderedOutcome {
    pub application_id: String,
    pub resume_md: PathBuf,
    pub resume_docx: PathBuf,
    pub resume_pdf: PathBuf,
    pub cover_md: PathBuf,
    pub cover_docx: PathBuf,
    pub bytes: BTreeMap<PathBuf, u64>,
}

/// Render a tailored application to DOCX + PDF on disk, attach artifact rows,
/// and transition both listing and application to `rendered`.
pub async fn render_one(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
) -> Result<RenderedOutcome> {
    let pool = open_pool(root).await?;

    let application = match queries::find_application_by_id(&pool, application_id).await {
        Ok(a) => a,
        Err(careerai_db::DbError::NotFound(_)) => {
            match queries::find_latest_application_for_listing(&pool, application_id).await {
                Ok(Some(a)) => a,
                _ => anyhow::bail!("application not found: {application_id}"),
            }
        }
        Err(e) => return Err(e).context("fetch application"),
    };

    let app_id = application.id.clone();

    if application.state != "tailored" && application.state != "rendered" {
        anyhow::bail!(
            "application {app_id} is in state '{}'; expected 'tailored' or 'rendered'",
            application.state
        );
    }

    let payload = match queries::find_payload_by_application_id(&pool, &app_id).await {
        Ok(p) => p,
        Err(careerai_db::DbError::NotFound(_)) => {
            anyhow::bail!("application payload not found: {app_id}");
        }
        Err(e) => return Err(e).context("fetch application payload"),
    };

    let resume_view: ResumeView =
        serde_json::from_str(&payload.resume_view_json).context("deserialize resume_view_json")?;
    let cover_letter = CoverLetter {
        body: payload.cover_letter_text.clone(),
    };

    let listing = queries::find_by_id(&pool, &application.listing_id)
        .await
        .context("fetch listing for application")?;
    let profile = load_profile(root)?;

    // Resolve artifacts_dir against `root` if it's relative so the CLI and
    // integration tests share the same on-disk layout regardless of cwd.
    let mut render_cfg = cfg.render.clone();
    if render_cfg.artifacts_dir.is_relative() {
        render_cfg.artifacts_dir = root.join(&render_cfg.artifacts_dir);
    }

    let artifacts = render_application(
        &render_cfg,
        &application.id,
        &resume_view,
        &cover_letter,
        &profile.personal.name,
        &listing.company,
    )
    .await
    .context("render_application")?;

    // Attach artifact rows for each rendered file that exists on disk.
    for (kind, path) in [
        ("resume_md", &artifacts.resume_md),
        ("resume_docx", &artifacts.resume_docx),
        ("resume_pdf", &artifacts.resume_pdf),
        ("cover_md", &artifacts.cover_md),
        ("cover_docx", &artifacts.cover_docx),
    ] {
        if let Some(&size) = artifacts.bytes.get(path) {
            if size > 0 {
                queries::attach_artifact(
                    &pool,
                    &application.id,
                    &NewArtifact {
                        kind: kind.to_string(),
                        path: path.to_string_lossy().into_owned(),
                        bytes: i64::try_from(size).unwrap_or(i64::MAX),
                    },
                )
                .await
                .with_context(|| format!("attach_artifact {kind}"))?;
            }
        }
    }

    queries::transition(
        &pool,
        &application.listing_id,
        ListingState::Rendered,
        Some(&format!("app={}", application.id)),
    )
    .await
    .context("transition listing to rendered")?;
    queries::set_application_state(&pool, &application.id, "rendered")
        .await
        .context("set application state=rendered")?;

    Ok(RenderedOutcome {
        application_id: application.id,
        resume_md: artifacts.resume_md,
        resume_docx: artifacts.resume_docx,
        resume_pdf: artifacts.resume_pdf,
        cover_md: artifacts.cover_md,
        cover_docx: artifacts.cover_docx,
        bytes: artifacts.bytes,
    })
}

/// Render all currently tailored or previously rendered applications in sequence.
pub async fn render_all(root: &Path, cfg: &CoreConfig) -> Result<Vec<RenderedOutcome>> {
    let pool = open_pool(root).await?;
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM applications WHERE state IN ('tailored', 'rendered')")
            .fetch_all(&pool)
            .await
            .context("fetch tailored/rendered applications")?;

    let mut outcomes = Vec::with_capacity(rows.len());
    for (id,) in rows {
        match render_one(root, cfg, &id).await {
            Ok(outcome) => outcomes.push(outcome),
            Err(e) => {
                tracing::warn!(application_id = %id, error = %e, "render_all: failed to render application");
            }
        }
    }
    Ok(outcomes)
}
