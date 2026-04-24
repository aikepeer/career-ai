//! Discover / match / shortlist entry points called from `main.rs`.
//!
//! Kept off the main.rs clap tree so the CLI layer is purely dispatch and
//! the business logic is testable independently later.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::{info, warn};

use careerai_core::config::CoreConfig;
use careerai_db::models::NewListing;
use careerai_db::{pool_from_path, queries, SqlitePool};
use careerai_match::{
    classify, flatten_profile, rank_all, score_histogram, split_at_threshold, Decision,
    FilterRules, JaccardScorer,
};
use careerai_profile::Profile;
use careerai_sources::{
    GreenhouseSource, LeverSource, RawListing, RemoteOkSource, RemotiveSource, Source,
};

use careerai_core::state::ListingState;

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
    out
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
                        Ok((_, true)) => report.new_rows += 1,
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

#[derive(Debug, Default)]
pub struct DiscoveryReport {
    pub fetched: usize,
    pub new_rows: usize,
    pub duplicates: usize,
    pub errors: usize,
}

pub async fn match_all(root: &Path, cfg: &CoreConfig, tune: bool) -> Result<MatchReport> {
    let pool = open_pool(root).await?;
    let profile = load_profile(root)?;
    let rules = FilterRules::load(root).context("load rules")?;

    let discovered = queries::list_by_state(&pool, ListingState::Discovered, 10_000)
        .await
        .context("list discovered")?;
    info!(count = discovered.len(), "matching against profile");

    let raws: Vec<RawListing> = discovered
        .iter()
        .map(|l| RawListing {
            source: l.source.clone(),
            external_id: l.external_id.clone(),
            title: l.title.clone(),
            company: l.company.clone(),
            location: l.location.clone(),
            url: l.url.clone(),
            description: l.description.clone(),
            raw_json: l.raw_json.clone(),
        })
        .collect();

    // Apply hard filters first. Rejected listings still move off `discovered`
    // so we don't re-score them every run.
    let mut post_filter: Vec<(&careerai_db::models::Listing, RawListing)> = Vec::new();
    let mut filtered_out = 0usize;
    for (db_row, raw) in discovered.iter().zip(raws) {
        match classify(&raw, cfg, &rules) {
            Decision::Keep => post_filter.push((db_row, raw)),
            Decision::Reject(reason) => {
                filtered_out += 1;
                queries::transition(&pool, &db_row.id, ListingState::FilteredOut, Some(reason))
                    .await?;
            }
        }
    }

    let profile_text = flatten_profile(&profile);
    let raws_only: Vec<RawListing> = post_filter.iter().map(|(_, r)| r.clone()).collect();
    let ranked = rank_all(&JaccardScorer, &profile_text, &raws_only);

    if tune {
        // Tuning mode: don't persist, just return the histogram so the CLI
        // can print it.
        return Ok(MatchReport {
            filtered_out,
            shortlisted: 0,
            also_filtered: 0,
            histogram: score_histogram(&ranked),
        });
    }

    let threshold = cfg.matching.score_threshold;
    let (keep, drop) = split_at_threshold(ranked, threshold);

    for scored in &keep {
        let db_row = post_filter
            .iter()
            .find(|(_, r)| r.external_id == scored.listing.external_id)
            .map(|(d, _)| d)
            .context("bug: scored listing missing from post_filter map")?;
        queries::set_score(&pool, &db_row.id, f64::from(scored.score)).await?;
        queries::transition(
            &pool,
            &db_row.id,
            ListingState::Shortlisted,
            Some(&format!("score={:.3}", scored.score)),
        )
        .await?;
    }

    for scored in &drop {
        let db_row = post_filter
            .iter()
            .find(|(_, r)| r.external_id == scored.listing.external_id)
            .map(|(d, _)| d)
            .context("bug: below-threshold listing missing")?;
        queries::set_score(&pool, &db_row.id, f64::from(scored.score)).await?;
        queries::transition(
            &pool,
            &db_row.id,
            ListingState::FilteredOut,
            Some(&format!("below threshold ({:.3})", scored.score)),
        )
        .await?;
    }

    Ok(MatchReport {
        filtered_out,
        shortlisted: keep.len(),
        also_filtered: drop.len(),
        histogram: [(0.0, 0); 10],
    })
}

#[derive(Debug, Default)]
pub struct MatchReport {
    pub filtered_out: usize,
    pub shortlisted: usize,
    pub also_filtered: usize,
    pub histogram: [(f32, usize); 10],
}

pub async fn shortlist_show(root: &Path, limit: i64) -> Result<Vec<careerai_db::Listing>> {
    let pool = open_pool(root).await?;
    let rows = queries::list_by_state(&pool, ListingState::Shortlisted, limit)
        .await
        .context("list shortlisted")?;
    Ok(rows)
}

fn load_profile(root: &Path) -> Result<Profile> {
    let path = root.join("profile").join("profile.yaml");
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    Profile::from_yaml(&text).context("parse profile yaml")
}
