//! LLM-backed profile extraction. Bridges `careerai_profile`'s
//! `LlmCaller` trait to `careerai_llm::Backend` via the `Adapter`
//! type below. Lives in `careerai-cli` because the CLI is the only
//! crate that depends on both `careerai-profile` and `careerai-llm`,
//! breaking what would otherwise be a circular crate edge.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;

/// Best-effort probe: is an Anthropic API key reachable without any
/// network round-trip? Checks `ANTHROPIC_API_KEY` first, then the
/// keyring (service "career-ai", username "anthropic/api_key"). Used
/// to default `--use-llm`.
fn api_key_reachable() -> bool {
    const ENV_KEY_NAMES: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "DEEPSEEK_API_KEY",
        "OPENAI_API_KEY",
        "GROK_API_KEY",
        "XAI_API_KEY",
        "OPENROUTER_API_KEY",
        "CAREERAI_LLM_API_KEY",
        "CLAUDE_WEB_API_KEY",
    ];
    for &var_name in ENV_KEY_NAMES {
        if std::env::var(var_name).is_ok_and(|v| !v.is_empty()) {
            return true;
        }
    }
    if let Ok(entry) = keyring::Entry::new("career-ai", "anthropic/api_key") {
        if let Ok(v) = entry.get_password() {
            if !v.is_empty() {
                return true;
            }
        }
    }
    false
}

pub(super) fn backend_maybe_available(
    backend_override: Option<&careerai_core::config::BackendChoice>,
) -> bool {
    if let Some(b) = backend_override {
        if *b != careerai_core::config::BackendChoice::Auto {
            return true;
        }
    }
    if std::env::var("CAREERAI_LLM_BACKEND").is_ok() {
        return true;
    }
    if which::which("claude").is_ok() || which::which("agy").is_ok() || which::which("goose").is_ok() {
        return true;
    }
    api_key_reachable()
}

/// Parse PDF/DOCX inputs through an LLM extractor; LinkedIn ZIPs go
/// through their structured CSV path unchanged. Selects the backend
/// (`claude` CLI vs rig-core Anthropic API) via
/// `careerai_llm::Backend::resolve` honoring `cfg.llm.backend` and the
/// `--llm-backend` global flag.
pub(super) fn run_with_llm(
    paths: &[&Path],
    backend_override: Option<careerai_core::config::BackendChoice>,
) -> Result<careerai_profile::Profile> {
    #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
    {
        use std::sync::Arc;

        let cwd = std::env::current_dir()?;
        // Fall back to `LlmConfig::default()` ONLY when no `config/`
        // directory is present (e.g. running `profile import` before
        // `init`). When config exists, surface load/parse failures so
        // malformed YAML and similar real errors don't silently hide
        // behind defaults.
        let mut llm_cfg = if cwd.join("config").exists() {
            CoreConfig::load(&cwd).context("load config/")?.llm
        } else {
            careerai_core::config::LlmConfig::default()
        };
        if let Some(b) = backend_override {
            llm_cfg.backend = b;
        }

        // Honor `config.llm.cache_dir` so live profile-extract caches
        // sit next to tailor caches under `data/cache/llm`. `.gitignore`
        // already excludes `/data/`.
        let cache_root: PathBuf = if llm_cfg.cache_dir.is_empty() {
            PathBuf::from("data").join("cache").join("llm")
        } else {
            PathBuf::from(&llm_cfg.cache_dir)
        };
        let cache_dir = if cache_root.is_absolute() {
            cache_root
        } else {
            cwd.join(cache_root)
        };
        let cache = Arc::new(careerai_llm::Cache::new(cache_dir));

        let mut opts = careerai_profile::ExtractOptions::default();
        // Plumb config.llm.parse_resume_model into the extractor; strip
        // any leading `provider/` prefix that the layered config uses
        // (rig's anthropic transport expects the bare model id).
        if !llm_cfg.parse_resume_model.is_empty() {
            opts.model = strip_provider_prefix(&llm_cfg.parse_resume_model).to_string();
        }
        if !llm_cfg.prompt_version.is_empty() {
            opts.prompt_version.clone_from(&llm_cfg.prompt_version);
        }

        let backend = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(careerai_llm::Backend::resolve(
                llm_cfg.backend.clone(),
                &llm_cfg,
                cache,
            ))
        })
        .map_err(|e| anyhow::anyhow!("resolve llm backend: {e}"))?;

        let adapter = adapter::Adapter::new(&backend);
        let ctx = careerai_profile::LlmExtractContext::new(&adapter, opts);
        careerai_profile::import_paths_with_llm(paths, Some(&ctx))
            .context("parsing profile sources via LLM")
    }
    #[cfg(not(any(feature = "live-llm-cli", feature = "live-llm-api")))]
    {
        let _ = (paths, backend_override);
        anyhow::bail!(
            "LLM extraction requires the `live-llm-cli` or `live-llm-api` cargo feature. \
             Re-run with: cargo run -p careerai-cli --features live-llm-cli -- profile import …"
        )
    }
}

/// Strip a leading `provider/` prefix (e.g. `anthropic/claude-haiku-4-5`
/// → `claude-haiku-4-5`). Layered config templates use the prefixed
/// form for human readability, but rig's transports want the bare model
/// id. No-op when no slash is present.
#[cfg_attr(not(any(feature = "live-llm", test)), allow(dead_code))]
fn strip_provider_prefix(model: &str) -> &str {
    model.split_once('/').map_or(model, |(_, rest)| rest)
}

/// Adapter that lets a `careerai_llm::Llm` be used as a
/// `careerai_profile::LlmCaller`.
#[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
mod adapter {
    use async_trait::async_trait;
    use careerai_llm::{Llm, LlmRequest};
    use careerai_profile::llm_extract::{ExtractRequest, LlmCaller};

    pub struct Adapter<'a> {
        inner: &'a dyn Llm,
    }

    impl<'a> Adapter<'a> {
        pub fn new(inner: &'a dyn Llm) -> Self {
            Self { inner }
        }
    }

    #[async_trait]
    impl LlmCaller for Adapter<'_> {
        async fn call(&self, req: &ExtractRequest) -> Result<String, String> {
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn strip_provider_prefix_handles_layered_config_form() {
        assert_eq!(
            strip_provider_prefix("anthropic/claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
        assert_eq!(strip_provider_prefix("openai/gpt-4o-mini"), "gpt-4o-mini");
    }

    #[test]
    fn strip_provider_prefix_passes_bare_model_id_through() {
        assert_eq!(
            strip_provider_prefix("claude-haiku-4-5"),
            "claude-haiku-4-5"
        );
        assert_eq!(strip_provider_prefix(""), "");
    }
}
