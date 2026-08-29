//! Rendering to DOCX + PDF.
//!
//! Templating via `tera` → Markdown → `pandoc` subprocess. Single-column,
//! ATS-safe output. Pandoc is an external runtime dependency; install via
//! `apt install pandoc` / `brew install pandoc`.

#![forbid(unsafe_code)]

pub mod artifacts;
pub mod config;
pub mod error;
pub mod pandoc;
pub mod templates;

use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::{debug, info};

pub use crate::artifacts::{layout_for, ArtifactLayout};
pub use crate::config::RenderConfig;
pub use crate::error::{RenderError, Result};

#[derive(Debug, Clone)]
pub struct RenderedArtifacts {
    pub resume_md: PathBuf,
    pub resume_docx: PathBuf,
    pub resume_pdf: PathBuf,
    pub cover_md: PathBuf,
    pub cover_docx: PathBuf,
    pub bytes: BTreeMap<PathBuf, u64>,
}

/// Render a tailored resume + cover letter into an application-scoped
/// artifact directory. Wipes and recreates the directory to guarantee a
/// clean slate per render.
///
/// `app_id` MUST be a valid UUID string — we use it as a path component
/// and reject anything else before touching the filesystem. Production
/// callers thread `Uuid::now_v7().to_string()` here, so the failure
/// path only triggers under developer error / tampering.
pub async fn render_application(
    cfg: &RenderConfig,
    app_id: &str,
    view: &careerai_tailor::model::ResumeView,
    letter: &careerai_tailor::model::CoverLetter,
    personal_name: &str,
    listing_company: &str,
) -> Result<RenderedArtifacts> {
    // Defense against path traversal through a caller-supplied app_id.
    // UUIDs are 8-4-4-4-12 hex with hyphens — a format that forbids `..`,
    // `/`, backslashes, and NUL bytes. Any other shape is rejected.
    if uuid::Uuid::parse_str(app_id).is_err() {
        return Err(RenderError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("app_id is not a valid UUID: {app_id:?}"),
        )));
    }

    let layout = layout_for(cfg, app_id);
    info!(
        target = "render",
        app_id,
        root = %layout.root.display(),
        "rendering application"
    );

    // Probe pandoc once up front — fail fast before any FS work if the
    // binary is missing. Also cheaper than three `which` probes later.
    let pandoc_bin = pandoc::resolve_pandoc_bin(cfg)?;

    // Wipe + recreate the per-application directory.
    match tokio::fs::remove_dir_all(&layout.root).await {
        Ok(()) => debug!(target = "render", "existing artifacts dir wiped"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(RenderError::Io(e)),
    }
    tokio::fs::create_dir_all(&layout.root).await?;

    let today = today_iso();

    // Resume markdown.
    let resume_md = templates::render_resume(view, personal_name)?;
    tokio::fs::write(&layout.resume_md, resume_md.as_bytes()).await?;

    // Resume HTML (executive format matching original layout).
    let resume_html = templates::render_resume_html(view, personal_name, None)?;
    tokio::fs::write(&layout.resume_html, resume_html.as_bytes()).await?;

    // Cover letter markdown.
    let cover_md = templates::render_cover_letter(letter, personal_name, listing_company, &today)?;
    tokio::fs::write(&layout.cover_md, cover_md.as_bytes()).await?;

    // Conversions run in parallel.
    let (docx_res, pdf_res, cover_res) = tokio::join!(
        pandoc::md_to_docx_with_bin(&pandoc_bin, &layout.resume_md, &layout.resume_docx, cfg),
        pandoc::html_to_pdf(&layout.resume_html, &layout.resume_pdf, cfg),
        pandoc::md_to_docx_with_bin(&pandoc_bin, &layout.cover_md, &layout.cover_docx, cfg),
    );
    docx_res?;
    cover_res?;
    if let Err(e) = pdf_res {
        tracing::warn!(target = "render", error = %e, "PDF compilation failed or engine missing; DOCX artifacts preserved");
    }

    let bytes = stat_all(&layout).await?;

    if !cfg.keep_intermediate_markdown {
        let _ = tokio::fs::remove_file(&layout.resume_md).await;
        let _ = tokio::fs::remove_file(&layout.resume_html).await;
        let _ = tokio::fs::remove_file(&layout.cover_md).await;
    }

    Ok(RenderedArtifacts {
        resume_md: layout.resume_md,
        resume_docx: layout.resume_docx,
        resume_pdf: layout.resume_pdf,
        cover_md: layout.cover_md,
        cover_docx: layout.cover_docx,
        bytes,
    })
}

async fn stat_all(layout: &ArtifactLayout) -> Result<BTreeMap<PathBuf, u64>> {
    let mut out = BTreeMap::new();
    for p in [
        &layout.resume_md,
        &layout.resume_html,
        &layout.resume_docx,
        &layout.resume_pdf,
        &layout.cover_md,
        &layout.cover_docx,
    ] {
        if let Ok(meta) = tokio::fs::metadata(p).await {
            out.insert(p.clone(), meta.len());
        }
    }
    Ok(out)
}

/// Today's date as `YYYY-MM-DD` (UTC). Intentionally lightweight — the
/// cover letter template tolerates any ISO-ish string, and we avoid
/// pulling `chrono` just for this.
fn today_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (y, m, d) = epoch_to_ymd(secs);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Civil date from unix seconds (UTC). Howard Hinnant's proleptic
/// Gregorian algorithm (days_from_civil inverse). `secs` is clamped
/// implicitly by `u64`, so values fit comfortably inside `i64`.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]
fn epoch_to_ymd(secs: u64) -> (i64, u32, u32) {
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
                                  // Month/day are always in [1, 31] / [1, 12] — truncation to u32 is safe.
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn today_iso_is_well_formed() {
        let s = today_iso();
        assert_eq!(s.len(), 10, "expected YYYY-MM-DD, got {s}");
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
    }

    #[test]
    fn epoch_to_ymd_known_dates() {
        // 2026-04-24 00:00:00 UTC = 1_776_988_800
        assert_eq!(epoch_to_ymd(1_776_988_800), (2026, 4, 24));
        // 1970-01-01
        assert_eq!(epoch_to_ymd(0), (1970, 1, 1));
        // 2000-01-01
        assert_eq!(epoch_to_ymd(946_684_800), (2000, 1, 1));
    }
}
