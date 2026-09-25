//! One-call profile-wide bullet variant compilation.

use careerai_core::config::LlmConfig;
use careerai_llm::trait_def::Llm;
use careerai_llm::LlmRequest;
use careerai_profile::schema::Profile;
use serde::Deserialize;
use tracing::warn;

use crate::error::Result;
use crate::guardrails;
use crate::variants::{BulletVariant, EntryVariants, ProfileVariants};

#[derive(Debug, Deserialize)]
struct CompiledBullet {
    path: String,
    variants: Vec<String>,
}

/// Compile every profile bullet in one provider request.
pub async fn compile_profile_variants_once(
    profile: &Profile,
    llm: &(dyn Llm + Send + Sync),
    cfg: &LlmConfig,
) -> Result<ProfileVariants> {
    let identity = ProfileVariants::from_profile_identity(profile);
    let mut bullets = Vec::new();
    for (entry_index, entry) in profile.experience.iter().enumerate() {
        for (bullet_index, original) in entry.bullets.iter().enumerate() {
            bullets.push(serde_json::json!({
                "path": format!("experience[{entry_index}].bullets[{bullet_index}]"),
                "original": original,
            }));
        }
    }
    for (entry_index, entry) in profile.projects.iter().enumerate() {
        for (bullet_index, original) in entry.bullets.iter().enumerate() {
            bullets.push(serde_json::json!({
                "path": format!("projects[{entry_index}].bullets[{bullet_index}]"),
                "original": original,
            }));
        }
    }
    if bullets.is_empty() {
        return Ok(identity);
    }

    let variant_count = cfg.variant_count.max(1);
    let request = LlmRequest {
        system: "You are a technical resume editor. Return only a JSON array of objects with path and variants. Preserve every number, proper noun, employer, date, and factual claim from each original bullet. Never invent claims.".into(),
        profile_block: serde_json::to_string(profile)?,
        user: format!(
            "For each bullet in this JSON array, generate up to {variant_count} concise variants emphasizing different technical angles. Maximum 280 characters per variant.\n{}",
            serde_json::to_string(&bullets)?
        ),
        prompt_version: "variants.v2".into(),
        model: if cfg.tailor_model.is_empty() {
            cfg.model.clone()
        } else {
            cfg.tailor_model.clone()
        },
        temperature: 0.2,
        max_tokens: 10_000,
        cache_profile: cfg.anthropic_prompt_cache,
    };
    let response = match llm.complete(&request).await {
        Ok(response) => response,
        Err(error) => {
            warn!(target = "tailor::variants", error = %error, "profile variant compilation failed; using identity variants");
            return Ok(identity);
        }
    };
    let parsed = match parse_response(response.text.trim()) {
        Ok(parsed) => parsed,
        Err(error) => {
            warn!(target = "tailor::variants", error = %error, "profile variant response was invalid; using identity variants");
            return Ok(identity);
        }
    };
    let token_sets = guardrails::build_token_sets(profile);
    let mut compiled = identity;
    for item in parsed {
        let Some((kind, entry_index, bullet_index)) = parse_path(&item.path) else {
            continue;
        };
        let Some(bullet) = bullet_at_mut(&mut compiled, kind, entry_index, bullet_index) else {
            continue;
        };
        let original = bullet.original.clone();
        for candidate in item.variants.into_iter().take(variant_count) {
            let candidate = candidate.trim();
            if candidate.is_empty() || candidate.chars().count() > 280 {
                continue;
            }
            if guardrails::forbid_invented_entities_with(
                candidate,
                &original,
                &token_sets,
                &item.path,
            )
            .is_ok()
                && !bullet.variants.iter().any(|existing| existing == candidate)
            {
                bullet.variants.push(candidate.to_string());
            }
        }
    }
    Ok(compiled)
}

fn parse_response(raw: &str) -> Result<Vec<CompiledBullet>> {
    let json = raw
        .strip_prefix("```json")
        .and_then(|text| text.strip_suffix("```"))
        .map_or(raw, str::trim);
    Ok(serde_json::from_str(json)?)
}

fn parse_path(path: &str) -> Option<(&str, usize, usize)> {
    let (kind, rest) = if let Some(rest) = path.strip_prefix("experience[") {
        ("experience", rest)
    } else {
        let rest = path.strip_prefix("projects[")?;
        ("projects", rest)
    };
    let (entry, rest) = rest
        .split_once("] .bullets[")
        .or_else(|| rest.split_once("].bullets["))?;
    let (bullet, end) = rest.split_once(']')?;
    if !end.is_empty() {
        return None;
    }
    Some((kind, entry.parse().ok()?, bullet.parse().ok()?))
}

fn bullet_at_mut<'a>(
    variants: &'a mut ProfileVariants,
    kind: &str,
    entry_index: usize,
    bullet_index: usize,
) -> Option<&'a mut BulletVariant> {
    let entries: &mut Vec<EntryVariants> = if kind == "experience" {
        &mut variants.experience
    } else {
        &mut variants.projects
    };
    entries
        .get_mut(entry_index)
        .and_then(|entry| entry.bullets.get_mut(bullet_index))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use careerai_core::config::LlmConfig;
    use careerai_llm::{Llm, LlmRequest, LlmResponse};
    use careerai_profile::schema::{Experience, Personal, Profile};

    struct FixedLlm;

    #[async_trait]
    impl Llm for FixedLlm {
        fn name(&self) -> &'static str {
            "fixed"
        }

        async fn complete(&self, _request: &LlmRequest) -> careerai_llm::Result<LlmResponse> {
            Ok(LlmResponse {
                text: r#"[{"path":"experience[0].bullets[0]","variants":["Built reliable Rust services handling 10M req/sec"]}]"#.into(),
                prompt_tokens: 1,
                completion_tokens: 1,
                cache_hit: false,
                cached_prompt_tokens: 0,
            })
        }
    }

    #[tokio::test]
    async fn compiles_all_profile_bullets_in_one_request() {
        let profile = Profile {
            personal: Personal {
                name: "Alice".into(),
                ..Default::default()
            },
            experience: vec![Experience {
                bullets: vec!["Built Rust services handling 10M req/sec".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let variants = compile_profile_variants_once(&profile, &FixedLlm, &LlmConfig::default())
            .await
            .unwrap();
        assert_eq!(variants.experience[0].bullets[0].variants.len(), 2);
    }
}
