//! Discovery stage — ingest job postings from all enabled sources.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use tracing::{info, warn};

use careerai_core::config::CoreConfig;
use careerai_db::models::NewListing;
use careerai_db::{queries, SqlitePool};
use careerai_sources::{
    FreehireSource, GreenhouseSource, IndeedRssSource, LeverSource, McpJobsSource, NaukriSource,
    RemoteOkSource, RemotiveSource, Source,
};

use crate::open_pool;

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

/// Build the list of configured source adapters from `CoreConfig`.
pub fn build_sources(cfg: &CoreConfig) -> Vec<Arc<dyn Source>> {
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
    if cfg.sources.freehire.enabled {
        let mut s = FreehireSource::new().with_limit(cfg.sources.freehire.limit);
        if !cfg.sources.freehire.keywords.is_empty() {
            s = s.with_query(cfg.sources.freehire.keywords.clone());
        }
        if cfg.sources.freehire.remote_only {
            s = s.with_remote_only(true);
        }
        if let Some(region) = &cfg.sources.freehire.region {
            s = s.with_region(region.clone());
        }
        if let Some(days) = cfg.sources.freehire.jobage {
            s = s.with_jobage(days);
        }
        out.push(Arc::new(s));
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

/// Build all source adapters configured for a specific source name.
/// Multi-company ATS sources (Greenhouse, Lever, Ashby, Teamtailor) produce
/// one adapter per configured company.
pub fn build_sources_for_name(cfg: &CoreConfig, name: &str) -> Vec<Arc<dyn Source>> {
    let mut out = Vec::new();
    match name {
        "greenhouse" => {
            for company in &cfg.sources.greenhouse.companies {
                out.push(Arc::new(GreenhouseSource::new(company.clone())) as Arc<dyn Source>);
            }
        }
        "lever" => {
            for company in &cfg.sources.lever.companies {
                out.push(Arc::new(LeverSource::new(company.clone())) as Arc<dyn Source>);
            }
        }
        "ashby" => {
            for company in &cfg.sources.ashby.companies {
                out.push(
                    Arc::new(careerai_sources::AshbySource::new(company.clone()))
                        as Arc<dyn Source>,
                );
            }
        }
        "teamtailor" => {
            for company in &cfg.sources.teamtailor.companies {
                out.push(
                    Arc::new(careerai_sources::TeamtailorSource::new(company.clone()))
                        as Arc<dyn Source>,
                );
            }
        }
        "remotive" if cfg.sources.remotive.enabled => {
            let mut s = RemotiveSource::new();
            if let Some(cat) = &cfg.sources.remotive.category {
                s = s.with_category(cat.clone());
            }
            out.push(Arc::new(s));
        }
        "remoteok" if cfg.sources.remoteok.enabled => {
            out.push(Arc::new(RemoteOkSource::new()));
        }
        "freehire" if cfg.sources.freehire.enabled => {
            let mut s = FreehireSource::new().with_limit(cfg.sources.freehire.limit);
            if !cfg.sources.freehire.keywords.is_empty() {
                s = s.with_query(cfg.sources.freehire.keywords.clone());
            }
            if cfg.sources.freehire.remote_only {
                s = s.with_remote_only(true);
            }
            if let Some(region) = &cfg.sources.freehire.region {
                s = s.with_region(region.clone());
            }
            if let Some(days) = cfg.sources.freehire.jobage {
                s = s.with_jobage(days);
            }
            out.push(Arc::new(s));
        }
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
            out.push(Arc::new(s));
        }
        "indeed_rss" if cfg.sources.indeed_rss.enabled => {
            out.push(Arc::new(IndeedRssSource::new(
                cfg.sources.indeed_rss.clone(),
            )));
        }
        #[cfg(feature = "browser")]
        "linkedin-browser" if cfg.sources.linkedin_browser.enabled => {
            out.push(Arc::new(careerai_sources::LinkedinBrowserSource::new(
                cfg.sources.linkedin_browser.clone(),
            )));
        }
        #[cfg(not(feature = "browser"))]
        "linkedin-browser" if cfg.sources.linkedin_browser.enabled => {
            tracing::warn!(
                "linkedin_browser source is enabled in config but `browser` feature is OFF; \
                 rebuild with `cargo build --features browser` to activate it"
            );
        }
        _ => {
            for mcp in &cfg.sources.mcp {
                if mcp.enabled && mcp.name == name {
                    out.push(Arc::new(McpJobsSource::new(mcp.clone())));
                }
            }
        }
    }
    out
}

async fn discover_with_sources(
    pool: &SqlitePool,
    sources: Vec<Arc<dyn Source>>,
) -> Result<DiscoveryReport> {
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
                    match queries::insert_or_ignore(pool, &new).await {
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

    discover_with_sources(&pool, sources).await
}

pub async fn discover_one(root: &Path, cfg: &CoreConfig, source: &str) -> Result<DiscoveryReport> {
    let sources = build_sources_for_name(cfg, source);
    if sources.is_empty() {
        return Ok(DiscoveryReport::default());
    }
    let pool = open_pool(root).await?;
    discover_with_sources(&pool, sources).await
}
