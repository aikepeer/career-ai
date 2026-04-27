//! The `Submitter` trait. All apply channels implement this.
//!
//! Dry-run is the default. Real submission gated by `auto_submit = true`
//! AND the per-source `submit_enabled` flag, both enforced at the
//! `submit_application` entry point in `lib.rs`.

use async_trait::async_trait;
use careerai_db::models::{Application, Artifact, Listing};
use careerai_profile::schema::Profile;

use crate::error::Result;

/// A prepared application as presented to a `Submitter`. Owned by the
/// caller; submitters borrow.
#[derive(Debug, Clone)]
pub struct SubmitContext<'a> {
    pub application: &'a Application,
    pub listing: &'a Listing,
    pub profile: &'a Profile,
    pub artifacts: &'a [Artifact],
    pub cover_letter_text: &'a str,
}

/// Outcome of one `submit_application` call.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SubmitOutcome {
    /// Real submission succeeded. `remote_id` is whatever the ATS echoes
    /// back (a candidate id, an application id, etc.) — used to
    /// correlate follow-ups at M7.
    Submitted { remote_id: String },
    /// Dry-run logged a `would_submit` event but sent no bytes AND made
    /// no state change. Distinct from `Drafted`, which writes Drafted
    /// to the DB but defers the network click to `careerai review`.
    DryRun { payload_summary: String },
    /// LinkedIn assist mode: the application was transitioned to
    /// `Drafted` and is awaiting operator confirmation in
    /// `careerai review`. State HAS changed; no network submission yet.
    Drafted { note: String },
    /// Skipped before even attempting (source disabled, gated off).
    Skipped { reason: String },
}

/// A structured request the per-source submitter would POST (in real
/// mode) or log (in dry-run mode). The dry-run wrapper inspects this to
/// write a deterministic, redacted event.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WouldSubmit {
    pub source: &'static str,
    pub url: String,
    pub method: &'static str,
    /// First 1 KB of the serialized body. Truncated deterministically so
    /// the event log is bounded regardless of payload size.
    pub body_preview: String,
    pub artifact_kinds: Vec<String>,
}

/// Classifier passed between the router and the submitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitDecision {
    DryRun,
    Live,
}

#[async_trait]
pub trait Submitter: Send + Sync {
    fn name(&self) -> &'static str;

    /// Build the `WouldSubmit` for this context — pure function, no I/O.
    /// Inspected by both the dry-run and live paths.
    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit>;

    /// Actually POST. Only called when `decision == Live`. Returns the
    /// remote id the ATS assigns to the application.
    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String>;
}
