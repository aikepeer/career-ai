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
use careerai_profile::schema::Profile;
use serde::{Deserialize, Serialize};

use crate::error::Result;

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

/// Compile all profile bullets in one cached LLM request, validating each
/// candidate through the existing entity guardrails.
pub async fn compile_profile_variants(
    profile: &Profile,
    llm: &(dyn Llm + Send + Sync),
    cfg: &LlmConfig,
) -> Result<ProfileVariants> {
    crate::variant_compiler::compile_profile_variants_once(profile, llm, cfg).await
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
