use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use reqwest::Client;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::error::Result;

use super::support::{
    artifact_kinds_owned, build_body_preview, http_client, post_json, post_multipart, read_resume,
    sanitize_external_id, slugify, split_name, CandidatePayload,
};

const GREENHOUSE_DEFAULT_BASE: &str = "https://boards-api.greenhouse.io";
const LEVER_DEFAULT_BASE: &str = "https://api.lever.co";
const ASHBY_DEFAULT_BASE: &str = "https://api.ashbyhq.com";

// --- Greenhouse ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GreenhouseSubmitter {
    base_url: String,
    http: Client,
}

impl Default for GreenhouseSubmitter {
    fn default() -> Self {
        Self {
            base_url: GREENHOUSE_DEFAULT_BASE.to_owned(),
            http: http_client(),
        }
    }
}

impl GreenhouseSubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn board_token<'a>(ctx: &'a SubmitContext<'_>) -> std::borrow::Cow<'a, str> {
        board_token_from_url(&ctx.listing.url).map_or_else(
            || std::borrow::Cow::Owned(slugify(&ctx.listing.company)),
            std::borrow::Cow::Borrowed,
        )
    }

    fn post_url(&self, ctx: &SubmitContext<'_>) -> String {
        format!(
            "{}/v1/boards/{}/jobs/{}",
            self.base_url.trim_end_matches('/'),
            Self::board_token(ctx),
            sanitize_external_id(&ctx.listing.external_id),
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

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        // Greenhouse's hosted application form POSTs JSON to the public
        // boards endpoint. The resume is base64-encoded inline.
        let (resume, filename) = read_resume(ctx).await?;
        let (first, last) = split_name(&ctx.profile.personal.name);
        let body = serde_json::json!({
            "first_name": first,
            "last_name": last,
            "email": ctx.profile.personal.email,
            "phone": ctx.profile.personal.phone,
            "resume": B64.encode(&resume),
            "resume_filename": filename,
            "resume_content_type": resume_content_type(&filename),
            "cover_letter": ctx.cover_letter_text,
        });
        let text = post_json(&self.http, &self.post_url(ctx), body).await?;
        Ok(remote_id_or("greenhouse", &ctx.listing.external_id, &text))
    }
}

// --- Lever -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LeverSubmitter {
    base_url: String,
    http: Client,
}

impl Default for LeverSubmitter {
    fn default() -> Self {
        Self {
            base_url: LEVER_DEFAULT_BASE.to_owned(),
            http: http_client(),
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
        format!(
            "{}/v1/postings/{}/apply",
            self.base_url.trim_end_matches('/'),
            sanitize_external_id(&ctx.listing.external_id),
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

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        let (resume, filename) = read_resume(ctx).await?;
        let form = reqwest::multipart::Form::new()
            .text("name", ctx.profile.personal.name.clone())
            .text("email", ctx.profile.personal.email.clone())
            .text("phone", ctx.profile.personal.phone.clone())
            .text("org", ctx.listing.company.clone())
            .text("urls", ctx.listing.url.clone())
            .text("comments", ctx.cover_letter_text.to_owned())
            .part(
                "resume",
                reqwest::multipart::Part::bytes(resume).file_name(filename),
            );

        // Lever validates the referring posting URL on apply.
        let text = post_multipart(
            &self.http,
            &self.post_url(ctx),
            form,
            Some(&ctx.listing.url),
        )
        .await?;
        Ok(remote_id_or("lever", &ctx.listing.external_id, &text))
    }
}

// --- Ashby -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AshbySubmitter {
    base_url: String,
    http: Client,
}

impl Default for AshbySubmitter {
    fn default() -> Self {
        Self {
            base_url: ASHBY_DEFAULT_BASE.to_owned(),
            http: http_client(),
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

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        // Ashby's hosted form posts JSON to `/applicationForm.submit`. The
        // exact field names vary by form configuration, so this is a
        // best-effort candidate shape — verify against a live captured
        // submission before enabling auto_submit for Ashby.
        let (resume, filename) = read_resume(ctx).await?;
        let (first, last) = split_name(&ctx.profile.personal.name);
        let body = serde_json::json!({
            "jobPostingId": ctx.listing.external_id,
            "firstName": first,
            "lastName": last,
            "email": ctx.profile.personal.email,
            "phone": ctx.profile.personal.phone,
            "resume": B64.encode(&resume),
            "resumeFilename": filename,
            "coverLetter": ctx.cover_letter_text,
        });
        let text = post_json(&self.http, &self.post_url(), body).await?;
        Ok(remote_id_or("ashby", &ctx.listing.external_id, &text))
    }
}

// --- shared helpers --------------------------------------------------------

/// Extract the first path segment after the host, used as Greenhouse's board
/// token. Falls back to `None` for malformed / non-board URLs.
fn board_token_from_url(url: &str) -> Option<&str> {
    let after_scheme = url.split("://").nth(1)?;
    let segment = after_scheme.split('/').nth(1)?;
    if segment.is_empty() {
        None
    } else {
        Some(segment)
    }
}

fn resume_content_type(filename: &str) -> &'static str {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("pdf") => "application/pdf",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        _ => "application/octet-stream",
    }
}

fn remote_id_or(source: &str, external_id: &str, text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        format!("{source}:{external_id}")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn board_token_from_url_extracts_first_path_segment() {
        assert_eq!(
            board_token_from_url("https://boards.greenhouse.io/acme/jobs/123"),
            Some("acme")
        );
        assert_eq!(board_token_from_url("https://boards.greenhouse.io/"), None);
        assert_eq!(board_token_from_url("not a url"), None);
    }

    #[test]
    fn resume_content_type_uses_extension() {
        assert_eq!(resume_content_type("resume.pdf"), "application/pdf");
        assert_eq!(
            resume_content_type("CV.DOCX"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
        assert_eq!(
            resume_content_type("resume.txt"),
            "application/octet-stream"
        );
    }
}
