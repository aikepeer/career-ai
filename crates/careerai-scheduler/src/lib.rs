//! Scheduler daemon.
//!
//! Wraps `tokio-cron-scheduler` and registers one async job per configured
//! source cadence. Each tick fires the real `discover` → `match` half of
//! the pipeline for that single source. A separate submit cadence (Wave 3)
//! drives a periodic dry-run apply sweep when `scheduler.submit_cadence` is
//! set. The submit job hard-pins `auto_submit=false` — the daemon never
//! live-submits.
//!
//! Architectural boundary (per `CLAUDE.md`): this crate owns daemon, cron
//! wiring, and graceful shutdown only. It does not pull in `careerai-sources`,
//! `careerai-match`, `careerai-submit`, `careerai-db`, or any other
//! implementation crate directly — the orchestration layer is
//! `careerai-pipeline`, and this crate goes through it.
//!
//! ## Module layout (split to stay under the 300-LOC cap)
//!
//! * `error.rs` — `SchedulerError` enum + `SHUTDOWN_DRAIN` constant
//! * `cron.rs`  — `build_tick_job`, `build_submit_job`, `effective_cadence`
//! * `lib.rs`   — `Scheduler` struct, `from_config`, lifecycle methods

mod cron;
mod error;
mod shutdown;
#[cfg(test)]
mod tests;

pub use error::SchedulerError;

use std::path::{Path, PathBuf};
#[cfg(feature = "test-hooks")]
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use careerai_core::config::CoreConfig;
use tokio::sync::Mutex;
use tokio_cron_scheduler::JobScheduler;
use tracing::{error, info, info_span, warn, Instrument};

use crate::cron::{build_submit_job, build_tick_job, effective_cadence};
use crate::error::SHUTDOWN_DRAIN;

/// Long-running daemon scaffolding around `tokio-cron-scheduler`.
///
/// At construction time we mint a process-wide `match_lock`
/// (`tokio::sync::Mutex`) that every per-source tick acquires before calling
/// `careerai_pipeline::match_one`. `match_all` walks every
/// `discovered`-state row regardless of source (deliberate global rescore
/// behavior), so concurrent ticks from different sources would otherwise
/// race on the SQLite UPDATE that transitions rows out of `discovered`. The
/// lock serializes those write paths without affecting `discover_one`,
/// which is naturally per-source and HTTP-bound. The lock itself is owned
/// by each job closure (cloned `Arc`s) — we do not need to store it on
/// `Self` because the registered jobs keep it alive for the daemon's
/// lifetime.
pub struct Scheduler {
    pub(crate) inner: JobScheduler,
}

// `tokio_cron_scheduler::JobScheduler` does not implement `Debug`, so we emit a
// manual placeholder. The workspace lint set warns on missing `Debug` impls,
// and dropping the derive entirely would violate that.
impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler").finish_non_exhaustive()
    }
}

impl Scheduler {
    /// Build a scheduler from `CoreConfig` rooted at `root`. Registers one
    /// cron `Job` per entry in `cfg.scheduler.cadence`. Each job, when fired,
    /// runs `careerai_pipeline::discover_one` followed by
    /// `careerai_pipeline::match_one` for that single source. Invalid cron
    /// expressions are logged and skipped — they must not abort daemon
    /// startup, since one typo in `local.yaml` would otherwise wedge every
    /// other source.
    ///
    /// `root` is the project root used by every pipeline entry point to
    /// locate `data/`, `profile/`, and `config/`. It is the same path the
    /// CLI passes (`std::env::current_dir()`).
    pub async fn from_config(root: &Path, cfg: &CoreConfig) -> Result<Self, SchedulerError> {
        Self::from_config_inner(
            root,
            cfg,
            #[cfg(feature = "test-hooks")]
            None,
        )
        .await
    }

    /// Test-only constructor that threads an `Arc<AtomicUsize>` into every
    /// registered job, so a test can wait for the first tick to land. The
    /// counter is incremented once per fired tick, regardless of which job
    /// fired.
    #[cfg(feature = "test-hooks")]
    pub async fn from_config_with_counter(
        root: &Path,
        cfg: &CoreConfig,
        counter: Arc<AtomicUsize>,
    ) -> Result<Self, SchedulerError> {
        Self::from_config_inner(root, cfg, Some(counter)).await
    }

