//! Drafter-reviewer pass (ported from ai-job-search's `/apply` workflow).
//!
//! After the primary tailor step produces a `TailorOutcome`, a second LLM
//! call — the *reviewer* — critiques the tailored resume + cover letter
//! against the JD and returns structured revision suggestions. The caller
//! can then feed those back into the tailor for a revision pass.
//!
//! Safety invariant preserved: the reviewer's suggestions are *advisory*
//! only. They are constrained to reword/reorder existing bullets — never
//! to invent new experience. The guardrails in `guardrails.rs` still run
//! on any revision applied.

use serde::{Deserialize, Serialize};

use careerai_core::config::LlmConfig;
use careerai_db::models::Listing;
use careerai_llm::hashing::{canonical_profile_hash, jd_hash};
use careerai_llm::trait_def::Llm;
use careerai_llm::types::LlmRequest;
use careerai_profile::schema::Profile;

use crate::error::{Result, TailorError};
use crate::model::{CoverLetter, ResumeView, TailorOutcome};

/// One per-bullet suggestion from the reviewer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulletSuggestion {
    /// JSON path into the resume view, e.g. `experience[0].bullets[2]`.
    pub path: String,
    /// What the reviewer thinks is weak about the current bullet.
    pub critique: String,
    /// The suggested reword. Must pass guardrails before applying.
    #[serde(default)]
    pub suggested_text: Option<String>,
}

/// The full critique returned by the reviewer LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCritique {
    /// Overall fit assessment (1-5).
    pub fit_score: u8,
    /// High-level feedback on the tailored resume.
    pub summary_feedback: String,
    /// Per-bullet suggestions, keyed by JSON path.
    #[serde(default)]
    pub bullet_suggestions: Vec<BulletSuggestion>,
    /// Cover-letter feedback.
    #[serde(default)]
    pub cover_letter_feedback: Option<String>,
    /// Keywords from the JD the reviewer thinks are missing or under-emphasised.
    #[serde(default)]
    pub missing_keywords: Vec<String>,
}

impl ReviewCritique {
    /// Parse the reviewer LLM's text response as JSON. Trims markdown
    /// code fences if present.
    fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        let json_str = if trimmed.starts_with("```") {
            let inner = trimmed
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            inner
        } else {
            trimmed
        };
        serde_json::from_str(json_str)
            .map_err(|e| TailorError::Schema(format!("review parse: {e}")))
    }
}

/// Build the reviewer `LlmRequest`. The reviewer receives the tailored
/// resume + cover letter + JD and is asked to critique, not to rewrite.
pub fn review_prompt(
    profile: &Profile,
    listing: &Listing,
    resume: &ResumeView,
    cover_letter: &CoverLetter,
    cfg: &LlmConfig,
) -> Result<LlmRequest> {
    let resume_json = serde_json::to_string_pretty(resume)
        .map_err(|e| TailorError::Schema(format!("serialize resume: {e}")))?;
    let cover_text = &cover_letter.body;

    let system = "You are a senior technical recruiter reviewing a tailored resume and cover letter for a specific job. Be critical but constructive. Respond ONLY as valid JSON matching the ReviewCritique schema. Never suggest inventing new experience — only reword or reorder existing bullets.";

    let user = format!(
        "## Job Description\nCompany: {company}\nTitle: {title}\n\n{jd}\n\n\
         ## Tailored Resume (JSON)\n{resume_json}\n\n\
         ## Cover Letter\n{cover_text}\n\n\
         ## Candidate Profile (for context — do NOT suggest inventing anything not here)\n\
         Name: {name}\n\
         Summary: {summary}\n\n\
         Review the resume and cover letter against this JD. Respond as JSON:\n\
         {{\n  \
         \"fit_score\": <1-5>,\n  \
         \"summary_feedback\": \"...\",\n  \
         \"bullet_suggestions\": [{{\"path\": \"experience[0].bullets[1]\", \"critique\": \"...\", \"suggested_text\": \"optional reword\"}}],\n  \
         \"cover_letter_feedback\": \"optional\",\n  \
         \"missing_keywords\": [\"...\"]\n\
         }}",
        company = listing.company,
        title = listing.title,
        jd = listing.description,
        resume_json = resume_json,
        cover_text = cover_text,
        name = profile.personal.name,
        summary = profile.summary,
    );

    Ok(LlmRequest {
        system: system.to_string(),
        profile_block: String::new(),
        user,
        prompt_version: format!("{}+reviewer", cfg.prompt_version),
        model: cfg.tailor_model.clone(),
        temperature: 0.2,
        max_tokens: 8192,
        cache_profile: false,
    })
}

/// Run the reviewer pass on an existing `TailorOutcome`. Returns the
/// critique; the caller decides whether to apply suggestions.
pub async fn review_tailored(
    llm: &(dyn Llm + Send + Sync),
    profile: &Profile,
    listing: &Listing,
    outcome: &TailorOutcome,
    cfg: &LlmConfig,
) -> Result<ReviewCritique> {
    let req = review_prompt(
        profile,
        listing,
        &outcome.resume_view,
        &outcome.cover_letter,
        cfg,
    )?;
    let resp = llm.complete(&req).await?;
    ReviewCritique::parse(&resp.text)
}

