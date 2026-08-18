//! ATS HTTP submitters (Greenhouse, Lever, Ashby).
//!
//! Each submitter's `prepare()` builds a provider-specific URL + a
//! realistic candidate payload. The live `submit()` POSTs that payload to
//! the public ATS endpoint (no candidate-facing API key), so treat the
//! field shapes as best-effort until a captured submission is verified.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod smartrecruiters;
mod submitters;
mod support;
mod teamtailor;
#[cfg(test)]
mod tests;

pub use smartrecruiters::SmartRecruitersSubmitter;
pub use submitters::{AshbySubmitter, GreenhouseSubmitter, LeverSubmitter};
#[cfg(any(test, feature = "browser"))]
pub(crate) use support::sanitize_external_id;
pub use teamtailor::TeamtailorSubmitter;
