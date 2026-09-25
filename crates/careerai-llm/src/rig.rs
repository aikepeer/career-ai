//! Real provider impl of [`Llm`] via rig-core. Supports Anthropic and
//! OpenAI. Gated behind the `live-llm-api` Cargo feature.
//!
//! ## Module layout
//!
//! * [`driver`] — [`RigLlm`] struct, [`Provider`] enum, body builders,
//!   [`Llm`] trait impl.
//! * [`response`] — Anthropic and OpenAI JSON response parsers.
//! * [`error`] — retry classifier and error-mapping helpers.

mod bodies;
mod driver;
mod error;
mod response;
#[cfg(test)]
mod tests;

pub use driver::{Provider, RigLlm};
