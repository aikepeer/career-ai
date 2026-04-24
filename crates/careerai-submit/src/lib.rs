//! Application submitters.
//!
//! Each channel implements `Submitter`. Dry-run is the default.
//! Real submission gated by `auto_submit = true` AND the per-source
//! `submit_enabled` flag, both enforced here.

#![forbid(unsafe_code)]

pub mod ats_http;
pub mod base;
pub mod dry_run;
pub mod error;

pub use ats_http::{AshbySubmitter, GreenhouseSubmitter, LeverSubmitter};
pub use base::{SubmitContext, SubmitDecision, SubmitOutcome, Submitter, WouldSubmit};
pub use dry_run::DryRunSubmitter;
pub use error::{Result, SubmitError};

use std::path::Path;

use careerai_core::config::SubmitConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_db::SqlitePool;
use careerai_profile::Profile;
use tracing::{info, warn};

/// Submit a prepared application. Routes to the correct per-source
/// `Submitter` based on the listing's `source` column. Honors dry-run
/// and per-source `enabled` flags.
///
/// Transitions `applications.state` and the linked `listings.state`:
/// - on real success: both → `submitted`; writes a `"submitted via {source}"`
///   event on the listing.
/// - on dry-run success: neither state changes; writes a `would_submit`
///   tracing event (no listing event — the run never intended to advance).
/// - on skip (source disabled / gated): application → `skipped`,
///   listing → `skipped`; event note `"skipped: source disabled"`.
/// - on failure: application → `failed`, listing → `failed`; event note
///   carries the error.
///
/// `root` is the project root used to resolve `profile/profile.yaml`.
/// Accepting it here (rather than wedging another field into
/// `SubmitConfig`) mirrors `careerai-cli::pipeline::tailor_one` which
/// already threads `root` through the same call sites.
pub async fn submit_application(
    pool: &SqlitePool,
    cfg: &SubmitConfig,
    root: &Path,
    application_id: &str,
) -> Result<SubmitOutcome> {
    // 1. Load the application and assert it's ready to submit.
    let application = queries::find_application_by_id(pool, application_id).await?;
    let state_str = application.state.as_str();
    if state_str != ListingState::Rendered.as_str() && state_str != ListingState::Prepared.as_str()
    {
        return Err(SubmitError::BadState {
            state: application.state.clone(),
        });
    }

    // 2. Load the listing + artifacts + payload + profile.
    let listing = queries::find_by_id(pool, &application.listing_id).await?;
    let artifacts = queries::list_artifacts(pool, &application.id).await?;
    let payload = queries::find_payload_by_application_id(pool, &application.id).await?;
    let profile = load_profile(root)?;

    let ctx = SubmitContext {
        application: &application,
        listing: &listing,
        profile: &profile,
        artifacts: &artifacts,
        cover_letter_text: &payload.cover_letter_text,
    };

    // 3. Resolve the submitter for this source. Unknown → hard error.
    let source = listing.source.as_str();
    let submitter: Box<dyn Submitter> = match source {
        "greenhouse" => Box::new(GreenhouseSubmitter::new()),
        "lever" => Box::new(LeverSubmitter::new()),
        "ashby" => Box::new(AshbySubmitter::new()),
        other => return Err(SubmitError::UnknownSource(other.to_owned())),
    };

    // 4. Per-source gating. Unknown source in the map => treated as
    // disabled. Callers opt in explicitly.
    let per_source_enabled = cfg.per_source.get(source).is_some_and(|s| s.enabled);
    if !per_source_enabled {
        let reason = "source disabled";
        info!(
            target: "submit",
            source,
            application_id = %application.id,
            "skipping — source not enabled in submit.per_source"
        );
        queries::set_application_state(pool, &application.id, ListingState::Skipped.as_str())
            .await?;
        queries::transition(
            pool,
            &listing.id,
            ListingState::Skipped,
            Some("skipped: source disabled"),
        )
        .await?;
        return Ok(SubmitOutcome::Skipped {
            reason: reason.to_owned(),
        });
    }

    // 5. Choose live vs dry-run.
    let decision = if cfg.auto_submit {
        SubmitDecision::Live
    } else {
        SubmitDecision::DryRun
    };

    match decision {
        SubmitDecision::Live => run_live(pool, submitter.as_ref(), &ctx).await,
        SubmitDecision::DryRun => {
            let wrapper = DryRunSubmitter::new(BoxedSubmitter(submitter));
            run_dry_run(&wrapper, &ctx).await
        }
    }
}

async fn run_live(
    pool: &SqlitePool,
    submitter: &dyn Submitter,
    ctx: &SubmitContext<'_>,
) -> Result<SubmitOutcome> {
    match submitter.submit(ctx).await {
        Ok(remote_id) => {
            info!(
                target: "submit",
                source = submitter.name(),
                application_id = %ctx.application.id,
                remote_id = %remote_id,
                "submitted"
            );
            queries::set_application_state(
                pool,
                &ctx.application.id,
                ListingState::Submitted.as_str(),
            )
            .await?;
            queries::transition(
                pool,
                &ctx.listing.id,
                ListingState::Submitted,
                Some(&format!("submitted via {}", submitter.name())),
            )
            .await?;
            Ok(SubmitOutcome::Submitted { remote_id })
        }
        Err(err) => {
            warn!(
                target: "submit",
                source = submitter.name(),
                application_id = %ctx.application.id,
                error = %err,
                "submission failed"
            );
            // Best-effort: record the failure, but always return the
            // original error.
            let note = format!("failed: {err}");
            let _ = queries::set_application_state(
                pool,
                &ctx.application.id,
                ListingState::Failed.as_str(),
            )
            .await;
            let _ =
                queries::transition(pool, &ctx.listing.id, ListingState::Failed, Some(&note)).await;
            Err(err)
        }
    }
}

async fn run_dry_run<S: Submitter>(
    wrapper: &DryRunSubmitter<S>,
    ctx: &SubmitContext<'_>,
) -> Result<SubmitOutcome> {
    // `prepare` is pure — we call it for the payload summary too.
    let would = wrapper.prepare(ctx)?;
    let _ = wrapper.submit(ctx).await?;
    Ok(SubmitOutcome::DryRun {
        payload_summary: format!(
            "{} {} (body {} bytes, artifacts {:?})",
            would.method,
            would.url,
            would.body_preview.len(),
            would.artifact_kinds
        ),
    })
}

/// Tiny adapter so a `Box<dyn Submitter>` can be moved into
/// `DryRunSubmitter<S: Submitter>` without loosening that bound to
/// `?Sized`. Forwards every call.
struct BoxedSubmitter(Box<dyn Submitter>);

#[async_trait::async_trait]
impl Submitter for BoxedSubmitter {
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn prepare(&self, ctx: &SubmitContext<'_>) -> Result<WouldSubmit> {
        self.0.prepare(ctx)
    }
    async fn submit(&self, ctx: &SubmitContext<'_>) -> Result<String> {
        self.0.submit(ctx).await
    }
}

impl std::fmt::Debug for BoxedSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoxedSubmitter")
            .field("name", &self.0.name())
            .finish()
    }
}

fn load_profile(root: &Path) -> Result<Profile> {
    let path = root.join("profile").join("profile.yaml");
    let text = std::fs::read_to_string(&path).map_err(SubmitError::Io)?;
    Profile::from_yaml(&text).map_err(|e| {
        // Surface profile-parse errors as IO-ish since the submit layer
        // doesn't have a dedicated profile-error variant and the
        // operator action is the same: fix the YAML.
        SubmitError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("parse {}: {e}", path.display()),
        ))
    })
}
