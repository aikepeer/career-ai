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
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::models::NewListing;
use careerai_db::{pool_from_path, queries, SqlitePool};
use careerai_profile::Profile;
use careerai_sources::{
    GreenhouseSource, IndeedRssSource, LeverSource, McpJobsSource, NaukriSource, RemoteOkSource,
    RemotiveSource, Source,
};

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
mod inspect;
mod linkedin;
mod match_;
mod render;
mod run;
mod tailor;

pub use apply::{applied_show, apply_all, apply_one, retry_application, AppliedOutcome};
pub use digest::digest_summary;
pub use inspect::{inspect_show, InspectReport};
pub use linkedin::{confirm_linkedin_submit, list_drafted_linkedin};
pub use match_::{match_all, match_one, MatchReport};
pub use render::{render_one, RenderedOutcome};
pub use run::{run_pipeline, ApplyReport, ApplySourceReport, RunFailure, RunReport};
pub use tailor::{tailor_one, TailoredOutcome};

pub async fn open_pool(root: &Path) -> Result<SqlitePool> {
    let path = root.join("data").join("careerai.sqlite");
    pool_from_path(&path)
        .await
        .with_context(|| format!("open db at {}", path.display()))
}

/// Build the list of configured source adapters from `CoreConfig`.
fn build_sources(cfg: &CoreConfig) -> Vec<Arc<dyn Source>> {
    let mut out: Vec<Arc<dyn Source>> = Vec::new();
    for company in &cfg.sources.greenhouse.companies {
        out.push(Arc::new(GreenhouseSource::new(company.clone())));
    }
    for company in &cfg.sources.lever.companies {
        out.push(Arc::new(LeverSource::new(company.clone())));
    }
    for company in &cfg.sources.ashby.companies {
        out.push(Arc::new(careerai_sources::AshbySource::new(
            company.clone(),
        )));
    }
    for company in &cfg.sources.teamtailor.companies {
        out.push(Arc::new(careerai_sources::TeamtailorSource::new(
            company.clone(),
        )));
    }
    if cfg.sources.remotive.enabled {
        let mut s = RemotiveSource::new();
        if let Some(cat) = &cfg.sources.remotive.category {
            s = s.with_category(cat.clone());
        }
        out.push(Arc::new(s));
    }
    if cfg.sources.remoteok.enabled {
        out.push(Arc::new(RemoteOkSource::new()));
    }
    if cfg.sources.naukri.enabled {
        let mut s = NaukriSource::new();
        if !cfg.sources.naukri.keywords.is_empty() {
            s = s.with_keywords(cfg.sources.naukri.keywords.clone());
        }
        if let Some(loc) = &cfg.sources.naukri.location {
            s = s.with_location(loc.clone());
        }
        if let Some(n) = cfg.sources.naukri.max_results {
            s = s.with_max_results(n);
        }
        out.push(Arc::new(s));
    }
    if cfg.sources.indeed_rss.enabled {
        out.push(Arc::new(IndeedRssSource::new(
            cfg.sources.indeed_rss.clone(),
        )));
    }
    for mcp_cfg in &cfg.sources.mcp {
        if !mcp_cfg.enabled {
            continue;
        }
        out.push(Arc::new(McpJobsSource::new(mcp_cfg.clone())));
    }
    // LinkedIn browser-driven discovery (PR #21). Only registered
    // when both the `browser` feature is compiled in AND the user
    // has opted in via `sources.linkedin_browser.enabled = true`.
    // The runtime adapter pulls in chromiumoxide + the M5
    // BrowserSession, so the cfg-gate keeps the default `cargo
    // build` lean for hosts without Chromium.
    #[cfg(feature = "browser")]
    {
        if cfg.sources.linkedin_browser.enabled {
            out.push(Arc::new(careerai_sources::LinkedinBrowserSource::new(
                cfg.sources.linkedin_browser.clone(),
            )));
        }
    }
    #[cfg(not(feature = "browser"))]
    {
        if cfg.sources.linkedin_browser.enabled {
            tracing::warn!(
                "linkedin_browser source is enabled in config but `browser` feature is OFF; \
                 rebuild with `cargo build --features browser` to activate it"
            );
        }
    }
    out
}

