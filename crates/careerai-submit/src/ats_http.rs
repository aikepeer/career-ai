//! ATS HTTP submitters (Greenhouse, Lever, Ashby).
//!
//! Each submitter's `prepare()` builds a provider-specific URL + a
//! realistic candidate payload. The live `submit()` currently
//! short-circuits with `SourceDisabled` — that needs an API-key path
//! wired in a follow-up (or the browser flow in M5).
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod submitters;
mod support;
#[cfg(test)]
mod tests;

pub use submitters::{AshbySubmitter, GreenhouseSubmitter, LeverSubmitter};
#[cfg(any(test, feature = "browser"))]
pub(crate) use support::sanitize_external_id;
