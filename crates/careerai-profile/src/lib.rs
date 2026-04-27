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
pub mod llm_extract;
pub mod merge;
pub mod pdf;
pub mod schema;

use std::path::Path;

use tracing::{info, warn};

pub use crate::error::{ProfileError, Result};
pub use crate::llm_extract::{
    extract_profile_from_text, ExtractError, ExtractOptions, ExtractRequest, LlmCaller,
};
pub use crate::schema::Profile;

/// Import one or more source files into a single merged [`Profile`].
///
/// Supported extensions: `.pdf`, `.docx`, `.zip` (LinkedIn export). Files
/// are parsed via the heuristic regex parser (PDF/DOCX) and the LinkedIn
/// CSV adapter (ZIP). LinkedIn-derived profiles are processed FIRST so
/// their structured data wins on scalar conflicts during the merge —
/// see [`merge::merge_pair`] for the precedence rule.
///
/// Unsupported extensions fail fast.
pub fn import_paths(paths: &[&Path]) -> Result<Profile> {
    import_paths_with_llm(paths, None)
}

/// Like [`import_paths`] but optionally routes PDF/DOCX text through an
/// LLM-backed extractor. When `llm` is `None`, falls back to the
/// heuristic regex parser (the M1 default) — same behavior as
/// [`import_paths`].
///
/// LinkedIn ZIPs always use the structured CSV path regardless of the
/// `llm` argument; LinkedIn already gives us reliably-shaped data.
pub fn import_paths_with_llm(
    paths: &[&Path],
    llm: Option<&LlmExtractContext<'_>>,
) -> Result<Profile> {
    // LinkedIn-first ordering: zips contribute base scalars (which win
    // in `merge_pair`); PDF/DOCX merge later and only fill gaps.
    let mut sorted: Vec<&&Path> = paths.iter().collect();
    sorted.sort_by_key(|p| {
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase);
        match ext.as_deref() {
            Some("zip") => 0,
            _ => 1,
        }
    });

    let mut parsed = Vec::with_capacity(sorted.len());
    for path in sorted {
        let p = parse_one(path, llm)?;
        parsed.push(p);
    }
    Ok(merge::merge_all(parsed))
}

/// Bundle of the bits [`import_paths_with_llm`] needs to drive an LLM
/// extraction. Held by reference so we don't clone the caller / opts on
/// every file.
pub struct LlmExtractContext<'a> {
    pub caller: &'a dyn LlmCaller,
    pub options: ExtractOptions,
}

impl std::fmt::Debug for LlmExtractContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `caller` is a trait object whose concrete impl may hold an API
        // key in its private state; never derive Debug for it.
        f.debug_struct("LlmExtractContext")
            .field("caller", &"<dyn LlmCaller>")
            .field("options", &self.options)
            .finish()
    }
}

impl<'a> LlmExtractContext<'a> {
    pub fn new(caller: &'a dyn LlmCaller, options: ExtractOptions) -> Self {
        Self { caller, options }
    }
}

fn parse_one(path: &Path, llm: Option<&LlmExtractContext<'_>>) -> Result<Profile> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);
    match ext.as_deref() {
        Some("pdf") => {
            info!(path = %path.display(), "parsing PDF");
            let text = pdf::extract_text_from_path(path)?;
            parse_text_with_optional_llm(&text, llm)
        }
        Some("docx") => {
            info!(path = %path.display(), "parsing DOCX");
            let text = docx::extract_text_from_path(path)?;
            parse_text_with_optional_llm(&text, llm)
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

fn parse_text_with_optional_llm(
    text: &str,
    llm: Option<&LlmExtractContext<'_>>,
) -> Result<Profile> {
    let Some(ctx) = llm else {
        return Ok(heuristic::parse(text));
    };

    // Synchronously drive the async extractor. We're called from a sync
    // function (`import_paths*`) which itself is invoked by the CLI's
    // `#[tokio::main]` runtime, so a tokio Handle is usually already in
    // scope. Use `block_in_place` to step out of the worker; fall back
    // to a fresh single-threaded runtime when no Handle exists (e.g.
    // a sync test).
    let result = if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let fut = extract_profile_from_text(text, ctx.caller, &ctx.options);
        tokio::task::block_in_place(|| handle.block_on(fut))
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(ProfileError::Io)?;
        let fut = extract_profile_from_text(text, ctx.caller, &ctx.options);
        rt.block_on(fut)
    };

    match result {
        Ok(profile) => Ok(profile),
        Err(e) => Ok(llm_fallback(text, &e)),
    }
}

fn llm_fallback(text: &str, err: &ExtractError) -> Profile {
    warn!(
        target: "profile.llm_extract",
        error = %err,
        "LLM extraction failed; falling back to heuristic parser",
    );
    heuristic::parse(text)
}
