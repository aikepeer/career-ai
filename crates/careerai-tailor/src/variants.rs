//! Pre-computed bullet variant compiler (Phase 2 of LLM Reduction).
//!
//! Strategy 2: Pre-compute N reworded variants per bullet emphasizing different
//! skills (e.g. streaming, reliability, scale, leadership), validated through
//! strict entity guardrails. At runtime, the local scorer selects the best
//! matching variant in <1ms without calling an LLM.

use std::path::Path;

use careerai_core::config::LlmConfig;
use careerai_llm::hashing::canonical_profile_hash;
use careerai_llm::trait_def::Llm;
use careerai_llm::LlmRequest;
use careerai_profile::schema::Profile;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::error::Result;
use crate::guardrails::{self, ProfileTokenSets};

/// A single bullet and its pre-computed emphasis variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BulletVariant {
    pub original: String,
    pub variants: Vec<String>,
}

/// Pre-computed variants for an entire experience or project entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EntryVariants {
    pub entry_index: usize,
    pub bullets: Vec<BulletVariant>,
}

/// Full set of pre-computed variants for a profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProfileVariants {
    pub profile_hash: String,
    pub experience: Vec<EntryVariants>,
    pub projects: Vec<EntryVariants>,
}

impl ProfileVariants {
    /// Create default identity variants where each bullet has only its original text.
    #[must_use]
    pub fn from_profile_identity(profile: &Profile) -> Self {
        let profile_hash = canonical_profile_hash(profile);
        let experience = profile
            .experience
            .iter()
            .enumerate()
            .map(|(entry_index, exp)| EntryVariants {
                entry_index,
                bullets: exp
                    .bullets
                    .iter()
                    .map(|b| BulletVariant {
                        original: b.clone(),
                        variants: vec![b.clone()],
                    })
                    .collect(),
            })
            .collect();

        let projects = profile
            .projects
            .iter()
            .enumerate()
            .map(|(entry_index, proj)| EntryVariants {
                entry_index,
                bullets: proj
                    .bullets
                    .iter()
                    .map(|b| BulletVariant {
                        original: b.clone(),
                        variants: vec![b.clone()],
                    })
                    .collect(),
            })
            .collect();

        Self {
            profile_hash,
            experience,
            projects,
        }
    }

    /// Load pre-compiled variants from a JSON cache file on disk.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let variants: Self = serde_json::from_str(&text)?;
        Ok(variants)
    }

    /// Save pre-compiled variants to a JSON cache file on disk.
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Get all candidate variants for a specific experience bullet.
    #[must_use]
    pub fn experience_bullet_candidates(
        &self,
        entry_idx: usize,
        bullet_idx: usize,
        fallback_original: &str,
    ) -> Vec<String> {
        if let Some(entry) = self.experience.get(entry_idx) {
            if let Some(bv) = entry.bullets.get(bullet_idx) {
                if !bv.variants.is_empty() {
                    return bv.variants.clone();
                }
            }
        }
        vec![fallback_original.to_string()]
    }

    /// Get all candidate variants for a specific project bullet.
    #[must_use]
    pub fn project_bullet_candidates(
        &self,
        entry_idx: usize,
        bullet_idx: usize,
        fallback_original: &str,
    ) -> Vec<String> {
        if let Some(entry) = self.projects.get(entry_idx) {
            if let Some(bv) = entry.bullets.get(bullet_idx) {
                if !bv.variants.is_empty() {
                    return bv.variants.clone();
                }
            }
        }
        vec![fallback_original.to_string()]
    }
}

/// Prompt LLM to compile multiple emphasis variants per bullet, validating each through guardrails.
pub async fn compile_profile_variants(
    profile: &Profile,
    llm: &(dyn Llm + Send + Sync),
    cfg: &LlmConfig,
) -> Result<ProfileVariants> {
    let profile_hash = canonical_profile_hash(profile);
    let token_sets = guardrails::build_token_sets(profile);

    info!(
        target: "tailor::variants",
        profile_hash = %profile_hash,
        "compiling bullet variants via LLM"
    );

    let mut experience_entries = Vec::new();
    for (entry_index, exp) in profile.experience.iter().enumerate() {
        let mut bullet_variants = Vec::new();
        for (bullet_index, original_bullet) in exp.bullets.iter().enumerate() {
            let path_label = format!("experience[{entry_index}].bullets[{bullet_index}]");
            let variants =
                compile_single_bullet_variants(original_bullet, &path_label, &token_sets, llm, cfg)
                    .await?;
            bullet_variants.push(BulletVariant {
                original: original_bullet.clone(),
                variants,
            });
        }
        experience_entries.push(EntryVariants {
            entry_index,
            bullets: bullet_variants,
        });
    }

    let mut project_entries = Vec::new();
    for (entry_index, proj) in profile.projects.iter().enumerate() {
        let mut bullet_variants = Vec::new();
        for (bullet_index, original_bullet) in proj.bullets.iter().enumerate() {
            let path_label = format!("projects[{entry_index}].bullets[{bullet_index}]");
            let variants =
                compile_single_bullet_variants(original_bullet, &path_label, &token_sets, llm, cfg)
                    .await?;
            bullet_variants.push(BulletVariant {
                original: original_bullet.clone(),
                variants,
            });
        }
        project_entries.push(EntryVariants {
            entry_index,
            bullets: bullet_variants,
        });
    }

    Ok(ProfileVariants {
        profile_hash,
        experience: experience_entries,
        projects: project_entries,
    })
}

