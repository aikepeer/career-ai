//! Pipeline orchestration: discover / match / tailor / render / apply.
//!
//! Owns the linear-state-machine entry points the CLI and scheduler both
//! drive. Lives in its own crate (rather than `careerai-core`) because the
//! implementation crates it pulls in — `careerai-db`, `careerai-sources`,
//! `careerai-match`, `careerai-tailor`, `careerai-render`, `careerai-submit`,
//! `careerai-llm`, `careerai-profile` — all already depend on `careerai-core`
//! for shared types. Putting orchestration in core would create a cycle.
//!
//! Architectural rule: this crate is the *only* place where the pipeline
//! stages get composed end-to-end. Both `careerai-cli` (for one-shot
//! subcommands) and `careerai-scheduler` (for cron ticks) call into here.
//! Neither of them should reach past this layer into the implementation
//! crates directly.

use std::path::Path;

use anyhow::{Context, Result};

use careerai_core::state::ListingState;
use careerai_db::{pool_from_path, queries, SqlitePool};
use careerai_profile::Profile;

// Pipeline-stage submodules. Each owns one stage end-to-end and stays
// under the project's 300-LOC cap. lib.rs now keeps only `discover_*`,
// `shortlist_show`, the shared report types
// (`DiscoveryReport`/`SourceCounts`/`DigestReport`), `open_pool`,
// `build_sources`, and `load_profile` — all the stage entries are
// re-exported so callers continue using
// `careerai_pipeline::{match_all, tailor_one, render_one, apply_one}`
// etc. without changes.
mod apply;
mod digest;
mod discover;
mod inspect;
mod linkedin;
mod match_;
mod rematch;
mod render;
mod run;
mod tailor;

pub use apply::{applied_show, apply_all, apply_one, retry_application, AppliedOutcome};
pub use digest::digest_summary;
pub use discover::{
    build_sources, build_sources_for_name, discover_all, discover_one, DiscoveryReport,
    NewListingRow,
};
pub use inspect::{inspect_show, InspectReport};
pub use linkedin::{confirm_linkedin_submit, list_drafted_linkedin};
pub use match_::{match_all, match_one, MatchReport};
pub use rematch::{rematch_shortlisted, RematchReport};
pub use render::{render_one, RenderedOutcome};
pub use run::{run_pipeline, ApplyReport, ApplySourceReport, RunFailure, RunReport};
pub use tailor::{tailor_one, TailoredOutcome};

pub async fn open_pool(root: &Path) -> Result<SqlitePool> {
    let path = root.join("data").join("careerai.sqlite");
    pool_from_path(&path)
        .await
        .with_context(|| format!("open db at {}", path.display()))
}

/// Per-source activity counts within the digest window.
///
/// `#[non_exhaustive]` reserves the right to add per-source breakdowns
/// (submitted, failed, etc.) without breaking exhaustive struct patterns
/// at call sites.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct SourceCounts {
    /// Distinct listings from this source that had any state transition
    /// within the window.
    pub total: usize,
}

/// Daily-digest snapshot returned by `digest_summary`. All count fields
/// are "distinct listings that reached this state within `since`" — so a
/// listing that moved Discovered → Shortlisted → Submitted in the window
/// contributes once each to `discovered`, `shortlisted`, and `submitted`.
///
/// **Counting invariant:** `matched` is a *superset* of `shortlisted`.
/// `matched = (listings that reached Shortlisted) + (listings that
/// reached FilteredOut)`. The CLI prints them side-by-side so the
/// operator can read `matched: 12   shortlisted: 8` as "12 listings
/// were classified, 8 of them survived the filter".
///
/// `#[non_exhaustive]` so report fields can be added without breaking
/// callers that pattern-match exhaustively.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct DigestReport {
    /// ISO-8601 UTC timestamp marking the start of the window.
    pub since_iso: String,
    pub discovered: usize,
    /// Listings that reached either `shortlisted` or `filtered_out` —
    /// i.e. matcher activity (kept + rejected combined). Superset of
    /// `shortlisted`.
    pub matched: usize,
    pub shortlisted: usize,
    pub drafted: usize,
    pub submitted: usize,
    pub failed: usize,
    pub responded: usize,
    pub per_source: std::collections::HashMap<String, SourceCounts>,
    /// Most recent `events.created_at` in the database, formatted as ISO-
    /// 8601 UTC. `None` when there are no events at all.
    pub last_tick: Option<String>,
    /// Human-readable warnings about credential expiry, missing
    /// keyring entries, or unparseable cookies, e.g.
    /// `"li_at expires in 1d 6h"`. The pipeline crate leaves this
    /// empty; the CLI populates it via `careerai_submit::credentials`.
    /// Held here (rather than in a CLI-side wrapper) so other
    /// consumers of `digest_summary` (future web UI, API) get the
    /// same shape without each re-implementing the populate step.
    pub cookie_warnings: Vec<String>,
}

// match_all / match_one / fire_high_score_if_above / MatchReport +
// the high_score_notify_tests block all live in match_.rs now.

pub async fn shortlist_show(root: &Path, limit: i64) -> Result<Vec<careerai_db::Listing>> {
    let pool = open_pool(root).await?;
    let rows = queries::list_by_state(&pool, ListingState::Shortlisted, limit)
        .await
        .context("list shortlisted")?;
    Ok(rows)
}

pub(crate) fn load_profile(root: &Path) -> Result<Profile> {
    let path = careerai_core::paths::profile_path(root);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Profile::from_yaml(&text).context("parse profile yaml")
}

// TailoredOutcome / RenderedOutcome / fixtures_dir / live_llm_opt_in /
// tailor_one / render_one all live in tailor.rs and render.rs.
// Re-exported via `pub use` at the top of this file.
//
// apply / applied / inspect / linkedin / digest stages live in
// {apply,linkedin,inspect,digest}.rs.
