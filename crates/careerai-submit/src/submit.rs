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

/// Result of dispatching a submitter from the source name.
enum Dispatch {
    Ready(Box<dyn Submitter>),
    Skip(&'static str),
    Unknown(String),
}

/// Build the per-source `Submitter` from the lowercased source name.
/// Browser-based submitters (LinkedIn, Naukri) are gated behind the
/// `browser` feature.
fn dispatch_submitter(source_lc: &str, cfg: &SubmitConfig) -> Dispatch {
    match source_lc {
        "greenhouse" => Dispatch::Ready(Box::new(GreenhouseSubmitter::new())),
        "lever" => Dispatch::Ready(Box::new(LeverSubmitter::new())),
        "ashby" => Dispatch::Ready(Box::new(AshbySubmitter::new())),
        "teamtailor" => Dispatch::Ready(Box::new(TeamtailorSubmitter::new())),
        "smartrecruiters" => Dispatch::Ready(Box::new(SmartRecruitersSubmitter::new())),
        #[cfg(feature = "browser")]
        "linkedin" => Dispatch::Ready(Box::new(crate::linkedin::LinkedinSubmitter::new(
            crate::linkedin::LinkedinConfig::from_core(cfg),
            shared_rate_limiter(),
        ))),
        #[cfg(not(feature = "browser"))]
        "linkedin" => Dispatch::Skip(
            "linkedin requires --features browser; rebuild with `cargo build -p careerai-cli --features browser`",
        ),
        #[cfg(feature = "browser")]
        "naukri" => Dispatch::Ready(Box::new(crate::naukri::NaukriSubmitter::new(
            crate::naukri::NaukriConfig::from_core(cfg),
            shared_rate_limiter(),
        ))),
        #[cfg(not(feature = "browser"))]
        "naukri" => Dispatch::Skip(
            "naukri requires --features browser; rebuild with `cargo build -p careerai-cli --features browser`",
        ),
        "remotive" | "remoteok" => Dispatch::Skip("feed-only source (no HTTP submitter available)"),
        other => Dispatch::Unknown(other.to_owned()),
    }
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
    let application = match queries::find_application_by_id(pool, application_id).await {
        Ok(a) => a,
        Err(careerai_db::DbError::NotFound(_)) => {
            match queries::find_latest_application_for_listing(pool, application_id).await {
                Ok(Some(a)) => a,
                _ => {
                    return Err(SubmitError::Db(careerai_db::DbError::NotFound(format!(
                        "application not found: {application_id}"
                    ))))
                }
            }
        }
        Err(e) => return Err(SubmitError::Db(e)),
    };
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

    let submitter: Box<dyn Submitter> = match dispatch_submitter(&source_lc, cfg) {
        Dispatch::Ready(s) => s,
        Dispatch::Skip(reason) => {
            return mark_skipped(pool, &application, &listing, reason).await;
        }
        Dispatch::Unknown(other) => return Err(SubmitError::UnknownSource(other)),
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
        // Fail closed: if the count query errors, we block the submission
        // rather than letting it through without checking the cap.
        let today_utc = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .map_or_else(chrono::Utc::now, |dt| {
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc)
            });
        let count = queries::count_submissions_since(pool, source, today_utc)
            .await
            .map_err(|e| {
                SubmitError::RateLimited(format!("daily cap check failed for {source}: {e}"))
            })?;
        if count >= policy.max_per_day {
            return Err(SubmitError::RateLimited(format!(
                "daily submission cap ({}) reached for {}",
                policy.max_per_day, source
            )));
        }

        // Enforce min_seconds_between using SQLite latest submission timestamp.
        // Best-effort cross-process check — the in-process governor limiter
        // provides precise enforcement. There is an inherent TOCTOU window
        // between this read and the actual submission that cannot be closed
        // without a distributed lock; this is acceptable for a single-user
        // local daemon.
        if let Ok(Some(last_sub)) = queries::latest_submission_time(pool, source).await {
            let elapsed = chrono::Utc::now()
                .signed_duration_since(last_sub)
                .num_seconds();
            let min_secs = u64::from(policy.min_seconds_between);
            if elapsed >= 0 {
                let elapsed_u64 = u64::try_from(elapsed).unwrap_or(0);
                if elapsed_u64 < min_secs {
                    let wait = min_secs.saturating_sub(elapsed_u64);
                    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                }
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
    // R01: claim a submission attempt row before any network I/O. This
    // gives us a durable record of the attempt even if the process
    // crashes mid-submit, and prevents concurrent workers from
    // double-submitting.
    let attempt = queries::claim_submission_attempt(pool, &ctx.application.id, None)
        .await
        .map_err(SubmitError::from)?;

    match submitter.submit(ctx).await {
        Ok(remote_id) => handle_submit_success(pool, submitter, ctx, &attempt, remote_id).await,
        Err(err) => handle_submit_failure(pool, submitter, ctx, &attempt, err).await,
    }
}

async fn handle_submit_success(
    pool: &SqlitePool,
    submitter: &dyn Submitter,
    ctx: &SubmitContext<'_>,
    attempt: &queries::SubmissionAttempt,
    remote_id: String,
) -> Result<SubmitOutcome> {
    info!(
        target: "submit",
        source = submitter.name(),
        application_id = %ctx.application.id,
        remote_id = %remote_id,
        attempt_id = attempt.id,
        "submitted"
    );

    // R01: record the successful outcome on the attempt row
    // before transitioning state. Non-blocking — the state
    // transition is the source of truth for pipeline flow.
    if let Err(e) =
        queries::mark_attempt_submitted(pool, attempt.id, Some(&remote_id), None).await
    {
        warn!(
            target: "submit",
            application_id = %ctx.application.id,
            attempt_id = attempt.id,
            error = %e,
            "mark_attempt_submitted failed (non-blocking)",
        );
    }

    queries::transition_application_and_listing(
        pool,
        &ctx.application.id,
        &ctx.listing.id,
        ListingState::Submitted.as_str(),
        ListingState::Submitted,
        Some(&format!("submitted via {}", submitter.name())),
    )
    .await?;

    // Record A/B variant for this submission. The label is derived
    // deterministically from the application ID.
    let variant_label = if ctx.application.id.bytes().fold(0u8, u8::wrapping_add) % 2 == 0 {
        "A"
    } else {
        "B"
    };
    let metadata = serde_json::json!({
        "source": submitter.name(),
        "listing_id": &ctx.listing.id,
        "attempt_id": attempt.id,
    })
    .to_string();
    if let Err(e) = queries::record_variant(
        pool,
        &ctx.application.id,
        variant_label,
        Some(&metadata),
        Some(&ctx.application.profile_hash),
    )
    .await
    {
        warn!(
            target: "submit",
            application_id = %ctx.application.id,
            error = %e,
            "record_variant failed (non-blocking)",
        );
    }

    Ok(SubmitOutcome::Submitted { remote_id })
}

async fn handle_submit_failure(
    pool: &SqlitePool,
    submitter: &dyn Submitter,
    ctx: &SubmitContext<'_>,
    attempt: &queries::SubmissionAttempt,
    err: SubmitError,
) -> Result<SubmitOutcome> {
    warn!(
        target: "submit",
        source = submitter.name(),
        application_id = %ctx.application.id,
        attempt_id = attempt.id,
        error = %err,
        "submission failed"
    );

    // R01: classify the failure. Network/timeout errors after the
    // request was sent are "uncertain" — the remote may have received
    // it. Structural/policy errors are "failed".
    let is_uncertain = is_uncertain_error(&err);
    if is_uncertain {
        if let Err(e) = queries::mark_attempt_uncertain(pool, attempt.id, &err.to_string()).await
        {
            warn!(
                target: "submit",
                application_id = %ctx.application.id,
                attempt_id = attempt.id,
                error = %e,
                "mark_attempt_uncertain failed (non-blocking)",
            );
        }
    } else if let Err(e) = queries::mark_attempt_failed(pool, attempt.id, &err.to_string()).await
    {
        warn!(
            target: "submit",
            application_id = %ctx.application.id,
            attempt_id = attempt.id,
            error = %e,
            "mark_attempt_failed failed (non-blocking)",
        );
    }

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

/// R01: classify whether a submit error means the remote may have
/// received the request despite the error. HTTP status errors and
/// structural/policy errors are definitive failures. Network/timeout
/// errors are uncertain — the request may have reached the server.
fn is_uncertain_error(err: &SubmitError) -> bool {
    match err {
        SubmitError::Http(e) => e.is_timeout() || e.is_connect(),
        _ => false,
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
