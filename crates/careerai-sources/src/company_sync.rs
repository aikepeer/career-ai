//! Auto-discover ATS companies whose currently-open jobs match the
//! user's `domains:` keywords.
//!
//! `careerai sources sync` runs this module against a seed list of
//! known-public Greenhouse / Lever / Ashby slugs (embedded via
//! `include_str!` from `templates/seed_companies.yaml`), probes each
//! board's public API, scores every JD against every domain in
//! `CoreConfig.domains`, and produces a three-way diff against the
//! currently-configured `sources.<ats>.companies` lists.
//!
//! Public surface:
//! - [`SyncReport`] — the diff (`add` / `keep` / `remove` /
//!   `probe_failures`).
//! - [`CompanyHit`] — one matched company entry.
//! - [`AtsVendor`] — Greenhouse | Lever | Ashby.
//! - [`SeedEntry`] — the on-disk schema of `seed_companies.yaml`.
//! - [`load_embedded_seed`] — convenience parser for the bundled list.
//! - [`sync`] — the entry point.
//!
//! Concurrency: up to [`PROBE_CONCURRENCY`] companies are probed in
//! parallel. Each probe has a [`PROBE_TIMEOUT`] hard ceiling. HTTP
//! errors are soft-failed (logged, recorded in
//! `report.probe_failures`); the sync never aborts on a single
//! upstream blip.

use std::time::Duration;

use careerai_core::config::CoreConfig;
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::ashby::AshbySource;
use crate::base::{RawListing, Source, SourceError};
use crate::greenhouse::GreenhouseSource;
use crate::lever::LeverSource;

/// Per-company HTTP timeout. The board APIs we hit (Greenhouse, Lever,
/// Ashby) all return in <2s on a warm cache; 10s is a generous ceiling
/// that still keeps a 60-slug sync under a minute.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Cap on the number of in-flight HTTP probes. Above this, we queue.
/// Tuned to keep the sync responsive without hammering a single
/// upstream (each ATS API is shared across many slugs).
pub const PROBE_CONCURRENCY: usize = 8;

/// Embedded seed list, parsed from
/// `crates/careerai-sources/src/templates/seed_companies.yaml` at
/// compile time.
pub const EMBEDDED_SEED_YAML: &str = include_str!("templates/seed_companies.yaml");

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("seed parse: {0}")]
    SeedParse(#[from] serde_yaml::Error),
}

/// One ATS vendor. Stable on the wire — used as a YAML tag in the seed
/// file, so `kebab-case` matches the existing config style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AtsVendor {
    Greenhouse,
    Lever,
    Ashby,
}

impl AtsVendor {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
        }
    }
}

/// One row in `seed_companies.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedEntry {
    pub slug: String,
    pub ats: AtsVendor,
    /// Optional. Currently informational; the probe scores against
    /// every domain regardless. Future use: skip probes for domains
    /// we know in advance won't match.
    #[serde(default)]
    pub domain_hint: Vec<String>,
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

/// Three-way diff against the currently-configured ATS lists.
#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    /// Companies that match now and aren't in the user's config yet.
    pub add: Vec<CompanyHit>,
    /// Companies that match now and are already configured.
    pub keep: Vec<CompanyHit>,
    /// Slugs that are configured but no longer post matching jobs.
    /// Surfaced as a soft suggestion — not auto-removed.
    pub remove: Vec<(AtsVendor, String)>,
    /// `(slug, error)` pairs for companies the probe couldn't reach.
    pub probe_failures: Vec<(String, String)>,
}

/// Parse the embedded `seed_companies.yaml`. Convenience wrapper so
/// callers don't have to repeat the `serde_yaml::from_str` call.
pub fn load_embedded_seed() -> Result<Vec<SeedEntry>, SyncError> {
    let entries: Vec<SeedEntry> = serde_yaml::from_str(EMBEDDED_SEED_YAML)?;
    Ok(entries)
}

