//! Shared LLM backend builder for CLI subcommands that need a live LLM
//! but don't go through the full tailor pipeline stage.
//!
//! Mirrors the backend resolution in `tailor.rs` but extracted so
//! interview-prep, email-draft, and reviewer passes can share one path.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_llm::trait_def::Llm;

/// Resolve a live LLM backend, honouring `CAREERAI_LLM_LIVE=1` and the
/// configured `cfg.llm.backend`. Returns `None` if not in live mode.
pub async fn build_live_llm(
    root: &Path,
    cfg: &CoreConfig,
) -> Result<Option<Arc<dyn Llm + Send + Sync>>> {
    if std::env::var("CAREERAI_LLM_LIVE").as_deref() != Ok("1") {
        return Ok(None);
    }

    #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
    {
        let cache_root = if cfg.llm.cache_dir.is_empty() {
            careerai_core::paths::cache_dir_for_root(root).join("llm")
        } else {
            let p = std::path::PathBuf::from(&cfg.llm.cache_dir);
            if p.is_absolute() {
                p
            } else {
                root.join(p)
            }
        };
        let cache = Arc::new(careerai_llm::Cache::new(cache_root));
        let backend = careerai_llm::Backend::resolve(cfg.llm.backend.clone(), &cfg.llm, cache)
            .await
            .map_err(|e| anyhow::anyhow!("resolve live LLM backend: {e}"))?;
        Ok(Some(Arc::new(backend)))
    }

    #[cfg(not(any(feature = "live-llm-cli", feature = "live-llm-api")))]
    {
        let _ = root;
        anyhow::bail!("CAREERAI_LLM_LIVE=1 set but no live-llm feature compiled in");
    }
}

/// Resolve a fixtures-based `MockLlm` for offline/test use.
pub fn build_fixture_llm(root: &Path) -> Result<Arc<dyn Llm + Send + Sync>> {
    let fixtures = careerai_core::paths::cache_dir_for_root(root)
        .join("llm")
        .join("fixtures");
    if !fixtures.is_dir() {
        anyhow::bail!(
            "no LLM fixtures at {}; set CAREERAI_LLM_LIVE=1 or provide fixtures",
            fixtures.display()
        );
    }
    let mock =
        careerai_llm::mock::MockLlm::from_dir(&fixtures).context("load mock LLM fixtures")?;
    Ok(Arc::new(mock))
}

/// Resolve an LLM: live if `CAREERAI_LLM_LIVE=1`, else fixtures.
pub async fn build_llm(root: &Path, cfg: &CoreConfig) -> Result<Arc<dyn Llm + Send + Sync>> {
    if let Some(live) = build_live_llm(root, cfg).await? {
        return Ok(live);
    }
    build_fixture_llm(root)
}
