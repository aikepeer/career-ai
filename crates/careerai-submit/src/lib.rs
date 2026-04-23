//! Application submitters.
//!
//! Each channel (ATS HTTP, `LinkedIn`, `Indeed`, email) implements the
//! `Submitter` trait. The dry-run wrapper is default and must never issue
//! a network write — it screenshots + logs a `would_submit` event instead.
//! Per-source rate limits enforced via `governor` at the boundary.
//! Implemented in M4/M5.

pub mod base;
