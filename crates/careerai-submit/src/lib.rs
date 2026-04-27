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
pub mod linkedin_selectors;
#[cfg(feature = "browser")]
pub mod naukri;
pub mod naukri_selectors;
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
#[cfg(feature = "browser")]
pub use naukri::{NaukriConfig, NaukriSubmitter};
pub use rate_limiter::{RateLimitError, RateLimiter, RatePermit, RatePolicy};

use std::path::Path;

use careerai_core::config::SubmitConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_db::SqlitePool;
use careerai_profile::Profile;
use tracing::{info, warn};

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
fn shared_rate_limiter() -> std::sync::Arc<rate_limiter::RateLimiter> {
    static RL: std::sync::OnceLock<std::sync::Arc<rate_limiter::RateLimiter>> =
        std::sync::OnceLock::new();
    RL.get_or_init(|| std::sync::Arc::new(rate_limiter::RateLimiter::new()))
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
    // 1. Load the application and assert it's ready to submit.
    //    `Drafted` is accepted because `pipeline::confirm_linkedin_submit`
    //    routes through this function after the operator approves a
    //    drafted LinkedIn application in `careerai review`. The pipeline
    //    layer does its own `state == Drafted` precheck before calling.
    //
    //    INVARIANT: `Drafted` is currently only produced by the LinkedIn
    //    interactive_only short-circuit in `pipeline::apply_one`. No
    //    other source writes Drafted today. Any future submitter that
    //    introduces a draft-then-confirm flow MUST add an equivalent
    //    pre-check in the pipeline layer before calling this function;
    //    the source-agnostic allowlist here is a defensive accept, not
    //    an authorisation to draft from arbitrary sources.
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
        // The RateLimiter is now a process-shared singleton via
        // `shared_rate_limiter()` so concurrent `submit_application`
        // calls coordinate one bucket. Day-cap, min-interval, and
        // jitter all do their job even when an `apply --all` batch
        // fans out submissions in parallel.
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
        // Naukri.com browser-driven submitter (Tasks 2.5+2.6). No
        // `interactive_only` gate — the daemon may auto-submit when
        // `auto_submit=true` AND `per_source.naukri.enabled=true`.
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
        // Feed-only sources don't have an HTTP submission API. Browser
        // submitters land in M5 for LinkedIn / Indeed; until then the
        // safe answer is to mark the application Skipped with a clear
        // reason rather than fail the whole batch with UnknownSource.
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
    //
    // Note: an earlier defense-in-depth gate at this point short-
    // circuited LinkedIn live submits to Skipped before the submitter
    // ran. That meant the audit screenshot was never produced — the
    // operator lost the artifact M5a promises. The gate now lives
    // INSIDE `LinkedinSubmitter::run_session` (`allow_submit_click`
    // field), which lets the full state-machine + screenshot run and
    // bails right before the actual click. Two locks remain (the
    // inner allow_submit_click flag and the final unimplemented!()
    // that ships only when M5b lands the click).
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