/// Cache key for the reviewer pass — distinct from the tailor key so
/// they don't collide.
#[must_use]
pub fn review_cache_key(profile: &Profile, listing: &Listing, cfg: &LlmConfig) -> String {
    let p_hash = canonical_profile_hash(profile);
    let j_hash = jd_hash(&listing.title, &listing.company, &listing.description);
    format!(
        "{}+reviewer+{}+{}+{}",
        cfg.prompt_version, p_hash, j_hash, cfg.tailor_model,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use careerai_llm::trait_def::Llm;
    use careerai_llm::types::{LlmRequest, LlmResponse};
    use careerai_profile::schema::{Personal, Profile};

    struct MockReviewLlm {
        response: String,
    }

    #[async_trait]
    impl Llm for MockReviewLlm {
        fn name(&self) -> &'static str {
            "mock-review"
        }
        async fn complete(&self, _req: &LlmRequest) -> careerai_llm::error::Result<LlmResponse> {
            Ok(LlmResponse {
                text: self.response.clone(),
                prompt_tokens: 100,
                completion_tokens: 200,
                cache_hit: false,
                cached_prompt_tokens: 0,
            })
        }
    }

    fn mock_listing() -> careerai_db::models::Listing {
        careerai_db::models::Listing {
            id: "lst-1".into(),
            source: "greenhouse".into(),
            external_id: "gh-1".into(),
            title: "Senior Rust Engineer".into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: "https://acme.com/job".into(),
            description: "We need a Rust engineer with embedded experience.".into(),
            raw_json: None,
            state: "shortlisted".into(),
            score: Some(0.05),
            created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2026-01-01T00:00:00Z".parse().unwrap(),
        }
    }

    fn mock_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "Test Person".into(),
                email: "test@example.com".into(),
                phone: "+1234567890".into(),
                location: "Earth".into(),
                links: careerai_profile::schema::Links::default(),
            },
            summary: "Rust engineer with 5 years experience".into(),
            ..Default::default()
        }
    }

    fn mock_outcome() -> TailorOutcome {
        TailorOutcome {
            application_id: "app-1".into(),
            resume_view: ResumeView {
                personal: Personal {
                    name: "Test".into(),
                    email: "t@t.com".into(),
                    phone: "+1".into(),
                    location: "Earth".into(),
                    links: careerai_profile::schema::Links::default(),
                },
                summary: "Rust engineer".into(),
                skills: careerai_profile::schema::Skills::default(),
                experience: vec![],
                education: vec![],
                projects: vec![],
            },
            cover_letter: CoverLetter {
                body: "Dear Acme, I am a Rust engineer.".into(),
            },
            diff_raw_json: "{}".into(),
            quality_score: None,
            tone_label: None,
        }
    }

    fn mock_cfg() -> LlmConfig {
        LlmConfig {
            provider: String::new(),
            model: String::new(),
            tailor_model: "test-model".into(),
            cover_letter_model: "test-model".into(),
            filter_model: String::new(),
            parse_resume_model: String::new(),
            cache_dir: String::new(),
            api_base_url: None,
            api_key: None,
            max_retries: 1,
            timeout_seconds: 300,
            prompt_version: "v1".into(),
            anthropic_prompt_cache: false,
            backend: careerai_core::config::BackendChoice::ClaudeCli,
            strategy: "hybrid".into(),
            drop_threshold: 0.01,
            llm_min_score: 0.03,
            effort: "low".into(),
            max_daily_cost_usd: None,
            max_daily_calls: None,
        }
    }

    #[test]
    fn parse_valid_critique() {
        let raw = r#"```json
        {
            "fit_score": 4,
            "summary_feedback": "Strong Rust fit, weak embedded signal.",
            "bullet_suggestions": [
                {"path": "experience[0].bullets[1]", "critique": "No metric", "suggested_text": "Built embedded Rust firmware for 10M devices"}
            ],
            "cover_letter_feedback": "Good tone, add a specific project.",
            "missing_keywords": ["embedded", "RTOS"]
        }
        ```"#;
        let critique = ReviewCritique::parse(raw).unwrap();
        assert_eq!(critique.fit_score, 4);
        assert_eq!(critique.bullet_suggestions.len(), 1);
        assert_eq!(critique.missing_keywords, vec!["embedded", "RTOS"]);
        assert!(critique.cover_letter_feedback.is_some());
    }

    #[test]
    fn parse_without_code_fences() {
        let raw = r#"{"fit_score": 3, "summary_feedback": "ok", "bullet_suggestions": []}"#;
        let critique = ReviewCritique::parse(raw).unwrap();
        assert_eq!(critique.fit_score, 3);
        assert!(critique.bullet_suggestions.is_empty());
    }

    #[test]
    fn parse_rejects_invalid_json() {
        let raw = "not json at all";
        assert!(ReviewCritique::parse(raw).is_err());
    }

    #[tokio::test]
    async fn review_tailored_returns_critique() {
        let llm = MockReviewLlm {
            response: r#"{"fit_score": 5, "summary_feedback": "Great", "bullet_suggestions": [], "missing_keywords": []}"#.into(),
        };
        let listing = mock_listing();
        let profile = mock_profile();
        let outcome = mock_outcome();
        let cfg = mock_cfg();

        let critique = review_tailored(&llm, &profile, &listing, &outcome, &cfg)
            .await
            .unwrap();
        assert_eq!(critique.fit_score, 5);
        assert_eq!(critique.summary_feedback, "Great");
    }

    #[test]
    fn review_cache_key_is_distinct_from_tailor() {
        let listing = mock_listing();
        let profile = mock_profile();
        let cfg = mock_cfg();
        let key = review_cache_key(&profile, &listing, &cfg);
        assert!(key.contains("reviewer"));
        assert!(!key.contains("+cover"));
    }
}
