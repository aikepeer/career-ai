use std::path::Path;

use careerai_core::config::SubmitConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_db::SqlitePool;
use careerai_profile::Profile;
use tracing::{info, warn};

use crate::ats_http::{AshbySubmitter, GreenhouseSubmitter, LeverSubmitter};
use crate::base::{SubmitContext, SubmitDecision, SubmitOutcome, Submitter};
use crate::dry_run::DryRunSubmitter;
use crate::error::{Result, SubmitError};

/// Process-wide rate limiter shared across every `submit_application`
/// call. Per-source counters (day-cap, min-interval bucket) live here,
/// not in a per-call instance, so two concurrent submits coordinate.
/// Built lazily on first access; no state is persisted across process
/// restarts (counters reset, which is the desired behavior at this
/// scale).
///
/// The function itself is `#[cfg(feature = "browser")]` because today
/// only `LinkedinSubmitter` consumes it. The `rate_limiter` module is
/// always-on (its deps are non-optional in `careerai-submit`); when
/// the next browser submitter lands (Indeed in M5b), the gate stays
/// in the same place.
#[cfg(feature = "browser")]
fn shared_rate_limiter() -> std::sync::Arc<crate::rate_limiter::RateLimiter> {
    static RL: std::sync::OnceLock<std::sync::Arc<crate::rate_limiter::RateLimiter>> =
        std::sync::OnceLock::new();
    RL.get_or_init(|| std::sync::Arc::new(crate::rate_limiter::RateLimiter::new()))
        .clone()
}

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
    let application = queries::find_application_by_id(pool, application_id).await?;
    let state_str = application.state.as_str();
    if state_str != ListingState::Rendered.as_str()
        && state_str != ListingState::Prepared.as_str()
        && state_str != ListingState::Drafted.as_str()
    {
        return Err(SubmitError::BadState {
            state: application.state.clone(),
        });
    }

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

    let source_lc = listing.source.to_ascii_lowercase();

    // Check per-source enabled flag BEFORE constructing the submitter.
    // Avoids a heap allocation when the source is disabled.
    let per_source_enabled = cfg
        .per_source
        .get(source_lc.as_str())
        .is_some_and(|s| s.enabled);
    if !per_source_enabled {
        return mark_skipped(pool, &application, &listing, "source disabled").await;
    }

    let submitter: Box<dyn Submitter> = match source_lc.as_str() {
        "greenhouse" => Box::new(GreenhouseSubmitter::new()),
        "lever" => Box::new(LeverSubmitter::new()),
        "ashby" => Box::new(AshbySubmitter::new()),
        #[cfg(feature = "browser")]
        "linkedin" => Box::new(crate::linkedin::LinkedinSubmitter::new(
            crate::linkedin::LinkedinConfig::from_core(cfg),
            shared_rate_limiter(),
        )),
        #[cfg(not(feature = "browser"))]
        "linkedin" => {
            return mark_skipped(
                pool,
                &application,
                &listing,
                "linkedin requires --features browser; rebuild with \
                 `cargo build -p careerai-cli --features browser`",
            )
            .await;
        }
        #[cfg(feature = "browser")]
        "naukri" => Box::new(crate::naukri::NaukriSubmitter::new(
            crate::naukri::NaukriConfig::from_core(cfg),
            shared_rate_limiter(),
        )),
        #[cfg(not(feature = "browser"))]
        "naukri" => {
            return mark_skipped(
                pool,
                &application,
                &listing,
                "naukri requires --features browser; rebuild with \
                 `cargo build -p careerai-cli --features browser`",
            )
            .await;
        }
        "remotive" | "remoteok" => {
            return mark_skipped(
                pool,
                &application,
                &listing,
                "feed-only source (no HTTP submitter available)",
            )
            .await;
        }
        other => return Err(SubmitError::UnknownSource(other.to_owned())),
    };

    let decision = if cfg.auto_submit {
        SubmitDecision::Live
    } else {
        SubmitDecision::DryRun
    };

    match decision {
        SubmitDecision::Live => run_live(pool, submitter.as_ref(), &ctx).await,
        SubmitDecision::DryRun => {
            let wrapper = DryRunSubmitter::new(submitter);
            run_dry_run(&wrapper, &ctx)
        }
    }
}

async fn mark_skipped(
    pool: &SqlitePool,
    application: &careerai_db::Application,
    listing: &careerai_db::Listing,
    reason: &str,
) -> Result<SubmitOutcome> {
    info!(
        target: "submit",
        source = %listing.source,
        application_id = %application.id,
        reason,
        "skipping submission"
    );
    let note = format!("skipped: {reason}");
    queries::transition_application_and_listing(
        pool,
        &application.id,
        &listing.id,
        ListingState::Skipped.as_str(),
        ListingState::Skipped,
        Some(&note),
    )
    .await?;
    Ok(SubmitOutcome::Skipped {
        reason: reason.to_owned(),
    })
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
            queries::transition_application_and_listing(
                pool,
                &ctx.application.id,
                &ctx.listing.id,
                ListingState::Submitted.as_str(),
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
            let note = format!("failed: {err}");
            if let Err(transition_err) = queries::transition_application_and_listing(
                pool,
                &ctx.application.id,
                &ctx.listing.id,
                ListingState::Failed.as_str(),
                ListingState::Failed,
                Some(&note),
            )
            .await
            {
                warn!(
                    target: "submit",
                    application_id = %ctx.application.id,
                    error = %transition_err,
                    "failed to transition application to failed state after submission error"
                );
            }
            Err(err)
        }
    }
}

fn run_dry_run(wrapper: &DryRunSubmitter, ctx: &SubmitContext<'_>) -> Result<SubmitOutcome> {
    let would = wrapper.prepare(ctx)?;
    crate::dry_run::log_would_submit(&would, ctx);
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

fn load_profile(root: &Path) -> Result<Profile> {
    let path = root.join("profile").join("profile.yaml");
    let text = std::fs::read_to_string(&path).map_err(SubmitError::Io)?;
    Profile::from_yaml(&text).map_err(|e| {
        SubmitError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("parse {}: {e}", path.display()),
        ))
    })
}