    async fn from_config_inner(
        root: &Path,
        cfg: &CoreConfig,
        #[cfg(feature = "test-hooks")] tick_counter: Option<Arc<AtomicUsize>>,
    ) -> Result<Self, SchedulerError> {
        let span = info_span!("scheduler");
        let root_buf = root.to_path_buf();
        let cfg_clone = cfg.clone();
        async move {
            let inner = JobScheduler::new().await?;

            // Build the effective per-source cron map: start from
            // `scheduler.cadence`, then let each enabled `sources.mcp[*]`
            // entry with a `cron` field override its name's entry. This
            // closes the PR-19 follow-up that flagged
            // `McpSourceConfig.cron` as parsed but unused.
            let cadence = effective_cadence(&cfg_clone);

            if cadence.is_empty() {
                warn!("scheduler.cadence is empty — daemon will idle with no jobs");
            }

            // Share root + cfg into every job closure via `Arc` so we don't
            // clone the full `CoreConfig` once per tick. The closures are
            // `'static` (required by tokio-cron-scheduler), so they each take
            // their own `Arc` clone.
            let root: Arc<PathBuf> = Arc::new(root_buf);
            let cfg: Arc<CoreConfig> = Arc::new(cfg_clone);
            let match_lock: Arc<Mutex<()>> = Arc::new(Mutex::new(()));

            // Sort to keep startup logs deterministic across runs (HashMap is
            // not ordered). Helps a lot when diffing daemon logs in CI / local.
            let mut entries: Vec<(&String, &String)> = cadence.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));

            for (source_name, cron_expr) in entries {
                let built = build_tick_job(
                    source_name,
                    cron_expr,
                    Arc::clone(&root),
                    Arc::clone(&cfg),
                    Arc::clone(&match_lock),
                    #[cfg(feature = "test-hooks")]
                    tick_counter.clone(),
                );
                match built {
                    Ok(job) => match inner.add(job).await {
                        Ok(uuid) => info!(
                            source = %source_name,
                            cron = %cron_expr,
                            job = %uuid,
                            "registered cron job",
                        ),
                        Err(e) => error!(
                            source = %source_name,
                            cron = %cron_expr,
                            error = %e,
                            "failed to add cron job; skipping this source",
                        ),
                    },
                    Err(e) => error!(
                        source = %source_name,
                        cron = %cron_expr,
                        error = %e,
                        "invalid cron expression; skipping this source",
                    ),
                }
            }

            // Register the submit cron exactly once if configured. Unset means
            // the daemon does not poll for apply at all — operators must run
            // `careerai apply` manually. Same swallow-and-log pattern as the
            // per-source jobs: never propagate, never panic.
            if let Some(submit_cron) = cfg.scheduler.submit_cadence.as_deref() {
                let built = build_submit_job(
                    submit_cron,
                    Arc::clone(&root),
                    Arc::clone(&cfg),
                    #[cfg(feature = "test-hooks")]
                    tick_counter.clone(),
                );
                match built {
                    Ok(job) => match inner.add(job).await {
                        Ok(uuid) => info!(
                            cron = %submit_cron,
                            job = %uuid,
                            "registered submit cron job (dry-run only)",
                        ),
                        Err(e) => error!(
                            cron = %submit_cron,
                            error = %e,
                            "failed to add submit cron job; skipping",
                        ),
                    },
                    Err(e) => error!(
                        cron = %submit_cron,
                        error = %e,
                        "invalid submit_cadence cron expression; skipping",
                    ),
                }
            } else {
                info!("submit cron disabled — set scheduler.submit_cadence to enable periodic dry-run apply sweeps");
            }

            Ok(Self { inner })
        }
        .instrument(span)
        .await
    }

    /// Start the underlying scheduler in the background. Cron firings begin
    /// after this returns.
    pub async fn start(&self) -> Result<(), SchedulerError> {
        self.inner.start().await?;
        info!(parent: &info_span!("scheduler"), "scheduler started");
        Ok(())
    }

    /// Shut down the scheduler, draining in-flight jobs up to
    /// `SHUTDOWN_DRAIN`. Used by `run_until_shutdown` on signal and by
    /// integration tests that need to stop the cron loop without waiting
    /// for SIGINT/SIGTERM.
    pub async fn shutdown(&mut self) -> Result<(), SchedulerError> {
        let drain = tokio::time::timeout(SHUTDOWN_DRAIN, self.inner.shutdown()).await;
        match drain {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(SchedulerError::from(e)),
            Err(_) => {
                warn!(
                    drain_secs = SHUTDOWN_DRAIN.as_secs(),
                    "scheduler shutdown timed out; returning anyway",
                );
                Ok(())
            }
        }
    }
}