/// Run the sync. Probes each seed slug, scores listings against the
/// configured domains, and partitions results into add / keep / remove.
///
/// # Errors
///
/// Only seed-parse errors propagate. HTTP / parse errors during probes
/// are soft-failed and accumulated into `report.probe_failures`.
pub async fn sync(cfg: &CoreConfig, seed: &[SeedEntry]) -> Result<SyncReport, SyncError> {
    sync_with_base_urls(cfg, seed, &BaseUrls::default()).await
}

/// Same as [`sync`] but with overridable base URLs for tests.
pub async fn sync_with_base_urls(
    cfg: &CoreConfig,
    seed: &[SeedEntry],
    bases: &BaseUrls,
) -> Result<SyncReport, SyncError> {
    let domains = &cfg.domains;
    if domains.is_empty() {
        warn!("no domains configured — sync would match nothing; aborting early");
        return Ok(SyncReport::default());
    }

    let configured_greenhouse: Vec<String> = cfg.sources.greenhouse.companies.clone();
    let configured_lever: Vec<String> = cfg.sources.lever.companies.clone();
    let configured_ashby: Vec<String> = cfg.sources.ashby.companies.clone();

    // Probe seeded slugs concurrently. We use a FuturesUnordered queue
    // bounded to PROBE_CONCURRENCY so we don't open 80 sockets at once.
    let mut probes: FuturesUnordered<_> = FuturesUnordered::new();
    let mut iter = seed.iter().cloned();
    for _ in 0..PROBE_CONCURRENCY {
        if let Some(entry) = iter.next() {
            probes.push(probe_one(entry, bases.clone(), domains.clone()));
        }
    }

    let mut hits: Vec<CompanyHit> = Vec::new();
    let mut probe_failures: Vec<(String, String)> = Vec::new();

    while let Some(outcome) = probes.next().await {
        match outcome {
            ProbeOutcome::Hit(hit) => hits.push(hit),
            ProbeOutcome::Miss(slug) => debug!(slug = %slug, "no domain match"),
            ProbeOutcome::Failure { slug, reason } => {
                warn!(slug = %slug, reason = %reason, "probe failed (soft)");
                probe_failures.push((slug, reason));
            }
        }
        if let Some(entry) = iter.next() {
            probes.push(probe_one(entry, bases.clone(), domains.clone()));
        }
    }

    // Partition into add / keep against the configured lists.
    let mut report = SyncReport {
        probe_failures,
        ..SyncReport::default()
    };
    for hit in hits {
        let configured = match hit.ats {
            AtsVendor::Greenhouse => &configured_greenhouse,
            AtsVendor::Lever => &configured_lever,
            AtsVendor::Ashby => &configured_ashby,
        };
        if configured.iter().any(|c| c.eq_ignore_ascii_case(&hit.slug)) {
            report.keep.push(hit);
        } else {
            report.add.push(hit);
        }
    }

    // Stable ordering for predictable CLI output + snapshot tests.
    report
        .add
        .sort_by(|a, b| a.ats.as_str().cmp(b.ats.as_str()).then(a.slug.cmp(&b.slug)));
    report
        .keep
        .sort_by(|a, b| a.ats.as_str().cmp(b.ats.as_str()).then(a.slug.cmp(&b.slug)));

    // Remove suggestions: configured slugs that aren't in the seed
    // list at all (we can't infer "no matches" for slugs we didn't
    // probe). The user added them by hand and they may now be stale.
    let all_hit_slugs: Vec<(AtsVendor, &str)> = report
        .add
        .iter()
        .chain(report.keep.iter())
        .map(|h| (h.ats, h.slug.as_str()))
        .collect();
    for (ats, configured) in [
        (AtsVendor::Greenhouse, &configured_greenhouse),
        (AtsVendor::Lever, &configured_lever),
        (AtsVendor::Ashby, &configured_ashby),
    ] {
        for slug in configured {
            // Surface a "consider removing" only when the slug is in
            // the seed (we probed it) and missed every domain. Slugs
            // outside the seed are out-of-scope — leave them alone.
            let in_seed = seed
                .iter()
                .any(|e| e.ats == ats && e.slug.eq_ignore_ascii_case(slug));
            let matched_now = all_hit_slugs
                .iter()
                .any(|(a, s)| *a == ats && s.eq_ignore_ascii_case(slug));
            if in_seed && !matched_now {
                report.remove.push((ats, slug.clone()));
            }
        }
    }
    report.remove.sort();
    report.probe_failures.sort();

    Ok(report)
}

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

