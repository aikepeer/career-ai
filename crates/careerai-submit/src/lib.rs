//! Application submitters.
//!
//! Each channel implements `Submitter`. Dry-run is the default.
//! Real submission gated by `auto_submit = true` AND the per-source
//! `submit_enabled` flag, both enforced here.

#![forbid(unsafe_code)]

pub mod ats_http;
pub mod base;
#[cfg(feature = "browser")]
pub mod browser_session;
pub mod credentials;
pub mod dry_run;
pub mod error;
#[cfg(feature = "browser")]
pub mod linkedin;
pub mod rate_limiter;

pub use ats_http::{AshbySubmitter, GreenhouseSubmitter, LeverSubmitter};
pub use base::{SubmitContext, SubmitDecision, SubmitOutcome, Submitter, WouldSubmit};
#[cfg(feature = "browser")]
pub use browser_session::{stealth_script_sha256, BrowserSession, BrowserSessionConfig};
pub use credentials::Credential;
pub use dry_run::DryRunSubmitter;
pub use error::{Result, SubmitError};
#[cfg(feature = "browser")]
pub use linkedin::{LinkedinConfig, LinkedinSubmitter};
pub use rate_limiter::{RateLimitError, RateLimiter, RatePolicy};

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

    // 3. Resolve the submitter for this source. Lookup is
    //    case-insensitive — listing.source is conventionally lowercase
    //    but the schema doesn't enforce it, and a typo shouldn't route
    //    a Greenhouse listing into UnknownSource.
    let source_lc = listing.source.to_ascii_lowercase();
    let submitter: Box<dyn Submitter> = match source_lc.as_str() {
        "greenhouse" => Box::new(GreenhouseSubmitter::new()),
        "lever" => Box::new(LeverSubmitter::new()),
        "ashby" => Box::new(AshbySubmitter::new()),
        // Browser-based LinkedIn path. Only built in when the crate is
        // compiled with `--features browser`; otherwise fall through to
        // a Skipped transition with an actionable rebuild hint.
        //
        // FIXME (M6 daemon): `RateLimiter::new()` is constructed per
        // call here so concurrent `submit_application` invocations do
        // not share a bucket. Safe for M5a because the live path bails
        // with `SourceDisabled` before the rate-gated submit click,
        // but must be hoisted to a process-shared singleton before the
        // daemon flips `auto_submit = true`.
        #[cfg(feature = "browser")]
        "linkedin" => Box::new(crate::linkedin::LinkedinSubmitter::new(
            crate::linkedin::LinkedinConfig::from_core(cfg),
            std::sync::Arc::new(RateLimiter::new()),
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
        // Feed-only sources don't have an HTTP submission API. Browser
        // submitters land in M5 for LinkedIn / Indeed; until then the
        // safe answer is to mark the application Skipped with a clear
        // reason rather than fail the whole batch with UnknownSource.
        "remotive" | "remoteok" | "naukri" => {
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

    // 4. Per-source gating. Unknown source in the map => treated as
    // disabled. Callers opt in explicitly.
    let per_source_enabled = cfg
        .per_source
        .get(source_lc.as_str())
        .is_some_and(|s| s.enabled);
    if !per_source_enabled {
        return mark_skipped(pool, &application, &listing, "source disabled").await;
    }

    // 5. Choose live vs dry-run.
    let decision = if cfg.auto_submit {
        SubmitDecision::Live
    } else {
        SubmitDecision::DryRun
    };

    // 5a. Defense-in-depth for the LinkedIn browser submitter. Even
    // with auto_submit + per_source enabled + a valid li_at cookie,
    // the live click stays suppressed unless cfg.linkedin.allow_submit_click
    // is explicitly true. The inner submitter ALSO returns SourceDisabled
    // before clicking — this is the second lock so a single line edit
    // in linkedin.rs can't ship a ToS violation.
    if matches!(decision, SubmitDecision::Live)
        && source_lc == "linkedin"
        && !cfg.linkedin.allow_submit_click
    {
        return mark_skipped(
            pool,
            &application,
            &listing,
            "linkedin live submit gated: set submit.linkedin.allow_submit_click=true to enable (M5a default-off; M5b lifts after audit)",
        )
        .await;
    }

    match decision {
        SubmitDecision::Live => run_live(pool, submitter.as_ref(), &ctx).await,
        SubmitDecision::DryRun => {
            let wrapper = DryRunSubmitter::new(submitter);
            run_dry_run(&wrapper, &ctx)
        }
    }
}

/// Common Skipped path: log + transition both rows + return outcome.
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
            // Atomic: both rows transition together or neither does.
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
            // Best-effort failure record in a single tx. Original error
            // always propagates, even if recording fails.
            let note = format!("failed: {err}");
            let _ = queries::transition_application_and_listing(
                pool,
                &ctx.application.id,
                &ctx.listing.id,
                ListingState::Failed.as_str(),
                ListingState::Failed,
                Some(&note),
            )
            .await;
            Err(err)
        }
    }
}

/// Dry-run path: prepare once, log via the structured `would_submit`
/// event (PII omitted), return a deterministic outcome. Never calls
/// `submit()` so any stray network access in a custom impl can't leak
/// out from here.
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
        // Surface profile-parse errors as IO-ish since the submit layer
        // doesn't have a dedicated profile-error variant and the
        // operator action is the same: fix the YAML.
        SubmitError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("parse {}: {e}", path.display()),
        ))
    })
}
