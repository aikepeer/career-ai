//! Job-board discovery.
//!
//! Each adapter implements the `Source` trait and returns normalized
//! `Listing` DTOs. New boards plug in by implementing `Source`.
//! Adapters implemented in M2+.

pub mod base;
