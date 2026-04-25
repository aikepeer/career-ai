//! Scheduler daemon.
//!
//! Wraps `tokio-cron-scheduler` and registers one async job per configured
//! source cadence. Each cron tick logs at `info` for now — Wave 2 will replace
//! the per-source closure body with real pipeline calls (discover → match →
//! tailor → render → submit) without changing this module's public API.
//!
//! Architectural boundary (per `CLAUDE.md`): this crate owns daemon, cron
//! wiring, and graceful shutdown only. It must not import submission or
//! render-layer code directly. Wave 2 will plug in pipeline entry points
//! through `careerai-core` so this rule continues to hold.

use std::time::Duration;

use careerai_core::config::CoreConfig;
use tokio::signal::unix::{signal, SignalKind};
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
/// Wave 1 only registers cron entries from `scheduler.cadence` and logs ticks.
/// Wave 2 will dispatch into the real pipeline from inside each registered
/// closure.
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
    /// Build a scheduler from `CoreConfig`. Registers one cron `Job` per entry
    /// in `cfg.scheduler.cadence`. Invalid cron expressions are logged and
    /// skipped — they must not abort daemon startup, since one typo in
    /// `local.yaml` would otherwise wedge every other source.
    pub async fn from_config(cfg: &CoreConfig) -> Result<Self, SchedulerError> {
        let inner = JobScheduler::new().await?;
        let span = info_span!("scheduler");
        let _enter = span.enter();

        if cfg.scheduler.cadence.is_empty() {
            warn!("scheduler.cadence is empty — daemon will idle with no jobs");
        }

        // Sort to keep startup logs deterministic across runs (HashMap is not
        // ordered). Helps a lot when diffing daemon logs in CI / local.
        let mut entries: Vec<(&String, &String)> = cfg.scheduler.cadence.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        for (source_name, cron_expr) in entries {
            match build_tick_job(source_name, cron_expr) {
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

        Ok(Self { inner })
    }

    /// Start the underlying scheduler in the background. Cron firings begin
    /// after this returns.
    pub async fn start(&self) -> Result<(), SchedulerError> {
        self.inner.start().await?;
        info!(parent: &info_span!("scheduler"), "scheduler started");
        Ok(())
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
            self.start().await?;

            // SIGTERM (e.g. systemd stop) and SIGINT (Ctrl-C) both initiate
            // graceful shutdown. Race them with `tokio::select!` so whichever
            // arrives first wins.
            let mut sigterm = signal(SignalKind::terminate())?;
            tokio::select! {
                res = tokio::signal::ctrl_c() => {
                    match res {
                        Ok(()) => info!("received SIGINT, shutting down"),
                        Err(e) => error!(error = %e, "ctrl_c handler failed; shutting down anyway"),
                    }
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
/// Wave 1 just logs. The closure is structured so Wave 2 can swap the body
/// for a real pipeline call without touching the surrounding scaffolding —
/// in particular, errors raised inside must be caught and logged, never
/// propagated, so one source's failure cannot cancel the others' jobs.
fn build_tick_job(source: &str, cron_expr: &str) -> Result<Job, JobSchedulerError> {
    let source_owned = source.to_owned();
    Job::new_async(cron_expr, move |_uuid, _scheduler| {
        let source = source_owned.clone();
        Box::pin(async move {
            // Wave 2 replaces the body of this inner async with the real
            // pipeline call. Keep the catch-all so a panicking or erroring
            // source can never cancel other registered jobs.
            let outcome: Result<(), &'static str> = async {
                info!(source = %source, "tick");
                Ok(())
            }
            .await;
            if let Err(e) = outcome {
                error!(source = %source, error = %e, "scheduled tick failed");
            }
        })
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Load a fresh `CoreConfig` backed by the embedded defaults — same
    /// pattern `careerai-core` uses in its own unit tests.
    fn embedded_cfg() -> CoreConfig {
        let tmp = tempfile::tempdir().unwrap();
        CoreConfig::load(tmp.path()).unwrap()
    }

    #[tokio::test]
    async fn from_config_with_embedded_defaults_succeeds() {
        let cfg = embedded_cfg();
        // Embedded defaults declare a non-empty cadence map, so this also
        // exercises the per-source registration branch.
        assert!(!cfg.scheduler.cadence.is_empty());
        let sched = Scheduler::from_config(&cfg).await;
        assert!(sched.is_ok(), "embedded cadence must register cleanly");
    }

    #[tokio::test]
    async fn from_config_with_empty_cadence_succeeds() {
        let mut cfg = embedded_cfg();
        cfg.scheduler.cadence.clear();
        let sched = Scheduler::from_config(&cfg).await;
        assert!(sched.is_ok(), "empty cadence must not be an error");
    }

    #[tokio::test]
    async fn from_config_skips_invalid_cron_without_failing() {
        // A clearly invalid cron string must not abort startup. Other
        // (valid) sources should still register.
        let mut cfg = embedded_cfg();
        cfg.scheduler.cadence.clear();
        cfg.scheduler
            .cadence
            .insert("greenhouse".into(), "0 0 */1 * * *".into());
        cfg.scheduler
            .cadence
            .insert("bogus".into(), "this is not a cron expression".into());
        let sched = Scheduler::from_config(&cfg).await;
        assert!(
            sched.is_ok(),
            "invalid cron in one source must not fail the scheduler",
        );
    }

    #[tokio::test]
    async fn start_then_immediate_shutdown_is_clean() {
        let mut cfg = embedded_cfg();
        cfg.scheduler.cadence.clear();
        cfg.scheduler
            .cadence
            .insert("greenhouse".into(), "0 0 */1 * * *".into());

        let mut sched = Scheduler::from_config(&cfg).await.unwrap();
        sched.start().await.unwrap();
        // Shutdown directly without waiting for a signal — exercises the
        // same drain path `run_until_shutdown` uses.
        let drained = tokio::time::timeout(SHUTDOWN_DRAIN, sched.inner.shutdown()).await;
        assert!(drained.is_ok(), "shutdown should not time out");
    }
}
