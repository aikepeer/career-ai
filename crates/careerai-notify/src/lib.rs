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
//!
//! ## Module layout
//!
//! * [`event`] — [`NotifyEvent`] enum and its `title()`/`summary()` helpers.
//! * [`dispatcher`] — [`Pipeline`] struct, [`Notifier`] trait,
//!   [`HTTP_TIMEOUT`] constant.
//! * [`channels`] — Slack, Telegram, email, ntfy notifier implementations.
//! * [`severity`] — [`Severity`] enum.
//! * [`config`] — [`NotifyConfig`] and per-channel config types.
//! * [`error`] — [`NotifyError`] types.

#![allow(clippy::module_name_repetitions)]

pub mod channels;
mod config;
mod dispatcher;
mod error;
mod event;
mod severity;
#[cfg(test)]
mod tests;

pub use config::{
    EmailConfig, NotifyChannels, NotifyConfig, NtfyConfig, SlackConfig, TelegramConfig,
};
pub use dispatcher::{Notifier, Pipeline, HTTP_TIMEOUT};
pub use error::NotifyError;
pub use event::NotifyEvent;
pub use severity::Severity;
