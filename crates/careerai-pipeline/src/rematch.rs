//! Re-match stage — re-score shortlisted listings against a new threshold.
//! Extracted from `match_.rs` to keep file sizes under the project cap.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::{info, warn};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_match::{flatten_profile, rank_all, split_at_threshold, JaccardScorer};
use careerai_sources::RawListing;

use crate::{load_profile, open_pool};

/// Report from a re-match pass over already-shortlisted listings.
#[derive(Debug, Default)]
pub struct RematchReport {
    /// How many shortlisted listings were re-scored.
    pub rescored: usize,
    /// How many were demoted back to `filtered_out` (score < threshold).
    pub demoted: usize,
}

/// Re-score all listings currently in `shortlisted` state and demote any
/// that fall below the current `score_threshold` back to `filtered_out`.
///
/// Useful when the operator raises `score_threshold` in `config/local.yaml`
/// and wants to retroactively prune the shortlist without a full
/// discover → match cycle.
///
/// Listings in `tailored`, `rendered`, or later stages are intentionally
/// **not** touched — those have already had work done on them.
pub async fn rematch_shortlisted(root: &Path, cfg: &CoreConfig) -> Result<RematchReport> {
    let pool = open_pool(root).await?;
    let profile = load_profile(root)?;
    let shortlisted = queries::list_by_state(&pool, ListingState::Shortlisted, 10_000)
        .await
        .context("list shortlisted")?;

    if shortlisted.is_empty() {
        info!("rematch-shortlisted: no shortlisted listings to re-score");
        return Ok(RematchReport::default());
    }

    info!(count = shortlisted.len(), "rematch-shortlisted: re-scoring");

    let raws: Vec<RawListing> = shortlisted
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

    let profile_text = flatten_profile(&profile);
    let ranked = rank_all(&JaccardScorer, &profile_text, &raws);
    let threshold = cfg.matching.score_threshold;
    let (_, drop) = split_at_threshold(ranked, threshold);

    let id_map: HashMap<(String, String), &careerai_db::models::Listing> = shortlisted
        .iter()
        .map(|l| ((l.source.clone(), l.external_id.clone()), l))
        .collect();

    let rescored = shortlisted.len();
    let mut demoted = 0usize;

    for scored in &drop {
        let Some(db_row) = id_map.get(&(
            scored.listing.source.clone(),
            scored.listing.external_id.clone(),
        )) else {
            warn!(
                source = %scored.listing.source,
                external_id = %scored.listing.external_id,
                "rematch: scored listing not found in id_map — skipping"
            );
            continue;
        };
        queries::transition(
            &pool,
            &db_row.id,
            ListingState::FilteredOut,
            Some(&format!(
                "rematch: score {:.3} below new threshold {:.3}",
                scored.score, threshold
            )),
        )
        .await?;
        demoted += 1;
        info!(
            id = %db_row.id,
            title = %scored.listing.title,
            company = %scored.listing.company,
            score = scored.score,
            threshold,
            "rematch: demoted shortlisted -> filtered_out"
        );
    }

    Ok(RematchReport { rescored, demoted })
}
