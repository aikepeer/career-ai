//! Best-effort multi-channel notification pipeline.
//!
//! Wires the human-in-the-loop signals — cookie expiry, source failures,
//! manual review needed, high-score matches, rate-limit exhaustion,
//! application responses — out to whichever channels the operator has
//! configured (Slack, Telegram, email, ntfy). Channels are independent
//! and best-effort: a single broken webhook NEVER bubbles up and breaks
//! a daemon tick. Each channel logs its own failure via `tracing::warn`.
//!
//! The `Pipeline` itself is cheap to construct and even cheaper to fire
//! when no channels are configured (`fire` early-returns under
//! `min_severity` or with an empty channel list).
//!
//! Secrets pattern: configs reference `*_env` (read at load time from
//! the process env) or `*_keyring` (read from the OS keyring); inline
//! secrets in YAML are explicitly NOT supported. See `NOTIFICATIONS.md`
//! for the operator-facing setup guide.

#![allow(clippy::module_name_repetitions)]

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod channels;
mod config;
mod error;
mod severity;

pub use config::{
    EmailConfig, NotifyChannels, NotifyConfig, NtfyConfig, SlackConfig, TelegramConfig,
};
pub use error::NotifyError;
pub use severity::Severity;

/// HTTP timeout applied uniformly to every webhook channel. Notifications
/// must never block a daemon tick — 5s is the spec ceiling.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// Events fired by the pipeline. Adding a new variant is non-breaking:
/// channels render via the `Display`-shaped `summary()` + `title()`
/// helpers below, so unknown variants degrade to a generic line rather
/// than failing the build of every channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotifyEvent {
    /// LinkedIn `li_at` cookie within 48h of expiry. Detected by
    /// `careerai digest`.
    CookieExpiringSoon { provider: String, hours_left: u64 },
    /// A discovery source returned an error mid-run (HTTP failure, DNS,
    /// auth, etc.).
    SourceUnreachable { source: String, reason: String },
    /// Cookie or token-based auth failed during a discovery / submit
    /// pass — distinct from a transport failure because the recovery is
    /// always "refresh the credential."
    AuthFailureMidRun { provider: String, action: String },
    /// Submitter encountered an unknown form field; operator must
    /// review the captured screenshot.
    ManualReviewNeeded {
        application_id: String,
        reason: String,
        screenshot_path: Option<PathBuf>,
    },
    /// Match score above the per-config threshold — same-day applies
    /// often pay off in this band.
    HighScoreMatch {
        listing_id: String,
        title: String,
        company: String,
        score: f32,
    },
    /// Governor permit denied because the daily cap is exhausted or the
    /// quiet-hours window is active. `next_window_seconds` is `0` when
    /// unknown.
    RateLimitExhausted {
        source: String,
        next_window_seconds: u64,
    },
    /// (M7 — wired in a future PR) the application got a response from
    /// the ATS / recruiter; classification is "reply" / "rejection" /
    /// "interview" / "unknown".
    ApplicationResponded {
        application_id: String,
        classification: String,
    },
}

impl NotifyEvent {
    /// Short human-readable title used by every channel.
    #[must_use]
    pub fn title(&self) -> String {
        match self {
            Self::CookieExpiringSoon { provider, .. } => {
                format!("[career-ai] {provider} cookie expiring soon")
            }
            Self::SourceUnreachable { source, .. } => {
                format!("[career-ai] source unreachable: {source}")
            }
            Self::AuthFailureMidRun { provider, .. } => {
                format!("[career-ai] auth failure mid-run: {provider}")
            }
            Self::ManualReviewNeeded { application_id, .. } => {
                format!("[career-ai] manual review needed: {application_id}")
            }
            Self::HighScoreMatch { title, company, .. } => {
                format!("[career-ai] high-score match: {title} @ {company}")
            }
            Self::RateLimitExhausted { source, .. } => {
                format!("[career-ai] rate-limit exhausted: {source}")
            }
            Self::ApplicationResponded {
                application_id,
                classification,
                ..
            } => format!("[career-ai] application {application_id} -> {classification}"),
        }
    }

