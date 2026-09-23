//! LLM-backed cover-letter skeleton compilation with slot validation.

use careerai_core::config::LlmConfig;
use careerai_llm::{Llm, LlmRequest};
use careerai_profile::schema::Profile;
use serde::Deserialize;

use crate::cover_skeleton::CoverSkeleton;
use crate::error::{Result, TailorError};

const ALLOWED_SLOTS: [&str; 5] = [
    "company",
    "title",
    "top_skills",
    "keywords_summary",
    "applicant_name",
];

#[derive(Debug, Deserialize)]
struct CandidateSkeleton {
    domain: String,
    keywords: Vec<String>,
    template: String,
}

/// Generate and validate profile-independent cover-letter skeletons once.
pub async fn compile_skeletons(
    profile: &Profile,
    llm: &(dyn Llm + Send + Sync),
    cfg: &LlmConfig,
) -> Result<Vec<CoverSkeleton>> {
    let count = cfg.skeleton_count.max(1);
    let profile_json = serde_json::to_string(profile)?;
    let request = LlmRequest {
        system: "You generate recipient-neutral cover-letter templates. Output only a JSON array. Every template must use only the allowed slots: {{company}}, {{title}}, {{top_skills}}, {{keywords_summary}}, {{applicant_name}}. Never include a real person's or employer's private details.".into(),
        profile_block: profile_json,
        user: format!(
            "Generate {count} distinct cover-letter skeletons for different technical domains. Return objects with domain, keywords, and template. Keep each template at most 350 words."
        ),
        prompt_version: "compile-skeletons.v1".into(),
        model: if cfg.cover_letter_model.is_empty() {
            cfg.model.clone()
        } else {
            cfg.cover_letter_model.clone()
        },
        temperature: 0.2,
        max_tokens: 2_000,
        cache_profile: cfg.anthropic_prompt_cache,
    };
    let response = llm.complete(&request).await?;
    let raw = strip_json_fence(response.text.trim());
    let candidates: Vec<CandidateSkeleton> = serde_json::from_str(raw)?;

    let mut result = Vec::with_capacity(candidates.len().min(count));
    for candidate in candidates.into_iter().take(count) {
        validate_candidate(&candidate)?;
        result.push(CoverSkeleton {
            domain: candidate.domain.trim().to_string(),
            keywords: candidate
                .keywords
                .into_iter()
                .map(|keyword| keyword.trim().to_ascii_lowercase())
                .filter(|keyword| !keyword.is_empty())
                .collect(),
            template: candidate.template.trim().to_string(),
        });
    }
    if result.is_empty() {
        return Err(TailorError::Schema(
            "skeleton compiler returned no valid templates".into(),
        ));
    }
    Ok(result)
}

fn validate_candidate(candidate: &CandidateSkeleton) -> Result<()> {
    if candidate.domain.trim().is_empty() {
        return Err(TailorError::Schema("skeleton domain is empty".into()));
    }
    if candidate.keywords.is_empty() {
        return Err(TailorError::Schema(format!(
            "skeleton '{}' has no keywords",
            candidate.domain
        )));
    }
    if candidate.template.split_whitespace().count() > 350 {
        return Err(TailorError::CoverLetterTooLong {
            words: candidate.template.split_whitespace().count(),
            cap: 350,
        });
    }
    for slot in candidate.template.match_indices("{{") {
        let rest = &candidate.template[slot.0 + 2..];
        let Some(end) = rest.find("}}") else {
            return Err(TailorError::Schema("unterminated skeleton slot".into()));
        };
        let name = rest[..end].trim();
        if !ALLOWED_SLOTS.contains(&name) {
            return Err(TailorError::Schema(format!(
                "unsupported skeleton slot '{{{{{name}}}}}'"
            )));
        }
    }
    Ok(())
}

fn strip_json_fence(raw: &str) -> &str {
    raw.strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(raw)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use careerai_llm::{LlmResponse, Result as LlmResult};
    use careerai_profile::schema::{Personal, Profile};

    struct FixedLlm;

    #[async_trait]
    impl Llm for FixedLlm {
        fn name(&self) -> &'static str {
            "fixed"
        }

        async fn complete(&self, _request: &LlmRequest) -> LlmResult<LlmResponse> {
            Ok(LlmResponse {
                text: r#"[{"domain":"Rust","keywords":["rust"],"template":"Dear {{company}} hiring team, {{title}} needs {{top_skills}}.\n\nRegards, {{applicant_name}}"}]"#.into(),
                prompt_tokens: 1,
                completion_tokens: 1,
                cache_hit: false,
                cached_prompt_tokens: 0,
            })
        }
    }

    #[tokio::test]
    async fn compiles_only_safe_slot_templates() {
        let profile = Profile {
            personal: Personal {
                name: "Alice Engineer".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let cfg = LlmConfig::default();
        let skeletons = compile_skeletons(&profile, &FixedLlm, &cfg).await.unwrap();
        assert_eq!(skeletons.len(), 1);
        assert!(skeletons[0].template.contains("{{company}}"));
    }

    #[test]
    fn rejects_unknown_slot() {
        let error = validate_candidate(&CandidateSkeleton {
            domain: "test".into(),
            keywords: vec!["rust".into()],
            template: "{{unknown}}".into(),
        })
        .unwrap_err();
        assert!(error.to_string().contains("unsupported skeleton slot"));
    }
}
