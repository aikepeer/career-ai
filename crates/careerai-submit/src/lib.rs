//! Application submitters.
//!
//! Each channel implements `Submitter`. Dry-run is the default.
//! Real submission gated by `auto_submit = true` AND the per-source
//! `submit_enabled` flag, both enforced here.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

#![forbid(unsafe_code)]

pub mod ats_http;
pub mod base;
#[cfg(feature = "browser")]
pub mod browser_session;
pub mod credentials;
pub mod dry_run;
pub mod error;
#[cfg(feature = "browser")]
pub mod linkedin;
pub mod linkedin_selectors;
#[cfg(feature = "browser")]
pub mod naukri;
pub mod naukri_selectors;
pub mod rate_limiter;
mod submit;

pub use ats_http::{AshbySubmitter, GreenhouseSubmitter, LeverSubmitter};
pub use base::{SubmitContext, SubmitDecision, SubmitOutcome, Submitter, WouldSubmit};
#[cfg(feature = "browser")]
pub use browser_session::{stealth_script_sha256, BrowserSession, BrowserSessionConfig};
pub use credentials::Credential;
pub use dry_run::DryRunSubmitter;
pub use error::{Result, SubmitError};
#[cfg(feature = "browser")]
pub use linkedin::{LinkedinConfig, LinkedinSubmitter};
#[cfg(feature = "browser")]
pub use naukri::{NaukriConfig, NaukriSubmitter};
pub use rate_limiter::{RateLimitError, RateLimiter, RatePermit, RatePolicy};
pub use submit::submit_application;
