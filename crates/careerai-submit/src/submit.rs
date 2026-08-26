use std::path::Path;

use careerai_core::config::{RatesConfig, SubmitConfig};
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_db::SqlitePool;
use careerai_profile::Profile;
use tracing::{info, warn};

use crate::ats_http::{
    AshbySubmitter, GreenhouseSubmitter, LeverSubmitter, SmartRecruitersSubmitter,
    TeamtailorSubmitter,
};
use crate::base::{SubmitContext, SubmitDecision, SubmitOutcome, Submitter};
use crate::dry_run::DryRunSubmitter;
use crate::error::{Result, SubmitError};
use crate::rate_limiter::RateLimiter;
use crate::rate_policy::{is_ats_http_source, rate_policy_for, submit_source_rate};

/// Process-wide rate limiter shared across every `submit_application`
/// call. Per-source counters (day-cap, min-interval bucket) live here,
/// not in a per-call instance, so two concurrent submits coordinate.
/// Built lazily on first access; no state is persisted across process
/// restarts (counters reset, which is the desired behavior at this
/// scale).
///
/// Browser submitters (LinkedIn/Naukri) consume this limiter directly
/// through their own dedicated config blocks; the ATS HTTP submitters
/// acquire a permit from it at the `submit_application` boundary.
fn shared_rate_limiter() -> std::sync::Arc<RateLimiter> {
    static RL: std::sync::OnceLock<std::sync::Arc<RateLimiter>> = std::sync::OnceLock::new();
    RL.get_or_init(|| std::sync::Arc::new(RateLimiter::new()))
        .clone()
}

/// Submit a prepared application. Routes to the correct per-source
/// `Submitter` based on the listing's `source` column. Honors dry-run
/// and per-source `enabled` flags.
pub async fn submit_application(
    pool: &SqlitePool,
    cfg: &SubmitConfig,
    rates: &RatesConfig,
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
        "teamtailor" => Box::new(TeamtailorSubmitter::new()),
        "smartrecruiters" => Box::new(SmartRecruitersSubmitter::new()),
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
                "linkedin requires --features browser; rebuild with `cargo build -p careerai-cli --features browser`",
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
                "naukri requires --features browser; rebuild with `cargo build -p careerai-cli --features browser`",
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
        SubmitDecision::Live => {
            run_live_with_boundary_rate_limit(
                pool,
                submitter.as_ref(),
                &ctx,
                source_lc.as_str(),
                cfg,
                rates,
            )
            .await
        }
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

/// Run a live submission, acquiring a rate-limit permit first for the
/// non-browser ATS HTTP submitters. Also queries SQLite to enforce day caps
/// and minimum intervals across process boundaries.
async fn run_live_with_boundary_rate_limit(
    pool: &SqlitePool,
    submitter: &dyn Submitter,
    ctx: &SubmitContext<'_>,
    source: &str,
    submit_cfg: &SubmitConfig,
    rates: &RatesConfig,
) -> Result<SubmitOutcome> {
    let limiter = shared_rate_limiter();
    let permit = if is_ats_http_source(source) {
        let policy = submit_source_rate(submit_cfg, source)
            .unwrap_or_else(|| rate_policy_for(rates, source));

        // Enforce day cap using SQLite history (cross-process safety).
        let today_utc = chrono::Utc::now().date_naive().and_hms_opt(0, 0, 0)
            .map(|dt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        if let Ok(count) = queries::count_submissions_since(pool, source, today_utc).await {
            if count >= policy.max_per_day {
                return Err(SubmitError::RateLimited(format!(
                    "daily submission cap ({}) reached for {}",
                    policy.max_per_day, source
                )));
            }
        }

        // Enforce min_seconds_between using SQLite latest submission timestamp.
        if let Ok(Some(last_sub)) = queries::latest_submission_time(pool, source).await {
            let elapsed = chrono::Utc::now().signed_duration_since(last_sub).num_seconds();
            let min_secs = u64::from(policy.min_seconds_between);
            if elapsed >= 0 && (elapsed as u64) < min_secs {
                let wait = min_secs.saturating_sub(elapsed as u64);
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
        }

        Some(
            limiter
                .acquire(source, &policy)
                .await
                .map_err(|e| SubmitError::RateLimited(e.to_string()))?,
        )
    } else {
        None
    };

    let outcome = run_live(pool, submitter, ctx).await;
    if let Some(permit) = permit {
        permit.commit();
    }
    outcome
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
    let path = careerai_core::paths::profile_path(root);
    let text = std::fs::read_to_string(&path).map_err(SubmitError::Io)?;
    Profile::from_yaml(&text).map_err(|e| {
        SubmitError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("parse {}: {e}", path.display()),
        ))
    })
}


