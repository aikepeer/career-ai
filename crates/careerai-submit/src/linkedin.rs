//! LinkedIn Easy Apply browser submitter.
//!
//! Drives a chromiumoxide-launched Chromium through the multi-step
//! Easy Apply modal. Auto-submit is gated by:
//!   1. `SubmitConfig.auto_submit = true` (top-level kill switch)
//!   2. `submit.per_source.linkedin.enabled = true` (per-source switch)
//!   3. Rate limiter permit granted (max_per_day + min-interval +
//!      quiet hours)
//!
//! In dry-run mode the dispatcher (`submit_application`) calls
//! `prepare()` only — no browser, no I/O. The live `submit()` path
//! deliberately stops one click short of the final "Submit application"
//! button. Split into per-concern submodules to stay under the 300-LOC
//! cap.

#![cfg(feature = "browser")]

mod config;
mod submitter;
#[cfg(test)]
mod tests;

pub use config::LinkedinConfig;
pub use submitter::LinkedinSubmitter;
