//! Tailor stage: turn a shortlisted listing into a tailored application.
//! Extracted from `lib.rs` to keep that file under the project's 300-LOC cap.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::info;

use careerai_core::config::CoreConfig;
use careerai_db::queries;
use careerai_llm::mock::MockLlm;
use careerai_tailor::tailor_for_listing;

use crate::{load_profile, open_pool, TailoredOutcome};

/// Resolve the LLM fixtures directory. Honors the `CAREERAI_LLM_FIXTURES_DIR`
/// environment override, else falls back to `<root>/data/cache/llm/fixtures`.
fn fixtures_dir(root: &Path) -> PathBuf {
    std::env::var("CAREERAI_LLM_FIXTURES_DIR").map_or_else(
        |_| root.join("data").join("cache").join("llm").join("fixtures"),
        PathBuf::from,
    )
}

/// Returns true when the caller has opted in to a live LLM backend via
/// `CAREERAI_LLM_LIVE=1`.
pub(crate) fn live_llm_opt_in() -> bool {
    std::env::var("CAREERAI_LLM_LIVE").ok().as_deref() == Some("1")
}

/// Tailor a shortlisted listing into an application row + persisted payload.
///
/// Backend selection: `CAREERAI_LLM_LIVE=1` opts in to a live backend
/// (`Backend::resolve` picks CLI vs API per `cfg.llm.backend`). Without
/// the env var, falls back to the `MockLlm` fixtures dir.
pub async fn tailor_one(
    root: &Path,
    cfg: &CoreConfig,
    listing_id: &str,
) -> Result<TailoredOutcome> {
    let pool = open_pool(root).await?;

    let listing = match queries::find_by_id(&pool, listing_id).await {
        Ok(l) => l,
        Err(careerai_db::DbError::NotFound(_)) => {
            anyhow::bail!("listing not found: {listing_id}");
        }
        Err(e) => return Err(e).context("fetch listing"),
    };

    if listing.state != "shortlisted" {
        anyhow::bail!(
            "listing {listing_id} is in state '{}'; expected 'shortlisted'",
            listing.state
        );
    }

    let profile = load_profile(root)?;

    info!(
        target = "tailor",
        listing_id = %listing.id,
        title = %listing.title,
        company = %listing.company,
        "tailoring listing"
    );

    #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
    {
        if live_llm_opt_in() {
            use std::sync::Arc;
            let cache_root = if cfg.llm.cache_dir.is_empty() {
                root.join("data").join("cache").join("llm")
            } else {
                let p = std::path::PathBuf::from(&cfg.llm.cache_dir);
                if p.is_absolute() {
                    p
                } else {
                    root.join(p)
                }
            };
            let cache = Arc::new(careerai_llm::Cache::new(cache_root));
            match careerai_llm::Backend::resolve(cfg.llm.backend, &cfg.llm, cache).await {
                Ok(backend) => {
                    let outcome =
                        tailor_for_listing(&pool, &backend, &listing.id, &profile, &cfg.llm)
                            .await
                            .context("tailor_for_listing")?;
                    return Ok(TailoredOutcome {
                        application_id: outcome.application_id,
                        listing_title: listing.title,
                        company: listing.company,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        target = "tailor",
                        error = %e,
                        "live backend unavailable; falling back to MockLlm fixtures"
                    );
                }
            }
        }
    }

    let fixtures = fixtures_dir(root);
    if !fixtures.is_dir() {
        anyhow::bail!(
            "no LLM fixtures at {}; either set CAREERAI_LLM_LIVE=1 (with a live \
             backend compiled in) or provide fixtures via CAREERAI_LLM_FIXTURES_DIR \
             / `<root>/data/cache/llm/fixtures/`",
            fixtures.display()
        );
    }
    let llm = MockLlm::from_dir(&fixtures)
        .with_context(|| format!("load llm fixtures from {}", fixtures.display()))?;

    let outcome = tailor_for_listing(&pool, &llm, &listing.id, &profile, &cfg.llm)
        .await
        .context("tailor_for_listing")?;

    Ok(TailoredOutcome {
        application_id: outcome.application_id,
        listing_title: listing.title,
        company: listing.company,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    /// Serialize the env-var mutations in this module's tests.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Regression for the `tailor_one` live-call gate. The helper must
    /// return `true` only for the literal value "1".
    #[test]
    fn live_llm_opt_in_true_only_for_literal_one() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var("CAREERAI_LLM_LIVE").ok();
        std::env::set_var("CAREERAI_LLM_LIVE", "1");
        assert!(super::live_llm_opt_in(), "literal '1' must opt in");
        std::env::set_var("CAREERAI_LLM_LIVE", "true");
        assert!(
            !super::live_llm_opt_in(),
            "'true' must NOT opt in (only '1')"
        );
        std::env::set_var("CAREERAI_LLM_LIVE", "");
        assert!(!super::live_llm_opt_in(), "empty must NOT opt in");
        std::env::remove_var("CAREERAI_LLM_LIVE");
        assert!(!super::live_llm_opt_in(), "unset must NOT opt in");
        if let Some(v) = prev {
            std::env::set_var("CAREERAI_LLM_LIVE", v);
        }
    }
}
