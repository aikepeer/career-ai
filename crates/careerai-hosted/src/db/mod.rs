//! Hosted database: PostgreSQL schema, RLS, context propagation (PR 3).
//!
//! Application roles: careerai_api (no bypass RLS), careerai_worker (no bypass
//! RLS), careerai_webhook (no bypass RLS), careerai_migrator (DDL only).
//! Every transaction begins by authenticating an internal signed context and
//! executing `SET LOCAL app.tenant_id`, `app.actor_id`, `app.job_id`, and
//! `app.access_reason`.

pub mod context;
pub mod migrations;

pub use context::{ContextError, SignedContext, TenantContext};
