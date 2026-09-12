//! Hosted multi-tenant API surface for career-ai.
//!
//! Implements the hosted subscription product per the design document
//! (`docs/HOSTED_PRODUCT_DESIGN.md`). PRs 2-7 add:
//! - Authentication and workspace authorization (PR 2)
//! - PostgreSQL schema with RLS and context propagation (PR 3)
//! - Entitlements, billing, durable jobs, audit, action protocol (PR 4+5)
//! - Read-only vertical slice (PR 6)
//! - Review-only tailored artifacts and manual outcomes (PR 7)

pub mod action;
pub mod api;
pub mod audit;
pub mod auth;
pub mod classify;
pub mod db;
pub mod entitlement;
pub mod export;
pub mod jobs;
pub mod ops;
pub mod workers;

pub use careerai_core;
