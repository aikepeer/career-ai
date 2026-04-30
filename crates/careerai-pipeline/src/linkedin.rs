//! LinkedIn assist-mode review flow: `list_drafted_linkedin` (read)
//! and `confirm_linkedin_submit` (race-guarded claim + submit).
//! Extracted from `lib.rs` to keep that file under 300 LOC.

use std::path::Path;

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;

use crate::apply::{apply_one, AppliedOutcome};
use crate::open_pool;

/// List all applications currently in the `drafted` state whose listing source
/// is `linkedin`, oldest-first.
///
/// Consumed by `careerai review` to enumerate the queue of applications that
/// the daemon parked in `Drafted` due to `interactive_only = true`. The
/// operator is then prompted to confirm or skip each one.
pub async fn list_drafted_linkedin(
    root: &Path,
    limit: i64,
) -> Result<Vec<careerai_db::Application>> {
    let pool = open_pool(root).await?;
    queries::list_drafted_linkedin(&pool, limit)
        .await
        .context("list drafted linkedin applications")
}

/// Confirm-submit a drafted LinkedIn application via the browser, called
/// from `careerai review` after the operator approves. Overrides
/// `interactive_only` to `false` for this single invocation so the daemon-
/// path short-circuit in `apply_one` doesn't fire.
///
/// The operator config on disk is **not** modified; only an ephemeral clone
/// is used.
///
/// Race guard: uses an atomic conditional UPDATE
/// (`queries::claim_drafted_application`) to transition Drafted → Rendered
/// before the browser launch. Two concurrent `careerai review` processes
/// serialize via SQLite's write lock; only one wins the claim. The loser
/// returns a clear error and never spawns a Chromium session.
pub async fn confirm_linkedin_submit(
    root: &Path,
    cfg: &CoreConfig,
    application_id: &str,
) -> Result<AppliedOutcome> {
    // Clone config and lift the assist-mode gate just for this call.
    let mut effective_cfg = cfg.clone();
    effective_cfg.submit.linkedin.interactive_only = false;

    let pool = open_pool(root).await?;
    let claimed = queries::claim_drafted_application(&pool, application_id)
        .await
        .context("claim drafted application")?;
    if !claimed {
        // Read the current state for an actionable error. The claim
        // already failed, so this read is purely diagnostic.
        let app = queries::find_application_by_id(&pool, application_id).await?;
        drop(pool);
        let expected_state = ListingState::Drafted.as_str();
        return Err(anyhow::anyhow!(
            "application {} is in state '{}', expected '{}' \
             (already submitted, already failed, or another `careerai review` won the claim)",
            application_id,
            app.state,
            expected_state,
        ));
    }
    drop(pool);

    // Claim won — application is now in Rendered state. Delegate to
    // apply_one with Some(true) to force live submission. `careerai
    // review` is the explicit operator-confirmation path; inheriting
    // cfg.submit.auto_submit (which defaults to false) would silently
    // dry-run after the operator typed 'y'.
    apply_one(root, &effective_cfg, application_id, Some(true)).await
}
