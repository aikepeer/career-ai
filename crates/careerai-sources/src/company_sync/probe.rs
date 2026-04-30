//! Per-company HTTP probe + keyword scoring. Drives `FuturesUnordered`
//! concurrency, calls Greenhouse / Lever / Ashby `discover()`, and
//! scores the returned listings against configured domains.

use std::time::Duration;

use crate::ashby::AshbySource;
use crate::base::{RawListing, Source, SourceError};
use crate::greenhouse::GreenhouseSource;
use crate::lever::LeverSource;

use super::seed::{AtsVendor, SeedEntry};

/// Per-company HTTP timeout. The board APIs we hit (Greenhouse, Lever,
/// Ashby) all return in <2s on a warm cache; 10s is a generous ceiling
/// that still keeps a 60-slug sync under a minute.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Cap on the number of in-flight HTTP probes. Above this, we queue.
/// Tuned to keep the sync responsive without hammering a single
/// upstream (each ATS API is shared across many slugs).
pub const PROBE_CONCURRENCY: usize = 8;

/// Optional override map for the public ATS endpoints. Production
/// callers use `Default::default()`; integration tests point each
/// entry at a `wiremock::MockServer`.
#[derive(Debug, Clone)]
pub struct BaseUrls {
    pub greenhouse: String,
    pub lever: String,
    pub ashby: String,
}

impl Default for BaseUrls {
    fn default() -> Self {
        Self {
            greenhouse: "https://boards-api.greenhouse.io".to_string(),
            lever: "https://api.lever.co".to_string(),
            ashby: "https://api.ashbyhq.com".to_string(),
        }
    }
}

/// One company that matched at least one configured domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyHit {
    pub slug: String,
    pub ats: AtsVendor,
    /// Domain names whose `keywords_any` matched at least one open
    /// listing on this company's board.
    pub matched_domains: Vec<String>,
    /// Total open listings (across all domains) that matched. A single
    /// listing can count toward multiple domains; this field is the
    /// sum across domains.
    pub matched_jobs: u32,
}

pub(crate) enum ProbeOutcome {
    Hit(CompanyHit),
    Miss(String),
    Failure { slug: String, reason: String },
}

pub(crate) async fn probe_one(
    entry: SeedEntry,
    bases: BaseUrls,
    domains: Vec<careerai_core::config::Domain>,
) -> ProbeOutcome {
    let probe = tokio::time::timeout(PROBE_TIMEOUT, fetch_listings(&entry, &bases)).await;
    let listings = match probe {
        Ok(Ok(rows)) => rows,
        Ok(Err(e)) => {
            return ProbeOutcome::Failure {
                slug: entry.slug,
                reason: e.to_string(),
            };
        }
        Err(_) => {
            return ProbeOutcome::Failure {
                slug: entry.slug,
                reason: format!("timeout after {}s", PROBE_TIMEOUT.as_secs()),
            };
        }
    };
    score_listings(&entry, &listings, &domains)
        .map(ProbeOutcome::Hit)
        .unwrap_or(ProbeOutcome::Miss(entry.slug))
}

async fn fetch_listings(
    entry: &SeedEntry,
    bases: &BaseUrls,
) -> Result<Vec<RawListing>, SourceError> {
    match entry.ats {
        AtsVendor::Greenhouse => {
            GreenhouseSource::new(entry.slug.clone())
                .with_base_url(bases.greenhouse.clone())
                .discover()
                .await
        }
        AtsVendor::Lever => {
            LeverSource::new(entry.slug.clone())
                .with_base_url(bases.lever.clone())
                .discover()
                .await
        }
        AtsVendor::Ashby => {
            AshbySource::new(entry.slug.clone())
                .with_base_url(bases.ashby.clone())
                .discover()
                .await
        }
    }
}

/// Score a slate of listings against every configured domain. Returns
/// `Some(hit)` iff at least one domain matched at least one listing.
pub(crate) fn score_listings(
    entry: &SeedEntry,
    listings: &[RawListing],
    domains: &[careerai_core::config::Domain],
) -> Option<CompanyHit> {
    let mut matched_domains: Vec<String> = Vec::new();
    let mut total: u32 = 0;
    for domain in domains {
        let count = listings
            .iter()
            .filter(|l| listing_matches(l, &domain.keywords_any))
            .count();
        if count > 0 {
            matched_domains.push(domain.name.clone());
            total = total.saturating_add(u32::try_from(count).unwrap_or(u32::MAX));
        }
    }
    if matched_domains.is_empty() {
        None
    } else {
        Some(CompanyHit {
            slug: entry.slug.clone(),
            ats: entry.ats,
            matched_domains,
            matched_jobs: total,
        })
    }
}

/// Does any keyword match the listing? Case-insensitive substring
/// across `title` + `description`. Empty keyword lists never match —
/// silently ignored so a domain block with `keywords_any: []` doesn't
/// false-positive every probe.
pub(crate) fn listing_matches(l: &RawListing, keywords_any: &[String]) -> bool {
    if keywords_any.is_empty() {
        return false;
    }
    let title_lower = l.title.to_ascii_lowercase();
    let desc_lower = l.description.to_ascii_lowercase();
    for kw in keywords_any {
        let kw_lower = kw.to_ascii_lowercase();
        if kw_lower.is_empty() {
            continue;
        }
        if title_lower.contains(&kw_lower) || desc_lower.contains(&kw_lower) {
            return true;
        }
    }
    false
}
