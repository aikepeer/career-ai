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
use crate::rate_limiter::{RateLimiter, RatePolicy};

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
/// non-browser ATS HTTP submitters. Browser submitters acquire their own
/// permit internally from their dedicated config blocks, so they are
/// deliberately excluded here to avoid double-counting. The permit is
/// committed on both success and failure because a live attempt issues (or
/// attempts) the network write the day-cap exists to bound.
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

/// True when `source` routes to a non-browser ATS HTTP submitter.
/// Browser submitters (`linkedin`, `naukri`) manage their own rate
/// policy, so they are excluded from the boundary-level acquisition.
fn is_ats_http_source(source: &str) -> bool {
    matches!(
        source,
        "greenhouse" | "lever" | "ashby" | "teamtailor" | "smartrecruiters"
    )
}

/// Resolve the rate policy for an ATS HTTP source from
/// `submit.per_source.<source>.*` (the same block that carries the
/// `enabled` gate). Returns `None` when the block has no explicit rate
/// fields so the caller can fall back to the legacy `config.rates`
/// entries without silently overriding them.
fn submit_source_rate(submit: &SubmitConfig, source: &str) -> Option<RatePolicy> {
    let ss = submit.per_source.get(source)?;
    let has_explicit_rate = ss.max_per_day != 0
        || ss.min_seconds_between != 0
        || ss.jitter_seconds != 0
        || ss.quiet_hours_utc.is_some();
    if !has_explicit_rate {
        return None;
    }

    let default = RatePolicy::default();
    let quiet_hours_utc = ss.quiet_hours_utc.filter(|(start, end)| {
        *start <= 23 && *end <= 24 && start != end && !(*start == 0 && *end == 24)
    });
    Some(RatePolicy {
        max_per_day: if ss.max_per_day == 0 {
            default.max_per_day
        } else {
            ss.max_per_day
        },
        min_seconds_between: ss.min_seconds_between,
        jitter_seconds: ss.jitter_seconds,
        quiet_hours_utc,
    })
}

