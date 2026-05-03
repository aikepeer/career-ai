//! Hard pass/reject filters applied before scoring.
//!
//! Checks are ordered cheap-first: title exclusions (short string) →
//! location allowlist → JD keyword exclusions (longer string scan) →
//! domain keywords + required-keyword rule.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod classify;
#[cfg(test)]
mod tests;

pub use classify::{apply_must_include_filter, classify, Decision};
