//! Raw resume text → structured [`Profile`] via heuristic section splitting.
//!
//! Resumes rarely parse cleanly. This module extracts the boring,
//! high-confidence bits (contact info, section blocks) and leaves the rest
//! as free text the user is expected to polish. Not a silver bullet — the
//! generated YAML is a seed, not a final artifact.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod parse;
mod regex;
#[cfg(test)]
mod tests;
mod text;

#[cfg(test)]
pub(crate) use parse::classify_header;
pub use parse::parse;