/// Resolve the rate policy for an ATS HTTP source from `config.rates`.
///
/// Lookup order: the source's own `rates.<source>` entry, then the shared
/// `rates.ats_http` group entry, then a conservative default. A
/// `max_per_day` of `0` is treated as "unset" and replaced with the
/// conservative default so an incomplete override cannot silently zero
/// out the cap — operators disable a source via
/// `submit.per_source.<source>.enabled: false`, not via a rate field.
fn rate_policy_for(rates: &RatesConfig, source: &str) -> RatePolicy {
    let Some(sr) = rates
        .per_source
        .get(source)
        .or_else(|| rates.per_source.get("ats_http"))
    else {
        return RatePolicy::default();
    };

    let default = RatePolicy::default();
    RatePolicy {
        max_per_day: if sr.max_per_day == 0 {
            default.max_per_day
        } else {
            sr.max_per_day
        },
        min_seconds_between: sr.min_seconds_between,
        jitter_seconds: sr.jitter_seconds,
        quiet_hours_utc: match sr.quiet_hours.as_slice() {
            [start, end]
                if *start <= 23 && *end <= 24 && start != end && !(*start == 0 && *end == 24) =>
            {
                Some((*start, *end))
            }
            _ => None,
        },
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::collections::HashMap;

    use careerai_core::config::{RatesConfig, SourceRate, SubmitConfig, SubmitSource};

    use super::{is_ats_http_source, rate_policy_for, submit_source_rate};

    fn rate(max_per_day: u32, min_seconds_between: u32, jitter_seconds: u32) -> SourceRate {
        SourceRate {
            max_per_day,
            min_seconds_between,
            jitter_seconds,
            quiet_hours: Vec::new(),
        }
    }

    fn rates(entries: &[(&str, SourceRate)]) -> RatesConfig {
        RatesConfig {
            per_source: entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect::<HashMap<_, _>>(),
        }
    }

    #[test]
    fn is_ats_http_source_distinguishes_http_from_browser() {
        for src in [
            "greenhouse",
            "lever",
            "ashby",
            "teamtailor",
            "smartrecruiters",
        ] {
            assert!(is_ats_http_source(src), "{src} should be ATS HTTP");
        }
        for src in ["linkedin", "naukri", "remotive", "remoteok", "indeed"] {
            assert!(!is_ats_http_source(src), "{src} should not be ATS HTTP");
        }
    }

    #[test]
    fn rate_policy_for_falls_back_to_ats_http_group() {
        let mut group = rate(7, 30, 5);
        group.quiet_hours = vec![0, 7];
        let cfg = rates(&[("ats_http", group)]);
        let p = rate_policy_for(&cfg, "greenhouse");
        assert_eq!(p.max_per_day, 7);
        assert_eq!(p.min_seconds_between, 30);
        assert_eq!(p.jitter_seconds, 5);
        assert_eq!(p.quiet_hours_utc, Some((0, 7)));
    }

    #[test]
    fn rate_policy_for_prefers_source_over_group() {
        let cfg = rates(&[
            ("ats_http", rate(50, 60, 30)),
            ("greenhouse", rate(3, 0, 0)),
        ]);
        let p = rate_policy_for(&cfg, "greenhouse");
        assert_eq!(p.max_per_day, 3);
    }

    #[test]
    fn rate_policy_for_treats_zero_cap_as_unset() {
        let cfg = rates(&[("greenhouse", rate(0, 0, 0))]);
        let p = rate_policy_for(&cfg, "greenhouse");
        assert_eq!(p.max_per_day, super::RatePolicy::default().max_per_day);
    }

    #[test]
    fn rate_policy_for_unknown_source_uses_default() {
        let p = rate_policy_for(&RatesConfig::default(), "greenhouse");
        assert_eq!(p.max_per_day, super::RatePolicy::default().max_per_day);
        assert_eq!(p.quiet_hours_utc, None);
    }

    #[test]
    fn rate_policy_for_rejects_degenerate_quiet_hours() {
        let mut full = rate(10, 0, 0);
        full.quiet_hours = vec![0, 24];
        assert_eq!(
            rate_policy_for(&rates(&[("greenhouse", full)]), "greenhouse").quiet_hours_utc,
            None
        );

        let mut equal = rate(10, 0, 0);
        equal.quiet_hours = vec![7, 7];
        assert_eq!(
            rate_policy_for(&rates(&[("greenhouse", equal)]), "greenhouse").quiet_hours_utc,
            None
        );

        let mut out_of_range = rate(10, 0, 0);
        out_of_range.quiet_hours = vec![25, 30];
        assert_eq!(
            rate_policy_for(&rates(&[("greenhouse", out_of_range)]), "greenhouse").quiet_hours_utc,
            None
        );
    }

    fn submit_config(entries: &[(&str, SubmitSource)]) -> SubmitConfig {
        SubmitConfig {
            per_source: entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect::<HashMap<_, _>>(),
            ..SubmitConfig::default()
        }
    }

    #[test]
    fn submit_source_rate_returns_none_when_no_explicit_rate() {
        let cfg = submit_config(&[("greenhouse", SubmitSource::default())]);
        assert!(submit_source_rate(&cfg, "greenhouse").is_none());
    }

    #[test]
    fn submit_source_rate_reads_per_source_block() {
        let ss = SubmitSource {
            enabled: true,
            max_per_day: 9,
            min_seconds_between: 45,
            jitter_seconds: 10,
            quiet_hours_utc: Some((1, 6)),
        };
        let cfg = submit_config(&[("greenhouse", ss)]);
        let p = submit_source_rate(&cfg, "greenhouse").unwrap();
        assert_eq!(p.max_per_day, 9);
        assert_eq!(p.min_seconds_between, 45);
        assert_eq!(p.jitter_seconds, 10);
        assert_eq!(p.quiet_hours_utc, Some((1, 6)));
    }

    #[test]
    fn submit_source_rate_treats_zero_cap_as_default() {
        let ss = SubmitSource {
            enabled: true,
            max_per_day: 0,
            min_seconds_between: 30,
            jitter_seconds: 0,
            quiet_hours_utc: None,
        };
        let cfg = submit_config(&[("greenhouse", ss)]);
        let p = submit_source_rate(&cfg, "greenhouse").unwrap();
        assert_eq!(p.max_per_day, super::RatePolicy::default().max_per_day);
        assert_eq!(p.min_seconds_between, 30);
    }

    #[test]
    fn submit_source_rate_rejects_degenerate_quiet_hours() {
        let mut ss = SubmitSource {
            enabled: true,
            max_per_day: 10,
            min_seconds_between: 0,
            jitter_seconds: 0,
            quiet_hours_utc: Some((0, 24)),
        };
        let cfg = submit_config(&[("greenhouse", ss.clone())]);
        assert_eq!(
            submit_source_rate(&cfg, "greenhouse")
                .unwrap()
                .quiet_hours_utc,
            None
        );

        ss.quiet_hours_utc = Some((7, 7));
        let cfg = submit_config(&[("greenhouse", ss.clone())]);
        assert_eq!(
            submit_source_rate(&cfg, "greenhouse")
                .unwrap()
                .quiet_hours_utc,
            None
        );

        ss.quiet_hours_utc = Some((25, 30));
        let cfg = submit_config(&[("greenhouse", ss)]);
        assert_eq!(
            submit_source_rate(&cfg, "greenhouse")
                .unwrap()
                .quiet_hours_utc,
            None
        );
    }
}
