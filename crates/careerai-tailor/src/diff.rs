//! Diff-based resume tailoring safety boundary.
//!
//! This is the non-negotiable safety boundary: LLM output can only
//! reorder or reword existing profile bullets. It cannot invent
//! experience, titles, dates, employers, or numbers. Nine validator
//! rules enforce that boundary; see [`validate`] below.
//!
//! ## Module layout
//!
//! * [`schema`] — [`DiffDoc`], [`DiffOp`], [`OpKind`], [`SummaryOp`],
//!   [`BulletPath`], [`Section`], path regex, parse/format helpers.
//! * [`parse`] — profile-walking helpers: enumerate bullet paths,
//!   look up original bullet text, count bullets per entry.
//! * [`validate`] — the nine-rule validator with entity guardrails.
//! * [`apply`] — apply a validated `DiffDoc` to produce a
//!   [`ResumeView`](crate::model::ResumeView).

mod apply;
mod parse;
mod schema;
#[cfg(test)]
mod tests;
mod validate;

pub use apply::apply;
pub use schema::DiffDoc;
#[cfg(test)]
pub(crate) use schema::{BulletPath, DiffOp, OpKind, SummaryOp};
pub use validate::{validate, validate_and_sanitize};
