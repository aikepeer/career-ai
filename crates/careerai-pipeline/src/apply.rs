//! Apply / applied / inspect (M4 wave 2) — the submit-side
//! pipeline stages. Extracted from `lib.rs` to keep that file
//! under the project's 300-LOC cap.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::{error, warn};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;

use crate::open_pool;

/// One row of `apply --all` output. One `AppliedOutcome` is emitted per
/// application the CLI attempted to submit, regardless of whether the
/// per-call result was success, skip, or dry-run.
#[derive(Debug)]
pub struct AppliedOutcome {
    pub application_id: String,
    pub source: String,
    pub outcome: careerai_submit::SubmitOutcome,
}

/// A single failed application attempt. Kept separate from `AppliedOutcome`
/// so `apply_all_detailed` can surface the failure list to `run_pipeline`
/// without changing `apply_all`'s existing `Vec<AppliedOutcome>` contract.
#[derive(Debug)]
pub struct ApplyFailure {
    pub application_id: String,
    pub source: String,
    pub error: String,
}

/// Build a per-call `SubmitConfig` honoring the optional CLI override.
///
/// When `override_auto_submit` is `Some(true)` the call is forced live;
/// `Some(false)` forces dry-run; `None` passes the config's stored value
/// through unchanged.
fn effective_submit_cfg(
    cfg: &CoreConfig,
    override_auto_submit: Option<bool>,
) -> careerai_core::config::SubmitConfig {
    let mut submit = cfg.submit.clone();
    if let Some(v) = override_auto_submit {
        submit.auto_submit = v;
    }
    submit
}

/// Submit a single prepared application (state `rendered` or `prepared`).
///
/// Gating, state transitions, and artifact loading all live inside
/// `careerai_submit::submit_application`; this wrapper only opens the pool
/// and overlays the `--auto-submit` flag on top of the config.
pub async fn apply_one(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
    auto_submit_override: Option<bool>,
) -> Result<AppliedOutcome> {
    let pool = open_pool(root).await?;

    // Pre-fetch surfaces the typed errors from careerai-db; the CLI's
    // exit-code mapper downcasts to `DbError::NotFound` directly. We do
    // NOT bail!() into a stringly-typed error here — that was previously
    // breaking exit-code classification once any `.context(...)` wrapper
    // ran upstream.
    let application = queries::find_application_by_id(&pool, application_id).await?;
    let listing = queries::find_by_id(&pool, &application.listing_id).await?;

    // LinkedIn assist mode: when interactive_only is set (default true), the
    // daemon never opens a browser session and never clicks Submit
    // autonomously. Transition the application to Drafted in the DB and
    // return `SubmitOutcome::Drafted` (a real state change, not a dry-run).
    // `careerai review` is the only path that turns Drafted → Submitted, by
    // calling `confirm_linkedin_submit` which atomically claims the row and
    // delegates to submit_application with interactive_only=false.
    // listings.source is conventionally lowercase but the schema doesn't
    // enforce it; submit_application uses to_ascii_lowercase too. Match
    // case-insensitively here so a "LinkedIn" or "LINKEDIN" row doesn't
    // silently bypass the assist-mode short-circuit.
    if listing.source.eq_ignore_ascii_case("linkedin") && cfg.submit.linkedin.interactive_only {
        queries::transition_application_and_listing(
            &pool,
            &application.id,
            &listing.id,
            ListingState::Drafted.as_str(),
            ListingState::Drafted,
            Some("drafted: awaiting careerai review"),
        )
        .await
        .context("transition application to drafted")?;
        return Ok(AppliedOutcome {
            application_id: application.id,
            source: listing.source,
            outcome: careerai_submit::SubmitOutcome::Drafted {
                note: "awaiting careerai review".into(),
            },
        });
    }

    let submit_cfg = effective_submit_cfg(cfg, auto_submit_override);
    let outcome =
        careerai_submit::submit_application(&pool, &submit_cfg, &cfg.rates, root, application_id)
            .await
            .context("submit_application")?;

    Ok(AppliedOutcome {
        application_id: application.id,
        source: listing.source,
        outcome,
    })
}

/// Iterate every application currently in state `rendered` or `prepared`
/// and submit each one. Per-application failures are logged and skipped —
/// `apply --all` deliberately doesn't abort the batch on the first error.
pub async fn apply_all(
    root: &Path,
    cfg: &CoreConfig,
    source_filter: Option<&str>,
    auto_submit_override: Option<bool>,
) -> Result<Vec<AppliedOutcome>> {
    let (outcomes, _) = apply_all_detailed(root, cfg, source_filter, auto_submit_override).await?;
    Ok(outcomes)
}

