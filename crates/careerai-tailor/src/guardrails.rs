//! Entity-level reword guardrails.
//!
//! The safety invariant: LLM rewords may not invent employers, numbers,
//! years, or proper nouns that don't appear in the profile (or,
//! narrowly, in the original bullet). Violations surface as
//! `TailorError::InventedContent` with the exact offending token so logs
//! and tests can show users what fired and why.
//!
//! ## Module layout
//!
//! * [`common_caps`] — curated whitelist of common sentence-start English words.
//! * [`tokens`] — regex factories, [`ProfileTokenSets`], builder, text flatteners.
//! * [`validator`] — the core guardrail functions.

mod common_caps;
#[cfg(test)]
mod tests;
mod tokens;
mod validator;

pub use tokens::build_jd_token_sets;
pub(crate) use tokens::{build_token_sets, flat_profile_text};
pub use validator::forbid_invented_entities;
pub(crate) use validator::forbid_invented_entities_with;
