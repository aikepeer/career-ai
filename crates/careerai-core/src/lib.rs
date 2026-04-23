//! Core orchestration for `career-ai`.
//!
//! Owns the pipeline state machine, configuration loading, and the shared
//! types that other crates depend on. No I/O beyond config + init scaffolding.

pub mod init;
pub mod state;
