//! ATS HTTP submitters (Greenhouse, Lever, Ashby).
//!
//! Each submitter's `prepare()` is complete: it builds a provider-
//! specific URL + a realistic candidate payload the live path would
//! POST. The live `submit()` currently short-circuits with
//! `SourceDisabled` because the read-only public APIs don't accept
//! submissions — that needs a Harvest/Lever/Ashby API-key path wired
//! in a follow-up (or the browser flow in M5). The flow still goes
//! end-to-end via `DryRunSubmitter`.

use async_trait::async_trait;
use serde::Serialize;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::error::{Result, SubmitError};

/// Maximum byte length of `WouldSubmit::body_preview`. Bounded so the
/// event log stays compact even for large payloads.
const BODY_PREVIEW_LIMIT: usize = 1024;

/// First ~256 chars of the cover letter embedded in the candidate body.
/// Enough to identify the letter in a dry-run event without dumping the
/// entire text into logs.
const COVER_PREVIEW_LIMIT: usize = 256;

#[derive(Debug, Serialize)]
struct CandidatePayload<'a> {
    name: &'a str,
    email: &'a str,
    phone: &'a str,
    listing_title: &'a str,
    listing_company: &'a str,
    listing_url: &'a str,
    resume_path: Option<&'a str>,
    cover_letter_preview: String,
    artifact_kinds: Vec<&'a str>,
}

impl<'a> CandidatePayload<'a> {
    fn from_ctx(ctx: &SubmitContext<'a>) -> Self {
        let resume_path = ctx
            .artifacts
            .iter()
            .find(|a| a.kind == "resume_docx" || a.kind == "resume_pdf")
            .map(|a| a.path.as_str());
        let cover_preview = truncate_chars(ctx.cover_letter_text, COVER_PREVIEW_LIMIT);
        let artifact_kinds = ctx.artifacts.iter().map(|a| a.kind.as_str()).collect();
        Self {
            name: ctx.profile.personal.name.as_str(),
            email: ctx.profile.personal.email.as_str(),
            phone: ctx.profile.personal.phone.as_str(),
            listing_title: ctx.listing.title.as_str(),
            listing_company: ctx.listing.company.as_str(),
            listing_url: ctx.listing.url.as_str(),
            resume_path,
            cover_letter_preview: cover_preview,
            artifact_kinds,
        }
    }
}

fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    // Truncate on a char boundary at/below max_bytes.
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

fn build_body_preview(payload: &CandidatePayload<'_>) -> Result<String> {
    let full = serde_json::to_string(payload)?;
    Ok(truncate_chars(&full, BODY_PREVIEW_LIMIT))
}

fn artifact_kinds_owned(ctx: &SubmitContext<'_>) -> Vec<String> {
    ctx.artifacts.iter().map(|a| a.kind.clone()).collect()
}

// --- Greenhouse ------------------------------------------------------------

const GREENHOUSE_DEFAULT_BASE: &str = "https://boards-api.greenhouse.io";

#[derive(Debug, Clone)]
pub struct GreenhouseSubmitter {
    base_url: String,
}

impl Default for GreenhouseSubmitter {
    fn default() -> Self {
        Self {
            base_url: GREENHOUSE_DEFAULT_BASE.to_owned(),
        }
    }
}

impl GreenhouseSubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the base URL (used by tests via wiremock).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn post_url(&self, ctx: &SubmitContext<'_>) -> String {
        // company slug is the listing's `company` lowercased with
        // non-alnum stripped — a best-effort stand-in for the Greenhouse
        // board token. Live submission would require the actual board
        // token + a Harvest API key.
        let company_slug = slugify(&ctx.listing.company);
        format!(
            "{}/v1/boards/{}/jobs/{}",
            self.base_url.trim_end_matches('/'),
            company_slug,
            sanitize_external_id(&ctx.listing.external_id)
        )
    }
}

