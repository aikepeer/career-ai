//! SmartRecruiters candidate application submitter.
//!
//! SmartRecruiters' hosted apply form POSTs JSON to the public
//! `/v1/companies/{company}/postings/{postingId}` endpoint. There is no
//! candidate-facing API key. The exact request field names vary by the
//! employer's form configuration, so this is a best-effort candidate shape —
//! verify against a live captured submission before enabling `auto_submit`.

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use reqwest::Client;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::error::Result;

use super::support::{
    artifact_kinds_owned, build_body_preview, http_client, post_json, read_resume,
    sanitize_external_id, slugify, split_name, CandidatePayload,
};

const SMART_RECRUITERS_DEFAULT_BASE: &str = "https://api.smartrecruiters.com";

#[derive(Debug, Clone)]
pub struct SmartRecruitersSubmitter {
    base_url: String,
    http: Client,
}

impl Default for SmartRecruitersSubmitter {
    fn default() -> Self {
        Self {
            base_url: SMART_RECRUITERS_DEFAULT_BASE.to_owned(),
            http: http_client(),
        }
    }
}

impl SmartRecruitersSubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn company_slug<'a>(ctx: &'a SubmitContext<'_>) -> std::borrow::Cow<'a, str> {
        first_path_segment(&ctx.listing.url).map_or_else(
            || std::borrow::Cow::Owned(slugify(&ctx.listing.company)),
            std::borrow::Cow::Borrowed,
        )
    }

    fn post_url(&self, ctx: &SubmitContext<'_>) -> String {
        format!(
            "{}/v1/companies/{}/postings/{}",
            self.base_url.trim_end_matches('/'),
            Self::company_slug(ctx),
            sanitize_external_id(&ctx.listing.external_id),
        )
    }
}

#[async_trait]
impl Submitter for SmartRecruitersSubmitter {
    fn name(&self) -> &'static str {
        "smartrecruiters"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        Ok(WouldSubmit {
            source: "smartrecruiters",
            url: self.post_url(ctx),
            method: "POST",
            body_preview: build_body_preview(&payload)?,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        let (resume, filename) = read_resume(ctx).await?;
        let (first, last) = split_name(&ctx.profile.personal.name);
        let body = serde_json::json!({
            "firstName": first,
            "lastName": last,
            "email": ctx.profile.personal.email,
            "phoneNumber": ctx.profile.personal.phone,
            "resume": B64.encode(&resume),
            "resumeFileName": filename,
            "coverLetter": ctx.cover_letter_text,
        });
        let text = post_json(&self.http, &self.post_url(ctx), body).await?;
        let trimmed = text.trim();
        Ok(if trimmed.is_empty() {
            format!("smartrecruiters:{}", ctx.listing.external_id)
        } else {
            trimmed.to_owned()
        })
    }
}

fn first_path_segment(url: &str) -> Option<&str> {
    let after_scheme = url.split("://").nth(1)?;
    let segment = after_scheme.split('/').nth(1)?;
    if segment.is_empty() {
        None
    } else {
        Some(segment)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::first_path_segment;

    #[test]
    fn first_path_segment_extracts_company() {
        assert_eq!(
            first_path_segment("https://jobs.smartrecruiters.com/Acme/12345"),
            Some("Acme")
        );
        assert_eq!(
            first_path_segment("https://jobs.smartrecruiters.com/"),
            None
        );
        assert_eq!(first_path_segment("not a url"), None);
    }
}