enum ProbeOutcome {
    Hit(CompanyHit),
    Miss(String),
    Failure { slug: String, reason: String },
}

async fn probe_one(
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
fn score_listings(
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
fn listing_matches(l: &RawListing, keywords_any: &[String]) -> bool {
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_core::config::{CoreConfig, Domain};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn cfg_with_domains(domains: Vec<Domain>) -> CoreConfig {
        let mut cfg: CoreConfig =
            serde_yaml::from_str(careerai_core::config::EMBEDDED_DEFAULTS).unwrap();
        cfg.domains = domains;
        cfg.sources.greenhouse.companies.clear();
        cfg.sources.lever.companies.clear();
        cfg.sources.ashby.companies.clear();
        cfg
    }

    fn ai_ml_domain() -> Domain {
        Domain {
            name: "ai_ml".into(),
            keywords_any: vec!["machine learning".into(), "llm".into()],
        }
    }

    fn robotics_domain() -> Domain {
        Domain {
            name: "robotics_embedded".into(),
            keywords_any: vec!["robotics".into(), "embedded".into()],
        }
    }

    fn raw(title: &str, description: &str) -> RawListing {
        RawListing {
            source: "test".into(),
            external_id: "id".into(),
            title: title.into(),
            company: "co".into(),
            location: None,
            url: "u".into(),
            description: description.into(),
            raw_json: None,
        }
    }

    #[test]
    fn listing_matches_is_case_insensitive_across_title_and_description() {
        let l = raw("Senior LLM Engineer", "Build large language model agents.");
        let kws = vec!["LLM".to_string()];
        assert!(listing_matches(&l, &kws));

        let l2 = raw("Senior Backend", "We deploy machine learning systems.");
        let kws2 = vec!["machine learning".to_string()];
        assert!(listing_matches(&l2, &kws2));

        let l3 = raw("Senior Backend", "Plain ol web servers.");
        assert!(!listing_matches(&l3, &kws2));
    }

    #[test]
    fn empty_keywords_never_match() {
        let l = raw("LLM Engineer", "Build LLM agents.");
        assert!(!listing_matches(&l, &[]));
    }

    #[test]
    fn empty_keyword_string_is_skipped() {
        let l = raw("Backend Engineer", "boring");
        let kws = vec![String::new(), "backend".to_string()];
        assert!(listing_matches(&l, &kws));
    }

    #[test]
    fn score_listings_aggregates_across_domains() {
        let entry = SeedEntry {
            slug: "acme".into(),
            ats: AtsVendor::Greenhouse,
            domain_hint: vec![],
        };
        let listings = vec![
            raw("LLM Engineer", "machine learning systems"),
            raw("Robotics Engineer", "ROS2 and embedded"),
            raw("Marketing", "spreadsheet"),
        ];
        let domains = vec![ai_ml_domain(), robotics_domain()];
        let hit = score_listings(&entry, &listings, &domains).unwrap();
        assert_eq!(hit.slug, "acme");
        assert_eq!(hit.ats, AtsVendor::Greenhouse);
        assert_eq!(hit.matched_domains, vec!["ai_ml", "robotics_embedded"]);
        // listing 0 hits ai_ml (LLM + ml), listing 1 hits robotics
        // (embedded). One JD can match multiple keywords in one
        // domain — we count the listing once per domain.
        assert!(hit.matched_jobs >= 2, "matched_jobs={}", hit.matched_jobs);
    }

    #[test]
    fn score_listings_returns_none_when_nothing_matches() {
        let entry = SeedEntry {
            slug: "acme".into(),
            ats: AtsVendor::Lever,
            domain_hint: vec![],
        };
        let listings = vec![raw("Recruiter", "recruiting"), raw("Sales", "sales")];
        let hit = score_listings(&entry, &listings, &[ai_ml_domain()]);
        assert!(hit.is_none());
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // exercises every partition path
    async fn sync_partitions_add_keep_remove() {
        let server = MockServer::start().await;

        // Greenhouse: `acme` has an LLM job → should land in `add`.
        // Greenhouse: `pre-existing` (already configured) has an LLM
        // job → should land in `keep`. Greenhouse: `stale` (already
        // configured but seeded) has no matching job → should land
        // in `remove`. Greenhouse: `boring` (seeded) has no
        // matching job → should be silently ignored (a Miss, not a
        // remove because not configured).
        Mock::given(method("GET"))
            .and(path_regex(r"^/v1/boards/acme/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [{
                    "id": 1,
                    "title": "Senior LLM Engineer",
                    "absolute_url": "https://greenhouse.io/acme/1",
                    "content": "<p>Build agents.</p>"
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v1/boards/pre-existing/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [{
                    "id": 2,
                    "title": "ML Researcher",
                    "absolute_url": "https://greenhouse.io/pre-existing/2",
                    "content": "<p>machine learning</p>"
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r"^/v1/boards/(stale|boring)/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [{
                    "id": 3,
                    "title": "Recruiter",
                    "absolute_url": "https://greenhouse.io/stale/3",
                    "content": "<p>hiring</p>"
                }]
            })))
            .mount(&server)
            .await;

        // Lever: `lev-co` has matching JD → add.
        Mock::given(method("GET"))
            .and(path_regex(r"^/v0/postings/lev-co"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                    "id": "p1",
                    "text": "Robotics Engineer",
                    "descriptionPlain": "ROS2 and embedded systems.",
                    "categories": {},
                    "hostedUrl": "https://lever.co/lev-co/p1"
                }])),
            )
            .mount(&server)
            .await;

        // Ashby: `ash-co` has matching JD → add.
        Mock::given(method("GET"))
            .and(path_regex(r"^/posting-api/job-board/ash-co"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [{
                    "id": "a1",
                    "title": "Applied ML",
                    "location": "Remote",
                    "jobUrl": "https://ashby.co/ash-co/a1",
                    "descriptionHtml": "<p>We use machine learning end to end.</p>"
                }]
            })))
            .mount(&server)
            .await;

        // Build cfg with `pre-existing` and `stale` already configured.
        let mut cfg = cfg_with_domains(vec![ai_ml_domain(), robotics_domain()]);
        cfg.sources.greenhouse.companies =
            vec!["pre-existing".into(), "stale".into(), "manual-add".into()];

        // Seed slugs (note `stale` is in the seed, `manual-add` is
        // not — so `manual-add` should NOT show up in `remove`).
        let seed = vec![
            SeedEntry {
                slug: "acme".into(),
                ats: AtsVendor::Greenhouse,
                domain_hint: vec![],
            },
            SeedEntry {
                slug: "pre-existing".into(),
                ats: AtsVendor::Greenhouse,
                domain_hint: vec![],
            },
            SeedEntry {
                slug: "stale".into(),
                ats: AtsVendor::Greenhouse,
                domain_hint: vec![],
            },
            SeedEntry {
                slug: "boring".into(),
                ats: AtsVendor::Greenhouse,
                domain_hint: vec![],
            },
            SeedEntry {
                slug: "lev-co".into(),
                ats: AtsVendor::Lever,
                domain_hint: vec![],
            },
            SeedEntry {
                slug: "ash-co".into(),
                ats: AtsVendor::Ashby,
                domain_hint: vec![],
            },
        ];

        let bases = BaseUrls {
            greenhouse: server.uri(),
            lever: server.uri(),
            ashby: server.uri(),
        };
        let report = sync_with_base_urls(&cfg, &seed, &bases).await.unwrap();

        // add: acme (gh), lev-co (lever), ash-co (ashby)
        let add_slugs: Vec<_> = report.add.iter().map(|h| h.slug.clone()).collect();
        assert!(
            add_slugs.contains(&"acme".to_string()),
            "add: {add_slugs:?}"
        );
        assert!(
            add_slugs.contains(&"lev-co".to_string()),
            "add: {add_slugs:?}"
        );
        assert!(
            add_slugs.contains(&"ash-co".to_string()),
            "add: {add_slugs:?}"
        );

        // keep: pre-existing
        let keep_slugs: Vec<_> = report.keep.iter().map(|h| h.slug.clone()).collect();
        assert!(
            keep_slugs.contains(&"pre-existing".to_string()),
            "keep: {keep_slugs:?}"
        );

        // remove: stale (in seed, configured, but missed). manual-add
        // (configured but not in seed) must NOT appear.
        let remove_slugs: Vec<_> = report.remove.iter().map(|(_, s)| s.clone()).collect();
        assert!(
            remove_slugs.contains(&"stale".to_string()),
            "remove: {remove_slugs:?}"
        );
        assert!(
            !remove_slugs.contains(&"manual-add".to_string()),
            "remove: {remove_slugs:?}"
        );

        // probe_failures should be empty (all probes returned 200).
        assert!(
            report.probe_failures.is_empty(),
            "failures: {:?}",
            report.probe_failures
        );
    }

    #[tokio::test]
    async fn sync_records_probe_failures_without_aborting() {
        let server = MockServer::start().await;
        // good: returns a matching JD
        Mock::given(method("GET"))
            .and(path_regex(r"^/v1/boards/good/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jobs": [{
                    "id": 1,
                    "title": "ML Engineer",
                    "absolute_url": "https://greenhouse.io/good/1",
                    "content": "machine learning"
                }]
            })))
            .mount(&server)
            .await;
        // bad: returns 500. Probe should soft-fail.
        Mock::given(method("GET"))
            .and(path_regex(r"^/v1/boards/bad/jobs"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let cfg = cfg_with_domains(vec![ai_ml_domain()]);
        let seed = vec![
            SeedEntry {
                slug: "good".into(),
                ats: AtsVendor::Greenhouse,
                domain_hint: vec![],
            },
            SeedEntry {
                slug: "bad".into(),
                ats: AtsVendor::Greenhouse,
                domain_hint: vec![],
            },
        ];
        let bases = BaseUrls {
            greenhouse: server.uri(),
            lever: server.uri(),
            ashby: server.uri(),
        };
        let report = sync_with_base_urls(&cfg, &seed, &bases).await.unwrap();
        assert_eq!(report.add.len(), 1);
        assert_eq!(report.add[0].slug, "good");
        assert_eq!(report.probe_failures.len(), 1);
        assert_eq!(report.probe_failures[0].0, "bad");
    }

    #[test]
    fn embedded_seed_parses_cleanly() {
        let entries = load_embedded_seed().expect("seed yaml is well-formed");
        assert!(!entries.is_empty(), "seed file should be non-empty");
        // Verify every entry has a non-empty slug + a valid ats.
        for e in &entries {
            assert!(!e.slug.is_empty(), "empty slug");
        }
        // Distribution sanity: at least one of each ats kind.
        let has_gh = entries.iter().any(|e| e.ats == AtsVendor::Greenhouse);
        let has_lev = entries.iter().any(|e| e.ats == AtsVendor::Lever);
        let has_ash = entries.iter().any(|e| e.ats == AtsVendor::Ashby);
        assert!(has_gh && has_lev && has_ash, "missing ATS coverage");
    }

    #[test]
    fn ats_vendor_str() {
        assert_eq!(AtsVendor::Greenhouse.as_str(), "greenhouse");
        assert_eq!(AtsVendor::Lever.as_str(), "lever");
        assert_eq!(AtsVendor::Ashby.as_str(), "ashby");
    }
}
