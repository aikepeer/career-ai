//! Discover stage: pull listings from configured sources and persist them.
//! Extracted from `lib.rs` to keep that file under the project's 300-LOC cap.

use std::path::Path;

use anyhow::Result;
use tracing::{info, warn};

use careerai_core::config::CoreConfig;
use careerai_db::models::NewListing;
use careerai_db::queries;

use crate::{build_sources, open_pool, DiscoveryReport};

/// Pull listings from every enabled source, deduplicate by external_id, and
/// persist new rows. Optionally filtered to `source_filter` names.
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

/// Run discovery for a single configured source. Delegates to `discover_all`
/// with a one-element filter.
pub async fn discover_one(root: &Path, cfg: &CoreConfig, source: &str) -> Result<DiscoveryReport> {
    let filter = [source.to_owned()];
    discover_all(root, cfg, &filter).await
}
