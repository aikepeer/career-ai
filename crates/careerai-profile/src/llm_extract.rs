//! LLM-backed resume → [`Profile`] extractor.
//!
//! The heuristic regex parser in [`crate::heuristic`] produces garbage on
//! the kind of free-form PDF text most resumes ship with. This module
//! replaces that path with a constrained LLM call: send the raw text plus a
//! strict JSON-only schema preamble and parse the response into a
//! [`Profile`]. On schema-validation failure we retry once with the
//! validator error appended; two failures produce
//! [`ExtractError::MaxRetries`] so the caller can fall back.
//!
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod build;
mod parse;
#[cfg(test)]
mod tests;
pub(crate) mod types;

pub use build::{build_request, build_schema_preamble, build_system_message, build_user_message};
pub use parse::{extract_profile_from_text, parse_and_validate, strip_code_fences};
pub use types::{
    ExtractError, ExtractOptions, ExtractRequest, LlmCaller, DEFAULT_MODEL, DEFAULT_PROMPT_VERSION,
};
