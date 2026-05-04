//! Three-way sync partition: add / keep / remove diff between the seed
//! list and the currently-configured ATS company lists.

use futures::stream::{FuturesUnordered, StreamExt};
use tracing::{debug, warn};

use careerai_core::config::CoreConfig;

use super::probe::{probe_one, BaseUrls, CompanyHit, ProbeOutcome, PROBE_CONCURRENCY, PROBE_TIMEOUT};
use super::seed::{AtsVendor, SeedEntry, SyncError};

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
            probes.push(probe_one(entry, bases.clone(), domains.clone(), PROBE_TIMEOUT));
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
            probes.push(probe_one(entry, bases.clone(), domains.clone(), PROBE_TIMEOUT));
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
            // A probe failure (timeout, 5xx, parse error) is "no
            // signal" rather than "confirmed zero matches" —
            // recommending removal in that case would silently drop
            // user-configured companies on a flaky network.
            let in_seed = seed
                .iter()
                .any(|e| e.ats == ats && e.slug.eq_ignore_ascii_case(slug));
            let matched_now = all_hit_slugs
                .iter()
                .any(|(a, s)| *a == ats && s.eq_ignore_ascii_case(slug));
            let probe_failed = report
                .probe_failures
                .iter()
                .any(|(s, _)| s.eq_ignore_ascii_case(slug));
            if in_seed && !matched_now && !probe_failed {
                report.remove.push((ats, slug.clone()));
            }
        }
    }
    report.remove.sort();
    report.probe_failures.sort();

    Ok(report)
}
