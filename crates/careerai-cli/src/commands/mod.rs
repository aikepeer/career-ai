//! Subcommand handlers extracted from `main.rs`.
//!
//! Each module owns the dispatch path for one CLI subcommand. The
//! `Command` enum + clap parsing + `main()` dispatch live in
//! `main.rs`; the actual handlers live here so each file stays under
//! the project's 300-LOC cap.

pub mod llm;
pub mod mcp;
pub mod notify;