/// Build a single source adapter by name. Used by `discover_one` to
/// avoid constructing every configured adapter when the scheduler fires
/// a per-source cron tick. Returns `None` when no enabled source
/// matches `name` (caller should treat this as a no-op, not an error).
fn build_one_source(cfg: &CoreConfig, name: &str) -> Option<Arc<dyn Source>> {
    match name {
        "greenhouse" => cfg
            .sources
            .greenhouse
            .companies
            .first()
            .map(|company| Arc::new(GreenhouseSource::new(company.clone())) as Arc<dyn Source>),
        "lever" => cfg
            .sources
            .lever
            .companies
            .first()
            .map(|company| Arc::new(LeverSource::new(company.clone())) as Arc<dyn Source>),
        "ashby" => cfg.sources.ashby.companies.first().map(|company| {
            Arc::new(careerai_sources::AshbySource::new(company.clone())) as Arc<dyn Source>
        }),
        "teamtailor" => cfg.sources.teamtailor.companies.first().map(|company| {
            Arc::new(careerai_sources::TeamtailorSource::new(company.clone())) as Arc<dyn Source>
        }),
        "remotive" if cfg.sources.remotive.enabled => {
            let mut s = RemotiveSource::new();
            if let Some(cat) = &cfg.sources.remotive.category {
                s = s.with_category(cat.clone());
            }
            Some(Arc::new(s))
        }
        "remoteok" if cfg.sources.remoteok.enabled => Some(Arc::new(RemoteOkSource::new())),
        "naukri" if cfg.sources.naukri.enabled => {
            let mut s = NaukriSource::new();
            if !cfg.sources.naukri.keywords.is_empty() {
                s = s.with_keywords(cfg.sources.naukri.keywords.clone());
            }
            if let Some(loc) = &cfg.sources.naukri.location {
                s = s.with_location(loc.clone());
            }
            if let Some(n) = cfg.sources.naukri.max_results {
                s = s.with_max_results(n);
            }
            Some(Arc::new(s))
        }
        "indeed_rss" if cfg.sources.indeed_rss.enabled => Some(Arc::new(IndeedRssSource::new(
            cfg.sources.indeed_rss.clone(),
        ))),
        #[cfg(feature = "browser")]
        "linkedin-browser" if cfg.sources.linkedin_browser.enabled => Some(Arc::new(
            careerai_sources::LinkedinBrowserSource::new(cfg.sources.linkedin_browser.clone()),
        )),
        #[cfg(not(feature = "browser"))]
        "linkedin-browser" if cfg.sources.linkedin_browser.enabled => {
            tracing::warn!(
                "linkedin_browser source is enabled in config but `browser` feature is OFF; \
                 rebuild with `cargo build --features browser` to activate it"
            );
            None
        }
        _ => {
            // MCP sources are keyed by their configured name, not a fixed
            // string. Fall back to scanning the mcp config list.
            cfg.sources.mcp.iter().find_map(|mcp| {
                if mcp.enabled && mcp.name == name {
                    Some(Arc::new(McpJobsSource::new(mcp.clone())) as Arc<dyn Source>)
                } else {
                    None
                }
            })
        }
    }
}

pub async fn discover_all(
    root: &Path,
    cfg: &CoreConfig,
    source_filter: &[String],
) -> Result<DiscoveryReport> {
    let pool = open_pool(root).await?;
    let mut sources = build_sources(cfg);
    if !source_filter.is_empty() {
        sources.retain(|s| source_filter.iter().any(|f| f == s.name()));
    }
    if sources.is_empty() {
        warn!("no sources enabled — edit config/default.yaml or config/local.yaml");
        return Ok(DiscoveryReport::default());
    }

    let mut report = DiscoveryReport::default();
    for source in sources {
        let name = source.name();
        match source.discover().await {
            Ok(listings) => {
                info!(source = name, count = listings.len(), "discovered");
                report.fetched += listings.len();
                for raw in listings {
                    let title = raw.title.clone();
                    let company = raw.company.clone();
                    let source = raw.source.clone();
                    let new = NewListing {
                        source: raw.source,
                        external_id: raw.external_id,
                        title: raw.title,
                        company: raw.company,
                        location: raw.location,
                        url: raw.url,
                        description: raw.description,
                        raw_json: raw.raw_json,
                    };
                    match queries::insert_or_ignore(&pool, &new).await {
                        Ok((id, true)) => {
                            report.new_rows += 1;
                            report.new.push(NewListingRow {
                                id,
                                title,
                                company,
                                source,
                            });
                        }
                        Ok((_, false)) => report.duplicates += 1,
                        Err(e) => {
                            warn!(error = %e, "persist failed");
                            report.errors += 1;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(source = name, error = %e, "source failed");
                report.errors += 1;
            }
        }
    }
    Ok(report)
}

/// Run discovery for a single configured source. Used by the scheduler's
/// per-source cron tick — each tick fires this for exactly one source.
///
/// Constructs only the requested source adapter via `build_one_source`,
/// avoiding the full `build_sources()` allocation (every adapter, every
/// tick). Returns an empty report (with no error) if `source` doesn't
/// match any enabled adapter.
pub async fn discover_one(root: &Path, cfg: &CoreConfig, source: &str) -> Result<DiscoveryReport> {
    let Some(source_adapter) = build_one_source(cfg, source) else {
        return Ok(DiscoveryReport::default());
    };
    let pool = open_pool(root).await?;
    let mut report = DiscoveryReport::default();
    let name = source_adapter.name();
    match source_adapter.discover().await {
        Ok(listings) => {
            info!(source = name, count = listings.len(), "discovered");
            report.fetched += listings.len();
            for raw in listings {
                let title = raw.title.clone();
                let company = raw.company.clone();
                let source = raw.source.clone();
                let new = NewListing {
                    source: raw.source,
                    external_id: raw.external_id,
                    title: raw.title,
                    company: raw.company,
                    location: raw.location,
                    url: raw.url,
                    description: raw.description,
                    raw_json: raw.raw_json,
                };
                match queries::insert_or_ignore(&pool, &new).await {
                    Ok((id, true)) => {
                        report.new_rows += 1;
                        report.new.push(NewListingRow {
                            id,
                            title,
                            company,
                            source,
                        });
                    }
                    Ok((_, false)) => report.duplicates += 1,
                    Err(e) => {
                        warn!(error = %e, "persist failed");
                        report.errors += 1;
                    }
                }
            }
        }
        Err(e) => {
            warn!(source = name, error = %e, "source failed");
            report.errors += 1;
        }
    }
    Ok(report)
}

#[derive(Debug, Default)]
pub struct DiscoveryReport {
    pub fetched: usize,
    pub new_rows: usize,
    pub duplicates: usize,
    pub errors: usize,
    /// Newly inserted listings (id + title/company/source projection).
    /// Lets the CLI print ids so the operator can run
    /// `careerai tailor <id>` straight after discovery.
    pub new: Vec<NewListingRow>,
}

/// Projection of a listing inserted by `discover_all` / `discover_one`.
#[derive(Debug, Default, Clone)]
pub struct NewListingRow {
    pub id: String,
    pub title: String,
    pub company: String,
    pub source: String,
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
