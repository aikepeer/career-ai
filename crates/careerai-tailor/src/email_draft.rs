//! Application email drafting (ported from career-ops `email` mode +
//! job_agentic's `outreach/composer.py`).
//!
//! Given a listing + profile, drafts a cold application email with a
//! subject line, body, and attachment checklist. **Draft-only** — this
//! module never sends anything; the caller prints the draft for the user
//! to review and send manually.

use serde::{Deserialize, Serialize};

use careerai_core::config::LlmConfig;
use careerai_db::models::Listing;
use careerai_llm::trait_def::Llm;
use careerai_llm::types::{LlmRequest, LlmResponse};
use careerai_profile::schema::Profile;

use crate::error::{Result, TailorError};

/// The drafted email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailDraft {
    pub subject: String,
    pub body: String,
    /// Suggested attachments (resume PDF, cover letter PDF).
    pub attachments: Vec<String>,
}

/// Build the email-draft LLM request.
pub fn email_prompt(profile: &Profile, listing: &Listing, cfg: &LlmConfig) -> LlmRequest {
    let system = "You are writing a concise, professional cold application email. No buzzwords ('excited', 'passionate', 'thrilled'). Under 150 words. Reference the specific company and role. Respond ONLY as valid JSON: {\"subject\": \"...\", \"body\": \"...\", \"attachments\": [\"resume.pdf\", \"cover_letter.pdf\"]}. Never invent experience — only use what's in the candidate profile.";

    let user = format!(
        "## Job\nCompany: {company}\nTitle: {title}\nLocation: {location}\n\n\
         ## Job Description\n{jd}\n\n\
         ## Candidate Profile\nName: {name}\nTitle: {headline}\n\
         Summary: {summary}\n\
         Key Skills: {skills}\n\n\
         Draft a cold application email. The body should be direct and professional, \
         reference one relevant achievement from the profile, and include the \
         candidate's GitHub/LinkedIn links from the profile. Respond as JSON.",
        company = listing.company,
        title = listing.title,
        location = listing.location.as_deref().unwrap_or("Not specified"),
        jd = listing.description,
        name = profile.personal.name,
        headline = profile.summary,
        summary = profile.summary,
        skills = profile
            .skills
            .all_skill_names()
            .cloned()
            .collect::<Vec<_>>()
            .join(", "),
    );

    LlmRequest {
        system: system.to_string(),
        profile_block: String::new(),
        user,
        prompt_version: format!("{}+email_draft", cfg.prompt_version),
        model: cfg.tailor_model.clone(),
        temperature: 0.4,
        max_tokens: 2048,
        cache_profile: false,
    }
}

/// Generate an email draft via the LLM.
pub async fn draft_email(
    llm: &(dyn Llm + Send + Sync),
    profile: &Profile,
    listing: &Listing,
    cfg: &LlmConfig,
) -> Result<EmailDraft> {
    let req = email_prompt(profile, listing, cfg);
    let resp: LlmResponse = llm.complete(&req).await?;
    parse_draft(&resp.text)
}

fn parse_draft(raw: &str) -> Result<EmailDraft> {
    let trimmed = raw.trim();
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };
    let draft: EmailDraft = serde_json::from_str(json_str)
        .map_err(|e| TailorError::Schema(format!("email parse: {e}")))?;
    Ok(draft)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_draft() {
        let raw = r#"```json
        {
            "subject": "Application: Senior Rust Engineer at Acme",
            "body": "Hi,\n\nI'm Test, a Rust engineer with 5 years of experience.\n\nI'd love to contribute to Acme's embedded systems work.\n\nBest,\nTest",
            "attachments": ["resume.pdf", "cover_letter.pdf"]
        }
        ```"#;
        let draft = parse_draft(raw).unwrap();
        assert!(draft.subject.contains("Rust Engineer"));
        assert!(draft.body.contains("5 years"));
        assert_eq!(draft.attachments.len(), 2);
    }

    #[test]
    fn parse_without_fences() {
        let raw = r#"{"subject": "Test", "body": "Body", "attachments": []}"#;
        let draft = parse_draft(raw).unwrap();
        assert_eq!(draft.subject, "Test");
    }

    #[test]
    fn parse_rejects_invalid() {
        assert!(parse_draft("not json").is_err());
    }
}
