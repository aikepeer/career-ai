//! Pipeline orchestration: discover / match / tailor / render / apply.
//!
//! Owns the linear-state-machine entry points the CLI and scheduler both
//! drive. Lives in its own crate (rather than `careerai-core`) because the
//! implementation crates it pulls in already depend on `careerai-core` for
//! shared types. Putting orchestration in core would create a cycle.
//!
//! Each pipeline stage lives in its own submodule so `lib.rs` stays under
//! the project's 300-LOC cap. Shared helpers (`open_pool`, `build_sources`,
//! `load_profile`) and report types stay here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::{pool_from_path, queries, SqlitePool};
use careerai_profile::Profile;
use careerai_sources::{
    GreenhouseSource, IndeedRssSource, LeverSource, McpJobsSource, NaukriSource, RemoteOkSource,
    RemotiveSource, Source,
};

// Pipeline-stage submodules.
mod apply;
mod digest;
mod discover;
mod inspect;
mod linkedin;
#[path = "match_.rs"]
mod match_;
mod render;
mod tailor;

// Re-exports so callers can use `careerai_pipeline::apply_one(...)` etc.
pub use apply::{applied_show, apply_all, apply_one, AppliedOutcome};
pub use digest::digest_summary;
pub use discover::{discover_all, discover_one};
pub use inspect::{inspect_show, InspectReport};
pub use linkedin::{confirm_linkedin_submit, list_drafted_linkedin};
pub use match_::{match_all, match_one};
pub use render::render_one;
pub use tailor::tailor_one;

pub async fn open_pool(root: &Path) -> Result<SqlitePool> {
    let path = root.join("data").join("careerai.sqlite");
    pool_from_path(&path)
        .await
        .with_context(|| format!("open db at {}", path.display()))
}

/// Build the list of configured source adapters from `CoreConfig`.
pub(crate) fn build_sources(cfg: &CoreConfig) -> Vec<Arc<dyn Source>> {
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

pub(crate) fn load_profile(root: &Path) -> Result<Profile> {
    let path = root.join("profile").join("profile.yaml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Profile::from_yaml(&text).context("parse profile yaml")
}

// ---------------------------------------------------------------------------
// Shared report types. Defined here (not in submodules) so the MCP server,
// CLI, and scheduler can import them without reaching into submodule paths.

#[derive(Debug, Default)]
pub struct DiscoveryReport {
    pub fetched: usize,
    pub new_rows: usize,
    pub duplicates: usize,
    pub errors: usize,
}

/// Per-source activity counts within the digest window.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct SourceCounts {
    pub total: usize,
}

/// Daily-digest snapshot returned by `digest_summary`.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct DigestReport {
    pub since_iso: String,
    pub discovered: usize,
    /// Listings that reached either `shortlisted` or `filtered_out`.
    pub matched: usize,
    pub shortlisted: usize,
    pub drafted: usize,
    pub submitted: usize,
    pub failed: usize,
    pub responded: usize,
    pub per_source: std::collections::HashMap<String, SourceCounts>,
    pub last_tick: Option<String>,
    pub cookie_warnings: Vec<String>,
}

#[derive(Debug, Default)]
pub struct MatchReport {
    pub filtered_out: usize,
    pub shortlisted: usize,
    pub also_filtered: usize,
    pub histogram: [(f32, usize); 10],
}

#[derive(Debug)]
pub struct TailoredOutcome {
    pub application_id: String,
    pub listing_title: String,
    pub company: String,
}

#[derive(Debug)]
pub struct RenderedOutcome {
    pub application_id: String,
    pub resume_md: PathBuf,
    pub resume_docx: PathBuf,
    pub resume_pdf: PathBuf,
    pub cover_md: PathBuf,
    pub cover_docx: PathBuf,
    pub bytes: BTreeMap<PathBuf, u64>,
}

pub async fn shortlist_show(root: &Path, limit: i64) -> Result<Vec<careerai_db::Listing>> {
    let pool = open_pool(root).await?;
    let rows = queries::list_by_state(&pool, ListingState::Shortlisted, limit)
        .await
        .context("list shortlisted")?;
    Ok(rows)
}
