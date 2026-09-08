//! Tailor stage — LLM-tailor a shortlisted listing into an
//! application row + persisted payload.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_llm::mock::MockLlm;
use careerai_tailor::tailor_for_listing;
use sqlx::SqlitePool;

use crate::{load_profile, open_pool};

#[derive(Debug)]
pub struct TailoredOutcome {
    pub application_id: String,
    pub listing_title: String,
    pub company: String,
}

/// Resolve the LLM fixtures directory. Honors the `CAREERAI_LLM_FIXTURES_DIR`
/// environment override, else falls back to `<root>/data/cache/llm/fixtures`.
fn fixtures_dir(root: &Path) -> PathBuf {
    std::env::var("CAREERAI_LLM_FIXTURES_DIR").map_or_else(
        |_| root.join("data").join("cache").join("llm").join("fixtures"),
        PathBuf::from,
    )
}

/// Returns true when the caller has opted in to a live LLM backend via
/// `CAREERAI_LLM_LIVE=1`. Any other value (including unset) means use
/// fixtures. Centralized so the gate logic is unit-testable without
/// spinning up a DB.
fn live_llm_opt_in() -> bool {
    std::env::var("CAREERAI_LLM_LIVE").ok().as_deref() == Some("1")
}

/// Tailor a shortlisted listing into an application row + persisted payload.
///
/// Backend selection: `CAREERAI_LLM_LIVE=1` opts in to a live backend
/// (`Backend::resolve` picks CLI vs API per `cfg.llm.backend`). Without
/// the env var, falls back to the `MockLlm` fixtures dir at
/// `CAREERAI_LLM_FIXTURES_DIR` (default `<root>/data/cache/llm/fixtures`)
/// — this keeps integration tests deterministic even on dev boxes with
/// a real `claude` install or `ANTHROPIC_API_KEY`.
#[allow(clippy::too_many_lines)]
pub async fn tailor_one(
    root: &Path,
    cfg: &CoreConfig,
    listing_id: &str,
) -> Result<TailoredOutcome> {
    let pool = open_pool(root).await?;
    tailor_one_with_pool(&pool, root, cfg, listing_id).await
}

/// Inner implementation that reuses an already-open pool. `tailor_all`
/// passes its shared pool here — opening one pool per listing would
/// create N SQLite pools (up to 8 connections each) for a batch run.
async fn tailor_one_with_pool(
    pool: &SqlitePool,
    root: &Path,
    cfg: &CoreConfig,
    listing_id: &str,
) -> Result<TailoredOutcome> {
    let listing = match queries::find_by_id(pool, listing_id).await {
        Ok(l) => l,
        Err(careerai_db::DbError::NotFound(_)) => {
            anyhow::bail!("listing not found: {listing_id}");
        }
        Err(e) => return Err(e).context("fetch listing"),
    };

    if listing.state != ListingState::Shortlisted.as_str() {
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
        strategy = %cfg.llm.strategy,
        "tailoring listing"
    );

    // Fast-path: Phase 1 Local Deterministic Tailoring (zero LLM calls).
    let is_live = {
        #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
        {
            live_llm_opt_in()
        }
        #[cfg(not(any(feature = "live-llm-cli", feature = "live-llm-api")))]
        {
            false
        }
    };

    #[allow(clippy::cast_possible_truncation)]
    let score = listing.score.unwrap_or(0.0) as f32;
    let min_llm_score = cfg.llm.llm_min_score;

    // R05: strategy == "local" is authoritative — it routes to local
    // deterministic tailoring even when is_live is true. The dashboard
    // sets CAREERAI_LLM_LIVE=1 for all subprocesses, which would
    // otherwise override a `local` strategy and make the LLM call
    // unavoidable. The env override CAREERAI_TAILOR_STRATEGY=local and
    // the hybrid below-threshold path remain as additional local routes.
    let use_local = cfg.llm.strategy == "local"
        || std::env::var("CAREERAI_TAILOR_STRATEGY").as_deref() == Ok("local")
        || (!is_live && !fixtures_dir(root).is_dir())
        || (is_live
            && cfg.llm.strategy == "hybrid"
            && listing.score.is_some()
            && score < min_llm_score);

    if use_local {
        info!(
            target = "tailor",
            listing_id = %listing.id,
            score = %score,
            min_llm_score = %min_llm_score,
            strategy = %cfg.llm.strategy,
            "routing to local deterministic tailoring"
        );
        let outcome = careerai_tailor::tailor_for_listing_local(
            pool,
            &listing.id,
            &profile,
            cfg.llm.drop_threshold,
        )
        .await
        .context("local tailor_for_listing")?;

        return Ok(TailoredOutcome {
            application_id: outcome.application_id,
            listing_title: listing.title,
            company: listing.company,
        });
    }

    // A live opt-in is fail-closed: backend resolution and every live
    // tailoring hop must succeed. Fixtures are an explicit offline mode,
    // never a recovery path for a requested live run. This prevents stale
    // fixture output from being persisted or inserted into the live cache.
    live_or_fixture_tailor(pool, cfg, root, &listing, &profile).await
}

