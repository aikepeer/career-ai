//! Error and constant types for the scheduler crate.

use std::time::Duration;

use tokio_cron_scheduler::JobSchedulerError;

/// Time the scheduler is allowed to drain after a shutdown signal before we
/// give up and return anyway. Kept short so `careerai daemon` always exits
/// promptly under operator Ctrl-C.
pub(crate) const SHUTDOWN_DRAIN: Duration = Duration::from_secs(10);

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
