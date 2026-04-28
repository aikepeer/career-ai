//! Notification errors. All variants are best-effort — `Pipeline::fire`
//! logs and swallows rather than bubbling.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NotifyError {
    /// Per-channel failure; carries the channel name so the warn-log
    /// line is greppable.
    #[error("notify channel `{channel}` failed: {reason}")]
    Channel {
        channel: &'static str,
        reason: String,
    },
    /// HTTP layer failure (timeout, DNS, TLS). Distinct from a channel
    /// returning a non-2xx response — that maps to `Channel`.
    #[error("notify http failure: {0}")]
    Http(String),
    /// Config secret not resolvable from the env var or keyring entry
    /// referenced. Logged once at startup; the channel is then disabled.
    #[error("notify secret missing: {0}")]
    SecretMissing(String),
    /// Generic config error (malformed value, empty required field).
    #[error("notify config error: {0}")]
    Config(String),
}
