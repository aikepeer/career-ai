//! Graceful-shutdown logic — `run_until_shutdown`.
//! Kept separate so `lib.rs` stays under the 300-LOC cap.

#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info, info_span, warn, Instrument};

use crate::error::{SchedulerError, SHUTDOWN_DRAIN};
use crate::Scheduler;

impl Scheduler {
    /// Run until SIGINT or SIGTERM, then gracefully shut down.
    ///
    /// On signal we call `JobScheduler::shutdown()` and wait up to
    /// `SHUTDOWN_DRAIN` for in-flight jobs to settle. If the drain times out
    /// we still return `Ok(())` — the daemon's contract is "exit promptly on
    /// Ctrl-C", not "block until every job finishes".
    pub async fn run_until_shutdown(mut self) -> Result<(), SchedulerError> {
        let span = info_span!("scheduler");
        async move {
            // H2: install signal handlers BEFORE starting the scheduler.
            // `tokio::signal::ctrl_c` registers lazily on first poll, and on
            // Unix the `signal()` syscall itself happens at construction —
            // any signal delivered between `start()` returning and the first
            // `select!` poll could otherwise fall through to the default
            // disposition (terminate without drain). Registering both here
            // closes that window: the kernel queues SIGINT/SIGTERM into our
            // handlers as soon as `signal()` returns.
            //
            // On Windows there is no `tokio::signal::unix`; the daemon
            // shuts down on Ctrl-C only (operator-driven). systemd-style
            // SIGTERM doesn't apply on Windows.
            #[cfg(unix)]
            {
                let mut sigint = signal(SignalKind::interrupt())?;
                let mut sigterm = signal(SignalKind::terminate())?;

                self.start().await?;

                // SIGTERM (e.g. systemd stop) and SIGINT (Ctrl-C) both
                // initiate graceful shutdown. Race them with
                // `tokio::select!` so whichever arrives first wins.
                tokio::select! {
                    _ = sigint.recv() => {
                        info!("received SIGINT, shutting down");
                    }
                    _ = sigterm.recv() => {
                        info!("received SIGTERM, shutting down");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                self.start().await?;

                tokio::signal::ctrl_c().await?;
                info!("received Ctrl-C, shutting down");
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
