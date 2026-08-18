//! LLM-assisted profile extraction for the dashboard import path.
//!
//! The heuristic PDF/DOCX parser produces acceptable seeds but frequently
//! mis-files values (emails in the name field, mixed skills, etc.). This
//! module drives the same `careerai-profile` constrained LLM extractor the
//! CLI uses, so resume text is routed through the configured LLM backend and
//! the structured result lands in the correct schema fields. When no backend
//! is reachable, or extraction fails, the caller falls back to the heuristic
//! parser so the import button never becomes a hard failure.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use careerai_core::config::CoreConfig;
use careerai_llm::{Backend, Cache, Llm, LlmRequest};
use careerai_profile::llm_extract::{ExtractOptions, ExtractRequest, LlmCaller};
use careerai_profile::{LlmExtractContext, Profile, ProfileError, Result};

/// Adapter that lets a `careerai_llm::Llm` be used as a
/// `careerai_profile::LlmCaller`.
struct Adapter<'a> {
    inner: &'a dyn Llm,
}

impl<'a> Adapter<'a> {
    fn new(inner: &'a dyn Llm) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl LlmCaller for Adapter<'_> {
    async fn call(&self, req: &ExtractRequest) -> std::result::Result<String, String> {
        let llm_req = LlmRequest {
            system: req.system.clone(),
            profile_block: req.profile_block.clone(),
            user: req.user.clone(),
            prompt_version: req.prompt_version.clone(),
            model: req.model.clone(),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            cache_profile: req.cache_schema,
        };
        self.inner
            .complete(&llm_req)
            .await
            .map(|r| r.text)
            .map_err(|e| e.to_string())
    }
}

/// Import `paths` preferring the configured LLM backend, falling back to the
/// heuristic parser on any failure. LinkedIn ZIPs always use their structured
/// CSV path regardless.
pub async fn import_paths_best_effort(paths: &[&Path]) -> Result<Profile> {
    match try_llm_import(paths).await {
        Ok(profile) => return Ok(profile),
        Err(e) => {
            tracing::warn!(
                target = "dashboard.profile_import",
                error = %e,
                "LLM profile extraction failed; falling back to heuristic parser",
            );
        }
    }
    careerai_profile::import_paths(paths)
}

async fn try_llm_import(paths: &[&Path]) -> Result<Profile> {
    let cwd = std::env::current_dir().map_err(ProfileError::Io)?;
    let llm_cfg = if cwd.join("config").exists() {
        CoreConfig::load(&cwd)
            .map_err(|e| ProfileError::Validation(format!("load config: {e}")))?
            .llm
    } else {
        careerai_core::config::LlmConfig::default()
    };

    let cache_root = if llm_cfg.cache_dir.is_empty() {
        PathBuf::from("data").join("cache").join("llm")
    } else {
        PathBuf::from(&llm_cfg.cache_dir)
    };
    let cache_dir = if cache_root.is_absolute() {
        cache_root
    } else {
        cwd.join(cache_root)
    };
    let cache = Arc::new(Cache::new(cache_dir));

    let backend = Backend::resolve(llm_cfg.backend.clone(), &llm_cfg, cache)
        .await
        .map_err(|e| ProfileError::Validation(format!("resolve llm backend: {e}")))?;

    let mut opts = ExtractOptions::default();
    if !llm_cfg.parse_resume_model.is_empty() {
        opts.model = strip_provider_prefix(&llm_cfg.parse_resume_model).to_string();
    }
    if !llm_cfg.prompt_version.is_empty() {
        opts.prompt_version.clone_from(&llm_cfg.prompt_version);
    }

    let adapter = Adapter::new(&backend);
    let ctx = LlmExtractContext::new(&adapter, opts);
    careerai_profile::import_paths_with_llm(paths, Some(&ctx))
}

/// Strip a leading `provider/` prefix (e.g. `anthropic/claude-haiku-4-5` →
/// `claude-haiku-4-5`). Rig transports want the bare model id.
fn strip_provider_prefix(model: &str) -> &str {
    model.split_once('/').map_or(model, |(_, rest)| rest)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::strip_provider_prefix;

    #[test]
    fn strips_provider_prefix() {
        assert_eq!(
            strip_provider_prefix("anthropic/claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
        assert_eq!(strip_provider_prefix("openai/gpt-4o-mini"), "gpt-4o-mini");
        assert_eq!(
            strip_provider_prefix("claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
        assert_eq!(strip_provider_prefix(""), "");
    }
}