/// The same iteration as `apply_all`, but also returns the failed
/// attempts so `run_pipeline` can include them in its report instead of
/// silently dropping them.
pub async fn apply_all_detailed(
    root: &Path,
    cfg: &CoreConfig,
    source_filter: Option<&str>,
    auto_submit_override: Option<bool>,
) -> Result<(Vec<AppliedOutcome>, Vec<ApplyFailure>)> {
    let pool = open_pool(root).await?;
    let eligible = eligible_applications(&pool, source_filter).await?;

    let mut outcomes = Vec::with_capacity(eligible.len());
    let mut failures = Vec::new();
    for app in eligible {
        match apply_one(root, cfg, &app.id, auto_submit_override).await {
            Ok(o) => outcomes.push(o),
            Err(e) => {
                // H4: don't silently retry forever. A single corrupt
                // artifact, persistent HTTP 500, or unknown-source error
                // would otherwise re-fail every tick (every 15 min by
                // default), polluting logs and burning rate-limit budget.
                // Transition the application to `failed` so apply_all stops
                // re-fetching it; operators who want to retry must
                // explicitly re-shortlist via `careerai shortlist`.
                warn!(
                    application_id = %app.id,
                    error = %e,
                    "apply_one failed, marking application failed",
                );
                if let Err(transition_err) =
                    queries::set_application_state(&pool, &app.id, "failed").await
                {
                    error!(
                        application_id = %app.id,
                        error = %transition_err,
                        "failed to transition application to failed state",
                    );
                }
                let source = queries::find_by_id(&pool, &app.listing_id)
                    .await
                    .map_or_else(|_| "unknown".to_string(), |listing| listing.source);
                failures.push(ApplyFailure {
                    application_id: app.id,
                    source,
                    error: format!("{e:#}"),
                });
            }
        }
    }
    Ok((outcomes, failures))
}

/// Reset a `failed` application back to its pre-submit state and re-run
/// `apply_one`. The pre-submit state is recovered from the listing's most
/// recent `failed` event so retry mirrors exactly what was attempted
/// before the failure.
pub async fn retry_application(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
) -> Result<AppliedOutcome> {
    let pool = open_pool(root).await?;

    let application = queries::find_application_by_id(&pool, application_id).await?;
    if application.state != "failed" {
        anyhow::bail!(
            "application {application_id} is in state '{}'; expected 'failed'",
            application.state,
        );
    }
    let listing = queries::find_by_id(&pool, &application.listing_id).await?;

    let pre_submit = pre_submit_state(&pool, &listing.id).await?;
    let listing_state: ListingState = pre_submit
        .parse()
        .with_context(|| format!("invalid pre-submit listing state '{pre_submit}'"))?;

    queries::transition_application_and_listing(
        &pool,
        &application.id,
        &listing.id,
        &pre_submit,
        listing_state,
        Some(&format!("retry application {application_id}")),
    )
    .await
    .context("reset failed application to pre-submit state")?;

    apply_one(root, cfg, application_id, None).await
}

async fn pre_submit_state(pool: &careerai_db::SqlitePool, listing_id: &str) -> Result<String> {
    let events = queries::events_for(pool, listing_id).await?;
    let pre_submit = events
        .iter()
        .rev()
        .find(|event| event.to_state == "failed")
        .and_then(|event| event.from_state.clone())
        .unwrap_or_else(|| ListingState::Rendered.as_str().to_string());
    Ok(pre_submit)
}

async fn eligible_applications(
    pool: &careerai_db::SqlitePool,
    source_filter: Option<&str>,
) -> Result<Vec<careerai_db::Application>> {
    // Both "rendered" and "prepared" are eligible per submit_application's
    // BadState guard. Use the JOIN-based query so a `--source` filter is
    // pushed into SQL — previously the CLI fetched every listing per row
    // (O(N) round-trips) and filtered client-side.
    let mut eligible: Vec<careerai_db::Application> = Vec::new();
    for state in ["rendered", "prepared"] {
        let rows =
            queries::list_applications_by_state_and_source(pool, state, source_filter, 1_000)
                .await
                .with_context(|| format!("list applications in state '{state}'"))?;
        eligible.extend(rows);
    }
    Ok(eligible)
}

/// List applications already submitted, newest first. Optional `source`
/// filter matches against the linked listing's source.
pub async fn applied_show(
    root: &Path,
    source_filter: Option<&str>,
    limit: i64,
) -> Result<Vec<careerai_db::Application>> {
    let pool = open_pool(root).await?;
    queries::list_applications_by_state_and_source(&pool, "submitted", source_filter, limit)
        .await
        .context("list submitted applications")
}