async fn compile_single_bullet_variants(
    original: &str,
    path_label: &str,
    token_sets: &ProfileTokenSets,
    llm: &(dyn Llm + Send + Sync),
    cfg: &LlmConfig,
) -> Result<Vec<String>> {
    let prompt = format!(
        "You are an expert technical resume editor. Given this resume bullet:\n\n\
        \"{original}\"\n\n\
        Generate 3 distinct reworded variants emphasizing different angles (e.g. scale, reliability, technical depth).\n\
        CRITICAL RULES:\n\
        1. Keep all numbers, metrics, proper nouns, and company names byte-identical or truthful to the original.\n\
        2. Maximum length 280 characters per variant.\n\
        3. Output ONLY a valid JSON array of 3 strings. Example: [\"Variant 1\", \"Variant 2\", \"Variant 3\"]"
    );

    let req = LlmRequest {
        system:
            "You are an expert technical resume editor. Output only valid JSON arrays of strings."
                .into(),
        profile_block: String::new(),
        user: prompt,
        prompt_version: "variants.v1".into(),
        model: if cfg.tailor_model.is_empty() {
            cfg.model.clone()
        } else {
            cfg.tailor_model.clone()
        },
        temperature: 0.2,
        max_tokens: 500,
        cache_profile: false,
    };

    let mut accepted = vec![original.to_string()];

    match llm.complete(&req).await {
        Ok(resp) => {
            if let Ok(parsed) = serde_json::from_str::<Vec<String>>(resp.text.trim()) {
                for candidate in parsed {
                    let trimmed = candidate.trim();
                    if trimmed.is_empty() || trimmed.chars().count() > 280 {
                        continue;
                    }
                    if guardrails::forbid_invented_entities_with(
                        trimmed, original, token_sets, path_label,
                    )
                    .is_ok()
                    {
                        if !accepted.contains(&trimmed.to_string()) {
                            accepted.push(trimmed.to_string());
                        }
                    } else {
                        warn!(
                            target = "tailor::variants",
                            path = path_label,
                            candidate = trimmed,
                            "variant dropped: failed entity guardrails"
                        );
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                target = "tailor::variants",
                error = %e,
                path = path_label,
                "variant LLM call failed, falling back to original bullet"
            );
        }
    }

    Ok(accepted)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::{Experience, Personal, Project};

    fn sample_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "Alice Engineer".into(),
                email: "alice@example.com".into(),
                ..Default::default()
            },
            experience: vec![Experience {
                title: "Rust Engineer".into(),
                company: "Acme Corp".into(),
                bullets: vec!["Built Tokio backend handling 10M req/sec.".into()],
                ..Default::default()
            }],
            projects: vec![Project {
                name: "Embedded OS".into(),
                bullets: vec!["Bootstrapped ARM kernel in Rust.".into()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn identity_variants_have_original_bullet() {
        let p = sample_profile();
        let variants = ProfileVariants::from_profile_identity(&p);
        assert_eq!(variants.experience.len(), 1);
        let candidates = variants.experience_bullet_candidates(0, 0, "");
        assert_eq!(
            candidates,
            vec!["Built Tokio backend handling 10M req/sec."]
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let p = sample_profile();
        let mut variants = ProfileVariants::from_profile_identity(&p);
        variants.experience[0].bullets[0]
            .variants
            .push("Optimized Tokio network IO at 10M req/sec.".into());

        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("variants.json");
        variants.save_to_file(&path).unwrap();

        let loaded = ProfileVariants::load_from_file(&path).unwrap();
        assert_eq!(loaded.experience[0].bullets[0].variants.len(), 2);
    }
}
