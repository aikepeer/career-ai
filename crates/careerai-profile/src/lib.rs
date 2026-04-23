//! Profile ingestion and canonical schema.
//!
//! Turns resume files (PDF, DOCX) and LinkedIn export ZIPs into a merged
//! `profile.yaml` validated against the schema in [`schema`]. Parsers are
//! best-effort seeds — the produced YAML is expected to be hand-edited.

pub mod dates;
pub mod docx;
pub mod error;
pub mod heuristic;
pub mod linkedin;
pub mod merge;
pub mod pdf;
pub mod schema;

use std::path::Path;

use tracing::{info, warn};

pub use crate::error::{ProfileError, Result};
pub use crate::schema::Profile;

/// Import one or more source files into a single merged [`Profile`].
///
/// Supported extensions: `.pdf`, `.docx`, `.zip` (LinkedIn export). Files
/// are parsed in the order given; later files overlay earlier ones via
/// [`merge::merge_pair`]. Unsupported extensions fail fast.
pub fn import_paths(paths: &[&Path]) -> Result<Profile> {
    let mut parsed = Vec::with_capacity(paths.len());
    for path in paths {
        let p = parse_one(path)?;
        parsed.push(p);
    }
    Ok(merge::merge_all(parsed))
}

fn parse_one(path: &Path) -> Result<Profile> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);
    match ext.as_deref() {
        Some("pdf") => {
            info!(path = %path.display(), "parsing PDF");
            let text = pdf::extract_text_from_path(path)?;
            Ok(heuristic::parse(&text))
        }
        Some("docx") => {
            info!(path = %path.display(), "parsing DOCX");
            let text = docx::extract_text_from_path(path)?;
            Ok(heuristic::parse(&text))
        }
        Some("zip") => {
            info!(path = %path.display(), "parsing LinkedIn export");
            linkedin::parse_export_from_path(path)
        }
        Some(other) => {
            warn!(path = %path.display(), ext = other, "unsupported extension");
            Err(ProfileError::UnsupportedFormat(other.to_string()))
        }
        None => Err(ProfileError::UnsupportedFormat(path.display().to_string())),
    }
}
