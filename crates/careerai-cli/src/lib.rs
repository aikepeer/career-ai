//! Library surface of the `careerai` CLI binary.
//!
//! Only exists so integration tests under `tests/` can drive the same
//! pipeline functions `main.rs` dispatches into. The binary itself
//! continues to own argument parsing + exit-code mapping.

pub mod pipeline;
