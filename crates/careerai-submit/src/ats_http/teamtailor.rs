//! Teamtailor candidate application submitter.
//!
//! Teamtailor's hosted careers site accepts a standard browser form POST at
//! `{listing_url}/application`. There is no candidate-facing API key — the
//! submitter posts multipart/form-data to that public endpoint. Field names
//! are a best-effort Rails-style guess; verify against a live captured
//! submission before enabling `auto_submit`.

use async_trait::async_trait;
use reqwest::Client;

use crate::base::{SubmitContext, Submitter, WouldSubmit};
use crate::error::Result;

use super::support::{
    artifact_kinds_owned, build_body_preview, http_client, post_multipart, read_resume,
    CandidatePayload,
};

#[derive(Debug, Clone)]
pub struct TeamtailorSubmitter {
    http: Client,
}

impl Default for TeamtailorSubmitter {
    fn default() -> Self {
        Self {
            http: http_client(),
        }
    }
}

impl TeamtailorSubmitter {
    pub fn new() -> Self {
        Self::default()
    }

    fn apply_url(ctx: &SubmitContext<'_>) -> String {
        Self::apply_url_for(&ctx.listing.url)
    }

    fn apply_url_for(listing_url: &str) -> String {
        // Listing URLs look like https://{company}.teamtailor.com/jobs/{id}.
        format!("{}/application", listing_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl Submitter for TeamtailorSubmitter {
    fn name(&self) -> &'static str {
        "teamtailor"
    }

    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        let payload = CandidatePayload::from_ctx(ctx);
        Ok(WouldSubmit {
            source: "teamtailor",
            url: Self::apply_url(ctx),
            method: "POST",
            body_preview: build_body_preview(&payload)?,
            artifact_kinds: artifact_kinds_owned(ctx),
        })
    }

    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        let (resume, filename) = read_resume(ctx).await?;
        let form = reqwest::multipart::Form::new()
            .text("candidate[name]", ctx.profile.personal.name.clone())
            .text("candidate[email]", ctx.profile.personal.email.clone())
            .text("candidate[phone]", ctx.profile.personal.phone.clone())
            .text("candidate[message]", ctx.cover_letter_text.to_owned())
            .part(
                "candidate[resume]",
                reqwest::multipart::Part::bytes(resume).file_name(filename),
            );

        let text = post_multipart(
            &self.http,
            &Self::apply_url(ctx),
            form,
            Some(&ctx.listing.url),
        )
        .await?;
        let trimmed = text.trim();
        Ok(if trimmed.is_empty() {
            format!("teamtailor:{}", ctx.listing.external_id)
        } else {
            trimmed.to_owned()
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::TeamtailorSubmitter;

    #[test]
    fn apply_url_appends_application() {
        assert_eq!(
            TeamtailorSubmitter::apply_url_for(
                "https://synmatchai.teamtailor.com/jobs/8148987-full-stack-engineer"
            ),
            "https://synmatchai.teamtailor.com/jobs/8148987-full-stack-engineer/application"
        );
    }

    #[test]
    fn apply_url_trims_trailing_slash() {
        assert_eq!(
            TeamtailorSubmitter::apply_url_for("https://synmatchai.teamtailor.com/jobs/1/"),
            "https://synmatchai.teamtailor.com/jobs/1/application"
        );
    }
}
