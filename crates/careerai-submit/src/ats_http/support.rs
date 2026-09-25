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

use std::time::Duration;

use reqwest::Client;

use crate::error::SubmitError;

const RESUME_KINDS: &[&str] = &["resume_pdf", "resume_docx"];
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

/// Locate the resume artifact path in a submit context.
pub(super) fn resume_artifact_path<'a>(ctx: &'a SubmitContext<'_>) -> Result<&'a str> {
    ctx.artifacts
        .iter()
        .find(|a| RESUME_KINDS.contains(&a.kind.as_str()))
        .map(|a| a.path.as_str())
        .ok_or_else(|| {
            SubmitError::MissingData("no resume artifact (resume_pdf or resume_docx)".into())
        })
}

/// Read the resume artifact bytes + filename from disk.
pub(super) async fn read_resume(ctx: &SubmitContext<'_>) -> Result<(Vec<u8>, String)> {
    let path = resume_artifact_path(ctx)?;
    let bytes = tokio::fs::read(path).await.map_err(SubmitError::Io)?;
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("resume")
        .to_string();
    Ok((bytes, filename))
}

/// Split a full name into `(first, last)`. A missing last name is `""`.
pub(super) fn split_name(full: &str) -> (&str, &str) {
    let trimmed = full.trim();
    match trimmed.split_once(' ') {
        Some((first, rest)) => (first, rest.trim()),
        None => (trimmed, ""),
    }
}

/// Tail of a response body for bounded error reporting.
pub(super) fn body_tail(bytes: &[u8], max: usize) -> String {
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

/// Shared reqwest client. Per-request timeouts are applied at each call
/// site so a hung ATS can't stall a tick.
pub(super) fn http_client() -> Client {
    Client::builder()
        .user_agent("careerai/0.1 (+https://github.com/justdoGIT/career-ai)")
        .build()
        .unwrap_or_default()
}

/// POST a JSON body and return the response text, or an error carrying the
/// status and a bounded tail of the body.
pub(super) async fn post_json(
    client: &Client,
    url: &str,
    body: serde_json::Value,
) -> Result<String> {
    let resp = client
        .post(url)
        .json(&body)
        .timeout(HTTP_TIMEOUT)
        .send()
        .await?;
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap_or_default();
    if !status.is_success() {
        return Err(SubmitError::HttpStatus {
            status: status.as_u16(),
            body_tail: body_tail(&bytes, 1024),
        });
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// POST a multipart form and return the response text.
pub(super) async fn post_multipart(
    client: &Client,
    url: &str,
    form: reqwest::multipart::Form,
    referer: Option<&str>,
) -> Result<String> {
    let mut req = client.post(url).multipart(form).timeout(HTTP_TIMEOUT);
    if let Some(r) = referer {
        req = req.header(reqwest::header::REFERER, r);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let bytes = resp.bytes().await.unwrap_or_default();
    if !status.is_success() {
        return Err(SubmitError::HttpStatus {
            status: status.as_u16(),
            body_tail: body_tail(&bytes, 1024),
        });
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
