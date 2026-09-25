//! Core orchestration for `career-ai`.
//!
//! Owns the pipeline state machine, configuration loading, and the shared
//! types that other crates depend on. No I/O beyond config + init scaffolding.

pub mod config;
pub mod init;
pub mod paths;
pub mod salary;
pub mod state;
