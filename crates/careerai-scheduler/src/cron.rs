//! Cron-job builders — `build_tick_job`, `build_submit_job`, and
//! `effective_cadence`. Pure functions that construct `tokio-cron-scheduler`
//! jobs from config. Kept separate so `lib.rs` stays under the 300-LOC cap.

use std::path::PathBuf;
use std::sync::Arc;

use careerai_core::config::CoreConfig;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobSchedulerError};
use tracing::{error, info};

/// Build the per-tick async job for a single source.
///
/// Each fired tick runs `discover_one` then `match_one` against the shared
/// pipeline crate, scoped to this single source. Errors from either stage
/// are logged at `error` and swallowed — one source's failure must never
/// propagate up and cancel sibling jobs registered with the same scheduler.
///
/// `match_lock` (H1) serializes the `match_one` call across all per-source
/// jobs. `match_all` walks every `discovered`-state row regardless of
/// source, so two sources ticking at the same minute (default greenhouse +
/// lever both fire `0 0 */1 * * *`) would otherwise race on the SQLite
/// UPDATE that transitions rows out of `discovered` — both would fetch the
/// same set, both would try to set state, and one would clobber the other's
/// score or hit a unique-state constraint. Discover stays unlocked: it's
/// per-source, network-bound, and writes only its own source's rows.
pub(crate) fn build_tick_job(
    source: &str,
    cron_expr: &str,
    root: Arc<PathBuf>,
    cfg: Arc<CoreConfig>,
    match_lock: Arc<Mutex<()>>,
    #[cfg(feature = "test-hooks")] tick_counter: Option<Arc<std::sync::atomic::AtomicUsize>>,
) -> Result<Job, JobSchedulerError> {
    let source_owned = source.to_owned();
    Job::new_async(cron_expr, move |_uuid, _scheduler| {
        let source = source_owned.clone();
        let root = Arc::clone(&root);
        let cfg = Arc::clone(&cfg);
        let match_lock = Arc::clone(&match_lock);
        #[cfg(feature = "test-hooks")]
        let tick_counter = tick_counter.clone();
        Box::pin(async move {
            // Each stage's Result is matched independently: a discover
            // failure should not block the match retry, since match runs
            // against rows already in the DB from prior ticks. Both arms
            // log per-source so log filtering by `source=...` keeps working.
            match careerai_pipeline::discover_one(root.as_path(), cfg.as_ref(), &source).await {
                Ok(report) => info!(
                    source = %source,
                    fetched = report.fetched,
                    new = report.new_rows,
                    duplicates = report.duplicates,
                    errors = report.errors,
                    "discover ok",
                ),
                Err(e) => error!(source = %source, error = %e, "discover failed"),
            }

            // H1: serialize the global match write path across sources.
            {
                let _lock = match_lock.lock().await;
                match careerai_pipeline::match_one(root.as_path(), cfg.as_ref(), &source).await {
                    Ok(report) => info!(
                        source = %source,
                        filtered_out = report.filtered_out,
                        shortlisted = report.shortlisted,
                        below_threshold = report.also_filtered,
                        "match ok",
                    ),
                    Err(e) => error!(source = %source, error = %e, "match failed"),
                }
            }

            #[cfg(feature = "test-hooks")]
            if let Some(counter) = &tick_counter {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    })
}

/// Build the submit cron job. The daemon hard-pins `auto_submit=false` —
/// live submission is operator-driven only (`careerai apply --auto-submit
/// ...`). This invariant is non-negotiable: the daemon must never live-submit
/// from a cron tick. The `debug_assert!` below documents the contract and
/// trips loudly in test/dev builds if someone ever flips the flag.
pub(crate) fn build_submit_job(
    cron_expr: &str,
    root: Arc<PathBuf>,
    cfg: Arc<CoreConfig>,
    #[cfg(feature = "test-hooks")] tick_counter: Option<Arc<std::sync::atomic::AtomicUsize>>,
) -> Result<Job, JobSchedulerError> {
    Job::new_async(cron_expr, move |_uuid, _scheduler| {
        let root = Arc::clone(&root);
        let cfg = Arc::clone(&cfg);
        #[cfg(feature = "test-hooks")]
        let tick_counter = tick_counter.clone();
        Box::pin(async move {
            // SAFETY INVARIANT (see CLAUDE.md "Submit safety invariant"):
            // the scheduler tick must always force dry-run. We pass
            // `Some(false)` to override whatever `cfg.submit.auto_submit`
            // says — operators can still flip live mode for one-shot CLI
            // runs, but the daemon never live-submits.
            let auto_submit_override: Option<bool> = Some(false);
            debug_assert!(
                matches!(auto_submit_override, Some(false)),
                "scheduler submit job must always be dry-run",
            );

            match careerai_pipeline::apply_all(
                root.as_path(),
                cfg.as_ref(),
                None, // no source filter — sweep every eligible application
                auto_submit_override,
            )
            .await
            {
                Ok(outcomes) => info!(submitted = outcomes.len(), "apply tick",),
                Err(e) => error!(error = %e, "apply tick failed"),
            }

            #[cfg(feature = "test-hooks")]
            if let Some(counter) = &tick_counter {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    })
}

/// Compose the effective per-source cron map from `cfg.scheduler.cadence`
/// plus per-source overrides on enabled `sources.mcp[*]` entries.
///
/// Resolution order, highest priority last:
///   1. `cfg.scheduler.cadence.<name>` — global cadence map.
///   2. `cfg.sources.mcp[name=<name>].cron` (when enabled and `Some`) —
///      per-source override.
///
/// Disabled MCP sources (`enabled: false`) are ignored entirely **and any
/// matching `scheduler.cadence` entry is removed** before per-source
/// overrides are applied. That way flipping `enabled: false` on an MCP
/// source can never accidentally leave a stale global cadence active —
/// disabling an MCP cancels its cron unconditionally. ATS sources have no
/// per-source `cron` field today; if that ever lands, extend this helper
/// rather than the caller.
pub(crate) fn effective_cadence(cfg: &CoreConfig) -> std::collections::HashMap<String, String> {
    let mut cadence: std::collections::HashMap<String, String> = cfg.scheduler.cadence.clone();
    // First pass: drop cadence entries for disabled MCP sources so a stale
    // `scheduler.cadence` value cannot survive `enabled: false`.
    for mcp in &cfg.sources.mcp {
        if !mcp.enabled && cadence.remove(&mcp.name).is_some() {
            info!(
                source = %mcp.name,
                "disabled mcp source removed stale scheduler.cadence entry",
            );
        }
    }
    // Second pass: apply per-source cron overrides for enabled MCP sources.
    for mcp in &cfg.sources.mcp {
        if !mcp.enabled {
            continue;
        }
        if let Some(cron_expr) = &mcp.cron {
            if let Some(prev) = cadence.insert(mcp.name.clone(), cron_expr.clone()) {
                if &prev != cron_expr {
                    info!(
                        source = %mcp.name,
                        global = %prev,
                        per_source = %cron_expr,
                        "mcp source cron overrides scheduler.cadence",
                    );
                }
            } else {
                info!(
                    source = %mcp.name,
                    cron = %cron_expr,
                    "mcp source cron registered (no cadence entry)",
                );
            }
        }
    }
    cadence
}
