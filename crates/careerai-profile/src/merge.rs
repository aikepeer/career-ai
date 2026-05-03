//! Merge multiple parsed [`Profile`]s into one canonical profile.
//!
//! Merge order: earlier profiles are the base, later profiles **fill gaps and
//! contribute additional list items** — they do NOT overwrite non-empty scalar
//! fields on the base. This makes the merge commutative where it matters
//! (list concatenation + dedupe) and deterministic where it doesn't (first
//! source wins for scalars). Summary is the one exception: longer wins.
//!
//! De-duplication:
//! - Experience: by `(company_lc, title_lc, start)` — first seen wins; later
//!   duplicates donate missing bullets/location/end.
//! - Education: by `(institution_lc, degree_lc)`.
//! - Projects: by `name_lc`.
//! - Skills: case-preserving, order-preserving, dedup by lowercase.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod core;
mod keys;
#[cfg(test)]
mod tests;
mod utils;

pub use core::{merge_all, merge_pair};
#[cfg(test)]
pub(crate) use keys::looks_like_date_garbage;
