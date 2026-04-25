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

use std::path::{Path, PathBuf};
#[cfg(feature = "test-hooks")]
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;

use careerai_core::config::CoreConfig;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tracing::{error, info, info_span, warn, Instrument};

/// Time the scheduler is allowed to drain after a shutdown signal before we
/// give up and return anyway. Kept short so `careerai daemon` always exits
/// promptly under operator Ctrl-C.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

/// Errors surfaced by the scheduler crate.
///
/// Kept small on purpose. The cron jobs themselves swallow their own errors
/// and log via `tracing::error!`, so a single bad source can never propagate
/// out and tear the whole daemon down.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// Either constructing the underlying `JobScheduler`, registering a job,
    /// starting it, or shutting it down failed.
    #[error("scheduler init failed: {0}")]
    Init(#[from] JobSchedulerError),

    /// Wiring SIGTERM via `tokio::signal::unix` failed. SIGINT goes through
    /// `tokio::signal::ctrl_c` which surfaces its own `io::Error` here too.
    #[error("signal handler install failed: {0}")]
    Signal(#[from] std::io::Error),
}

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
    inner: JobScheduler,
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

            if cfg_clone.scheduler.cadence.is_empty() {
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
            let mut entries: Vec<(&String, &String)> = cfg.scheduler.cadence.iter().collect();
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
                // H5: never silent. Operators who typo `submit_cadence` (or set
                // it under the wrong YAML key) need a visible startup line so
                // they can tell the difference between "intentionally disabled"
                // and "I thought I enabled this and it's quietly not running".
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

    /// Run until SIGINT or SIGTERM, then gracefully shut down.
    ///
    /// On signal we call `JobScheduler::shutdown()` and wait up to
    /// `SHUTDOWN_DRAIN` for in-flight jobs to settle. If the drain times out
    /// we still return `Ok(())` — the daemon's contract is "exit promptly on
    /// Ctrl-C", not "block until every job finishes".
    pub async fn run_until_shutdown(mut self) -> Result<(), SchedulerError> {
        let span = info_span!("scheduler");
        async move {
            // H2: install both signal handlers BEFORE starting the scheduler.
            // `tokio::signal::ctrl_c` registers lazily on first poll, and the
            // unix `signal()` call itself does the syscall, so any signal
            // delivered between `start()` returning and the first `select!`
            // poll could otherwise fall through to the default disposition
            // (terminate without drain). Registering both here closes that
            // window — the kernel queues SIGINT/SIGTERM into our handlers as
            // soon as `signal()` returns.
            let mut sigint = signal(SignalKind::interrupt())?;
            let mut sigterm = signal(SignalKind::terminate())?;

            self.start().await?;

            // SIGTERM (e.g. systemd stop) and SIGINT (Ctrl-C) both initiate
            // graceful shutdown. Race them with `tokio::select!` so whichever
            // arrives first wins.
            tokio::select! {
                _ = sigint.recv() => {
                    info!("received SIGINT, shutting down");
                }
                _ = sigterm.recv() => {
                    info!("received SIGTERM, shutting down");
                }
            }

            let drain = tokio::time::timeout(SHUTDOWN_DRAIN, self.inner.shutdown()).await;
            match drain {
                Ok(Ok(())) => info!("scheduler drained cleanly"),
                Ok(Err(e)) => error!(error = %e, "scheduler shutdown reported an error"),
                Err(_) => warn!(
                    drain_secs = SHUTDOWN_DRAIN.as_secs(),
                    "scheduler shutdown timed out; exiting anyway",
                ),
            }
            Ok::<(), SchedulerError>(())
        }
        .instrument(span)
        .await
    }
}

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
fn build_tick_job(
    source: &str,
    cron_expr: &str,
    root: Arc<PathBuf>,
    cfg: Arc<CoreConfig>,
    match_lock: Arc<Mutex<()>>,
    #[cfg(feature = "test-hooks")] tick_counter: Option<Arc<AtomicUsize>>,
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
            // Lock is acquired in its own scope so it drops as soon as
            // `match_one` returns, never held across discover.
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
fn build_submit_job(
    cron_expr: &str,
    root: Arc<PathBuf>,
    cfg: Arc<CoreConfig>,
    #[cfg(feature = "test-hooks")] tick_counter: Option<Arc<AtomicUsize>>,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Load a fresh `CoreConfig` backed by the embedded defaults — same
    /// pattern `careerai-core` uses in its own unit tests. Returns the
    /// config plus the tempdir guarding the scratch root, so the dir
    /// outlives the test body.
    fn embedded_cfg() -> (tempfile::TempDir, CoreConfig) {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = CoreConfig::load(tmp.path()).unwrap();
        (tmp, cfg)
    }

    #[tokio::test]
    async fn from_config_with_embedded_defaults_succeeds() {
        let (tmp, cfg) = embedded_cfg();
        // Embedded defaults declare a non-empty cadence map, so this also
        // exercises the per-source registration branch.
        assert!(!cfg.scheduler.cadence.is_empty());
        let sched = Scheduler::from_config(tmp.path(), &cfg).await;
        assert!(sched.is_ok(), "embedded cadence must register cleanly");
    }

    #[tokio::test]
    async fn from_config_with_empty_cadence_succeeds() {
        let (tmp, mut cfg) = embedded_cfg();
        cfg.scheduler.cadence.clear();
        let sched = Scheduler::from_config(tmp.path(), &cfg).await;
        assert!(sched.is_ok(), "empty cadence must not be an error");
    }

    #[tokio::test]
    async fn from_config_skips_invalid_cron_without_failing() {
        // A clearly invalid cron string must not abort startup. Other
        // (valid) sources should still register.
        let (tmp, mut cfg) = embedded_cfg();
        cfg.scheduler.cadence.clear();
        cfg.scheduler
            .cadence
            .insert("greenhouse".into(), "0 0 */1 * * *".into());
        cfg.scheduler
            .cadence
            .insert("bogus".into(), "this is not a cron expression".into());
        let sched = Scheduler::from_config(tmp.path(), &cfg).await;
        assert!(
            sched.is_ok(),
            "invalid cron in one source must not fail the scheduler",
        );
    }

    #[tokio::test]
    async fn start_then_immediate_shutdown_is_clean() {
        let (tmp, mut cfg) = embedded_cfg();
        cfg.scheduler.cadence.clear();
        cfg.scheduler
            .cadence
            .insert("greenhouse".into(), "0 0 */1 * * *".into());

        let mut sched = Scheduler::from_config(tmp.path(), &cfg).await.unwrap();
        sched.start().await.unwrap();
        // Use the public shutdown API — same path `run_until_shutdown` uses.
        sched.shutdown().await.expect("shutdown should not error");
    }
}