    /// Plain-text body used by every channel that doesn't use a
    /// channel-native rich format.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::CookieExpiringSoon {
                provider,
                hours_left,
            } => format!(
                "{provider} cookie expires in ~{hours_left}h. Run \
                 `careerai cookies refresh {provider}` to renew."
            ),
            Self::SourceUnreachable { source, reason } => {
                format!("Discovery source {source} failed: {reason}")
            }
            Self::AuthFailureMidRun { provider, action } => format!(
                "Auth failure for {provider} during {action}. \
                 Refresh the credential and re-run."
            ),
            Self::ManualReviewNeeded {
                application_id,
                reason,
                screenshot_path,
            } => match screenshot_path {
                Some(p) => format!(
                    "Application {application_id} needs manual review: \
                     {reason}. Screenshot: {}",
                    p.display()
                ),
                None => format!("Application {application_id} needs manual review: {reason}"),
            },
            Self::HighScoreMatch {
                listing_id,
                title,
                company,
                score,
            } => format!(
                "High-score match {score:.2}: {title} @ {company} \
                 (listing {listing_id}). Same-day applies tend to convert."
            ),
            Self::RateLimitExhausted {
                source,
                next_window_seconds,
            } => {
                if *next_window_seconds == 0 {
                    format!("Rate-limit exhausted for {source}.")
                } else {
                    format!(
                        "Rate-limit exhausted for {source}. Next window in \
                         ~{next_window_seconds}s."
                    )
                }
            }
            Self::ApplicationResponded {
                application_id,
                classification,
            } => format!("Application {application_id} got response: {classification}."),
        }
    }
}

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
    channels: Vec<std::sync::Arc<dyn Notifier>>,
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
        use std::sync::Arc;
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
    pub fn with_channels(
        channels: Vec<std::sync::Arc<dyn Notifier>>,
        min_severity: Severity,
    ) -> Self {
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
        let event = std::sync::Arc::new(event);
        let handles: Vec<_> = self
            .channels
            .iter()
            .map(|chan| {
                let chan = std::sync::Arc::clone(chan);
                let event = std::sync::Arc::clone(&event);
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Debug)]
    struct CountingNotifier {
        name: &'static str,
        delivered: Arc<AtomicUsize>,
        delay_ms: u64,
        fail: bool,
    }

    #[async_trait]
    impl Notifier for CountingNotifier {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn notify(
            &self,
            _event: &NotifyEvent,
            _severity: Severity,
        ) -> Result<(), NotifyError> {
            if self.delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            }
            if self.fail {
                return Err(NotifyError::Channel {
                    channel: self.name,
                    reason: "synthetic failure".into(),
                });
            }
            self.delivered.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn ev() -> NotifyEvent {
        NotifyEvent::SourceUnreachable {
            source: "test".into(),
            reason: "boom".into(),
        }
    }

    #[tokio::test]
    async fn fire_runs_all_channels_concurrently_and_swallows_errors() {
        let ok = Arc::new(AtomicUsize::new(0));
        let slow_ok = Arc::new(AtomicUsize::new(0));
        let pipe = Pipeline::with_channels(
            vec![
                Arc::new(CountingNotifier {
                    name: "ok",
                    delivered: ok.clone(),
                    delay_ms: 0,
                    fail: false,
                }),
                Arc::new(CountingNotifier {
                    name: "broken",
                    delivered: Arc::new(AtomicUsize::new(0)),
                    delay_ms: 0,
                    fail: true,
                }),
                Arc::new(CountingNotifier {
                    name: "slow",
                    delivered: slow_ok.clone(),
                    delay_ms: 50,
                    fail: false,
                }),
            ],
            Severity::Info,
        );
        // Should not bubble despite the failing channel.
        pipe.fire(ev(), Severity::Warning).await;
        assert_eq!(ok.load(Ordering::SeqCst), 1);
        assert_eq!(slow_ok.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fire_drops_events_below_min_severity() {
        let ok = Arc::new(AtomicUsize::new(0));
        let pipe = Pipeline::with_channels(
            vec![Arc::new(CountingNotifier {
                name: "ok",
                delivered: ok.clone(),
                delay_ms: 0,
                fail: false,
            })],
            Severity::Critical,
        );
        pipe.fire(ev(), Severity::Info).await;
        pipe.fire(ev(), Severity::Warning).await;
        assert_eq!(ok.load(Ordering::SeqCst), 0);
        pipe.fire(ev(), Severity::Critical).await;
        assert_eq!(ok.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fire_is_cheap_with_no_channels() {
        let pipe = Pipeline::with_channels(vec![], Severity::Info);
        // Just shouldn't panic / deadlock.
        pipe.fire(ev(), Severity::Critical).await;
        assert_eq!(pipe.channel_count(), 0);
    }

    /// A panicking channel must NOT crash the daemon tick: the spawned
    /// task is isolated, the JoinError gets warn-logged, sibling
    /// channels still deliver.
    #[derive(Debug)]
    struct PanickingNotifier;

    #[async_trait]
    impl Notifier for PanickingNotifier {
        fn name(&self) -> &'static str {
            "panicker"
        }
        async fn notify(
            &self,
            _event: &NotifyEvent,
            _severity: Severity,
        ) -> Result<(), NotifyError> {
            panic!("synthetic panic inside notifier");
        }
    }

    #[tokio::test]
    async fn fire_swallows_channel_panics() {
        let ok = Arc::new(AtomicUsize::new(0));
        let pipe = Pipeline::with_channels(
            vec![
                Arc::new(PanickingNotifier),
                Arc::new(CountingNotifier {
                    name: "ok",
                    delivered: ok.clone(),
                    delay_ms: 0,
                    fail: false,
                }),
            ],
            Severity::Info,
        );
        // Must not propagate the panic; sibling channel still delivers.
        pipe.fire(ev(), Severity::Critical).await;
        assert_eq!(ok.load(Ordering::SeqCst), 1);
    }
}