#[async_trait]
impl Submitter for GreenhouseSubmitter {
    fn name(&self) -> &'static str {
        "greenhouse"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        let body_preview = build_body_preview(&payload)?;
        Ok(WouldSubmit {
            source: "greenhouse",
            url: self.post_url(ctx),
            method: "POST",
            body_preview,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, _ctx: &SubmitContext<'_>) -> Result<String> {
        Err(SubmitError::SourceDisabled(
            "greenhouse live submit requires Harvest API key — use dry-run or the browser flow in M5".to_owned(),
        ))
    }
}

// --- Lever -----------------------------------------------------------------

const LEVER_DEFAULT_BASE: &str = "https://api.lever.co";

#[derive(Debug, Clone)]
pub struct LeverSubmitter {
    base_url: String,
}

impl Default for LeverSubmitter {
    fn default() -> Self {
        Self {
            base_url: LEVER_DEFAULT_BASE.to_owned(),
        }
    }
}

impl LeverSubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn post_url(&self, ctx: &SubmitContext<'_>) -> String {
        let company_slug = slugify(&ctx.listing.company);
        format!(
            "{}/v0/postings/{}/{}",
            self.base_url.trim_end_matches('/'),
            company_slug,
            sanitize_external_id(&ctx.listing.external_id)
        )
    }
}

#[async_trait]
impl Submitter for LeverSubmitter {
    fn name(&self) -> &'static str {
        "lever"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        let body_preview = build_body_preview(&payload)?;
        Ok(WouldSubmit {
            source: "lever",
            url: self.post_url(ctx),
            method: "POST",
            body_preview,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, _ctx: &SubmitContext<'_>) -> Result<String> {
        Err(SubmitError::SourceDisabled(
            "lever live submit requires Lever API key — use dry-run or the browser flow in M5"
                .to_owned(),
        ))
    }
}

// --- Ashby -----------------------------------------------------------------

const ASHBY_DEFAULT_BASE: &str = "https://api.ashbyhq.com";

#[derive(Debug, Clone)]
pub struct AshbySubmitter {
    base_url: String,
}

impl Default for AshbySubmitter {
    fn default() -> Self {
        Self {
            base_url: ASHBY_DEFAULT_BASE.to_owned(),
        }
    }
}

impl AshbySubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn post_url(&self) -> String {
        format!(
            "{}/applicationForm.submit",
            self.base_url.trim_end_matches('/')
        )
    }
}

#[async_trait]
impl Submitter for AshbySubmitter {
    fn name(&self) -> &'static str {
        "ashby"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        let body_preview = build_body_preview(&payload)?;
        Ok(WouldSubmit {
            source: "ashby",
            url: self.post_url(),
            method: "POST",
            body_preview,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, _ctx: &SubmitContext<'_>) -> Result<String> {
        Err(SubmitError::SourceDisabled(
            "ashby live submit requires Ashby API key — use dry-run or the browser flow in M5"
                .to_owned(),
        ))
    }
}

// --- helpers ---------------------------------------------------------------

fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        }
    }
    if out.is_empty() {
        out.push_str("unknown");
    }
    out
}

/// Sanitize an external job id before interpolating it into a URL path.
///
/// The `external_id` field comes from third-party feeds (Greenhouse,
/// Lever, Naukri, ...). A malicious feed could supply
/// `../../admin` or `foo?bar=baz#frag` and have the URL parser route the
/// POST to an unintended endpoint when live submission lands. Allowed
/// characters: alphanumerics, `-`, `_`. Dots are deliberately rejected
/// because `..` segments are interpreted by URL parsers as parent
/// directories. Real job ids in observed feeds are alphanumeric with
/// hyphens/underscores; the strict allowlist costs nothing.
fn sanitize_external_id(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("unknown");
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn slugify_strips_non_alnum_and_lowercases() {
        assert_eq!(slugify("Acme Robotics"), "acmerobotics");
        assert_eq!(slugify("Foo, Inc."), "fooinc");
        assert_eq!(slugify(""), "unknown");
    }

    #[test]
    fn sanitize_external_id_blocks_path_traversal() {
        // Dots become `_` so `..` can never appear in the output.
        assert_eq!(sanitize_external_id("../../admin"), "______admin");
        assert_eq!(sanitize_external_id("job-42"), "job-42");
        assert_eq!(sanitize_external_id("job_42_v2"), "job_42_v2");
        assert_eq!(sanitize_external_id("job.v2"), "job_v2");
        assert_eq!(sanitize_external_id("foo/bar"), "foo_bar");
        assert_eq!(sanitize_external_id("foo?bar=baz#frag"), "foo_bar_baz_frag");
        assert_eq!(sanitize_external_id(""), "unknown");
    }

    #[test]
    fn truncate_chars_respects_char_boundaries() {
        let s = "héllo"; // 'é' is two bytes
        let t = truncate_chars(s, 2);
        // Must not split the multibyte char.
        assert!(s.starts_with(&t));
        assert!(t.is_char_boundary(t.len()));
    }
}