/// Tailor via a live LLM backend (when compiled in and opted in) or via
/// `MockLlm` fixtures. Both paths funnel through
/// `tailor_for_listing` with the shared pool.
async fn live_or_fixture_tailor(
    pool: &SqlitePool,
    cfg: &CoreConfig,
    root: &Path,
    listing: &careerai_db::models::Listing,
    profile: &careerai_profile::schema::Profile,
) -> Result<TailoredOutcome> {
    #[cfg(any(feature = "live-llm-cli", feature = "live-llm-api"))]
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
        let backend = careerai_llm::Backend::resolve(cfg.llm.backend.clone(), &cfg.llm, cache)
            .await
            .map_err(|e| anyhow::anyhow!("resolve live LLM backend: {e}"))?;
        let outcome = tailor_for_listing(pool, &backend, &listing.id, profile, &cfg.llm, root)
            .await
            .context("live tailor_for_listing")?;
        return Ok(TailoredOutcome {
            application_id: outcome.application_id,
            listing_title: listing.title.clone(),
            company: listing.company.clone(),
        });
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

    let outcome = tailor_for_listing(pool, &llm, &listing.id, profile, &cfg.llm, root)
        .await
        .context("tailor_for_listing")?;

    Ok(TailoredOutcome {
        application_id: outcome.application_id,
        listing_title: listing.title.clone(),
        company: listing.company.clone(),
    })
}

/// Tailor all currently shortlisted listings in parallel, up to optional limit.
pub async fn tailor_all(
    root: &Path,
    cfg: &CoreConfig,
    limit: Option<usize>,
) -> Result<Vec<TailoredOutcome>> {
    let pool = open_pool(root).await?;
    let mut q = sqlx::QueryBuilder::new(
        "SELECT id FROM listings WHERE state = 'shortlisted' ORDER BY score DESC",
    );
    if let Some(n) = limit {
        q.push(" LIMIT ")
            .push_bind(i64::try_from(n).unwrap_or(i64::MAX));
    }
    let rows: Vec<(String,)> = q
        .build_query_as()
        .fetch_all(&pool)
        .await
        .context("fetch shortlisted listings")?;

    let total = rows.len();
    if total == 0 {
        return Ok(Vec::new());
    }

    let completed = Arc::new(AtomicUsize::new(0));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
    let cfg_arc = Arc::new(cfg.clone());
    let mut tasks = Vec::with_capacity(total);

    for (id,) in rows {
        let sem = Arc::clone(&semaphore);
        let completed = Arc::clone(&completed);
        let root_buf = root.to_path_buf();
        let cfg_clone = Arc::clone(&cfg_arc);
        let pool_clone = pool.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok()?;
            match tailor_one_with_pool(&pool_clone, &root_buf, &cfg_clone, &id).await {
                Ok(outcome) => {
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    info!(
                        done,
                        total,
                        title = %outcome.listing_title,
                        company = %outcome.company,
                        application_id = %outcome.application_id,
                        "tailored listing"
                    );
                    Some(outcome)
                }
                Err(e) => {
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::warn!(
                        listing_id = %id,
                        error = %format_args!("{e:#}"),
                        progress = format!("{done}/{total}"),
                        "tailor_all: failed to tailor listing"
                    );
                    None
                }
            }
        }));
    }

    let mut outcomes = Vec::with_capacity(total);
    for task in tasks {
        if let Ok(Some(outcome)) = task.await {
            outcomes.push(outcome);
        }
    }
    Ok(outcomes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Serialize the env-var mutations in this module's tests so parallel
    /// cargo-test threads can't race on `CAREERAI_LLM_LIVE`.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Regression for the `tailor_one` live-call gate. The helper must
    /// return `true` only for the literal value "1"; any other value
    /// (or absence) means use the `MockLlm` fixtures path. Without this
    /// gate, `tailor_one` made nondeterministic live calls on dev boxes
    /// with an authed `claude` binary, breaking `tailor_render_it.rs`.
    #[test]
    fn live_llm_opt_in_true_only_for_literal_one() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var("CAREERAI_LLM_LIVE").ok();
        std::env::set_var("CAREERAI_LLM_LIVE", "1");
        assert!(live_llm_opt_in(), "literal '1' must opt in");
        std::env::set_var("CAREERAI_LLM_LIVE", "true");
        assert!(!live_llm_opt_in(), "'true' must NOT opt in (only '1')");
        std::env::set_var("CAREERAI_LLM_LIVE", "");
        assert!(!live_llm_opt_in(), "empty must NOT opt in");
        std::env::remove_var("CAREERAI_LLM_LIVE");
        assert!(!live_llm_opt_in(), "unset must NOT opt in");
        if let Some(v) = prev {
            std::env::set_var("CAREERAI_LLM_LIVE", v);
        }
    }
}
