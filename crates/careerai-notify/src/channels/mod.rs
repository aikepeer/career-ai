//! Built-in notifier channels. Each module is independent and gated
//! only by its config block. None of these channels expose secrets in
//! logs (asserted via `tests/secret_redaction_it.rs`).

pub mod email;
pub mod ntfy;
pub mod slack;
pub mod telegram;
