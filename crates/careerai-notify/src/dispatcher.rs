//! Notification dispatcher: [`Pipeline`] struct, [`Notifier`] trait, and
//! the shared [`HTTP_TIMEOUT`] constant.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::event::NotifyEvent;
use crate::severity::Severity;
use crate::NotifyConfig;
use crate::{channels, NotifyError};

/// HTTP timeout applied uniformly to every webhook channel. Notifications
/// must never block a daemon tick — 5s is the spec ceiling.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// One notification channel. Implementations must NEVER bubble errors;
/// the contract is best-effort. The trait still returns `Result` so the
/// pipeline can choose to log per-channel failures uniformly.
#[async_trait]
pub trait Notifier: Send + Sync + std::fmt::Debug {
    /// Stable identifier (e.g. `"slack"`, `"telegram"`). Used in logs.
    fn name(&self) -> &'static str;

    /// Send `event` at `severity`. Implementations apply a 5s timeout
    /// (`HTTP_TIMEOUT`) to any outbound request.
    async fn notify(&self, event: &NotifyEvent, severity: Severity) -> Result<(), NotifyError>;
}

/// A best-effort fan-out of `Notifier`s plus a minimum severity filter.
///
/// Channels are stored behind `Arc` so each `fire` call can hand a
/// channel handle to `tokio::spawn` without cloning the underlying
/// notifier. Spawning isolates panics: a misbehaving channel impl
/// (e.g. arithmetic overflow inside a custom `Notifier`) returns a
/// `JoinError::Panic` that we log + swallow rather than crashing the
/// daemon tick.
pub struct Pipeline {
    channels: Vec<Arc<dyn Notifier>>,
    min_severity: Severity,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field(
                "channels",
                &self.channels.iter().map(|c| c.name()).collect::<Vec<_>>(),
            )
            .field("min_severity", &self.min_severity)
            .finish()
    }
}

impl Pipeline {
    /// Build a pipeline from config. Each channel is constructed
    /// independently; a misconfigured channel logs a warning and is
    /// skipped rather than failing the whole construction.
    pub fn from_config(cfg: &NotifyConfig) -> Result<Self, NotifyError> {
        let mut channels: Vec<Arc<dyn Notifier>> = Vec::new();

        if let Some(slack) = cfg.channels.slack.as_ref() {
            match channels::slack::SlackNotifier::from_config(slack) {
                Ok(n) => channels.push(Arc::new(n)),
                Err(e) => tracing::warn!(error = %e, "slack channel disabled"),
            }
        }
        if let Some(telegram) = cfg.channels.telegram.as_ref() {
            match channels::telegram::TelegramNotifier::from_config(telegram) {
                Ok(n) => channels.push(Arc::new(n)),
                Err(e) => tracing::warn!(error = %e, "telegram channel disabled"),
            }
        }
        if let Some(email) = cfg.channels.email.as_ref() {
            match channels::email::EmailNotifier::from_config(email) {
                Ok(n) => channels.push(Arc::new(n)),
                Err(e) => tracing::warn!(error = %e, "email channel disabled"),
            }
        }
        if let Some(ntfy) = cfg.channels.ntfy.as_ref() {
            match channels::ntfy::NtfyNotifier::from_config(ntfy) {
                Ok(n) => channels.push(Arc::new(n)),
                Err(e) => tracing::warn!(error = %e, "ntfy channel disabled"),
            }
        }

        Ok(Self {
            channels,
            min_severity: cfg.min_severity,
        })
    }

    /// Build a pipeline directly from a list of notifiers. Used by tests
    /// and by callers that want to wire their own channel set without
    /// going through config.
    #[must_use]
    pub fn with_channels(channels: Vec<Arc<dyn Notifier>>, min_severity: Severity) -> Self {
        Self {
            channels,
            min_severity,
        }
    }

    /// Number of active channels. Useful for sanity checks before a
    /// daemon tick (no channels = nothing to send).
    #[must_use]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Channel names (for diagnostics). Order matches construction.
    #[must_use]
    pub fn channel_names(&self) -> Vec<&'static str> {
        self.channels.iter().map(|c| c.name()).collect()
    }

    /// Best-effort fan-out: every channel runs concurrently, errors are
    /// logged and swallowed so a broken webhook can't break the daemon.
    /// Cheap when no channels are configured (early-returns) or when
    /// `severity < min_severity` (events below the floor are dropped).
    ///
    /// Channels run on isolated `tokio::spawn` tasks, so a panic in a
    /// channel impl is caught as a `JoinError::Panic` and warn-logged
    /// rather than aborting the daemon tick.
    pub async fn fire(&self, event: NotifyEvent, severity: Severity) {
        if severity < self.min_severity {
            tracing::debug!(
                severity = ?severity,
                min = ?self.min_severity,
                title = %event.title(),
                "notify: dropped (below min_severity)",
            );
            return;
        }
        if self.channels.is_empty() {
            tracing::debug!(title = %event.title(), "notify: no channels configured");
            return;
        }

        let title = event.title();
        let event = Arc::new(event);
        let handles: Vec<_> = self
            .channels
            .iter()
            .map(|chan| {
                let chan = Arc::clone(chan);
                let event = Arc::clone(&event);
                let title_for_log = title.clone();
                let name = chan.name();
                tokio::spawn(async move {
                    match chan.notify(&event, severity).await {
                        Ok(()) => {
                            tracing::debug!(
                                channel = name,
                                title = %title_for_log,
                                "notify: delivered",
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                channel = name,
                                error = %e,
                                title = %title_for_log,
                                "notify: channel failed (best-effort, suppressed)",
                            );
                        }
                    }
                })
            })
            .collect();

        for (idx, handle) in handles.into_iter().enumerate() {
            // We don't have the channel name post-spawn (move semantics
            // ate it) but the index lets the operator correlate with
            // `channel_names()` if they need to. Panics get warn-logged
            // here; per-channel errors were already logged inside the
            // task above.
            if let Err(e) = handle.await {
                tracing::warn!(
                    channel_index = idx,
                    error = %e,
                    "notify: channel task panicked (best-effort, suppressed)",
                );
            }
        }
    }
}
