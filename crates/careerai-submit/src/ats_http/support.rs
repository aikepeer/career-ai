use serde::Serialize;

use crate::base::SubmitContext;
use crate::error::Result;

/// Maximum byte length of `WouldSubmit::body_preview`.
const BODY_PREVIEW_LIMIT: usize = 1024;

/// First ~256 chars of the cover letter embedded in the candidate body.
const COVER_PREVIEW_LIMIT: usize = 256;

#[derive(Debug, Serialize)]
pub(super) struct CandidatePayload<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub phone: &'a str,
    pub listing_title: &'a str,
    pub listing_company: &'a str,
    pub listing_url: &'a str,
    pub resume_path: Option<&'a str>,
    pub cover_letter_preview: String,
    pub artifact_kinds: Vec<&'a str>,
}

impl<'a> CandidatePayload<'a> {
    pub fn from_ctx(ctx: &SubmitContext<'a>) -> Self {
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

pub(super) fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

pub(super) fn build_body_preview(payload: &CandidatePayload<'_>) -> Result<String> {
    let full = serde_json::to_string(payload)?;
    Ok(truncate_chars(&full, BODY_PREVIEW_LIMIT))
}

pub(super) fn artifact_kinds_owned(ctx: &SubmitContext<'_>) -> Vec<String> {
    ctx.artifacts.iter().map(|a| a.kind.clone()).collect()
}

/// Slugify a string to alphanumeric lowercase.
pub(super) fn slugify(s: &str) -> String {
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
pub(crate) fn sanitize_external_id(s: &str) -> String {
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
