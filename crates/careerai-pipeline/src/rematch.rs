//! Re-match stage — re-score shortlisted listings against a new threshold.
//! Extracted from `match_.rs` to keep file sizes under the project cap.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_match::{flatten_profile, JaccardScorer};
use careerai_sources::RawListing;

use crate::{load_profile, open_pool};

/// Report from a re-match pass over listings.
#[derive(Debug, Default)]
pub struct RematchReport {
    /// How many listings were re-evaluated.
    pub rescored: usize,
    /// How many were demoted back to `filtered_out` (score < threshold).
    pub demoted: usize,
    /// How many were promoted from `filtered_out` to `shortlisted` (score >= threshold).
    pub promoted: usize,
}

/// Re-score listings against the current `score_threshold`:
/// 1. Demotes `shortlisted` listings that fall below the threshold to `filtered_out`.
/// 2. Promotes `filtered_out` listings that pass filter rules and meet/exceed the threshold to `shortlisted`.
///
/// Listings in `tailored`, `rendered`, or later stages are intentionally
/// **not** touched — those have already had work done on them.
pub async fn rematch_shortlisted(root: &Path, cfg: &CoreConfig) -> Result<RematchReport> {
    let pool = open_pool(root).await?;
    let profile = load_profile(root)?;
    let rules = careerai_match::FilterRules::load(root).context("load rules")?;
    let threshold = cfg.matching.score_threshold;
    let profile_text = flatten_profile(&profile);

    let shortlisted = queries::list_by_state(&pool, ListingState::Shortlisted, 10_000)
        .await
        .context("list shortlisted")?;

    let filtered_out = queries::list_by_state(&pool, ListingState::FilteredOut, 10_000)
        .await
        .context("list filtered_out")?;

    let rescored = shortlisted.len() + filtered_out.len();
    let mut demoted = 0usize;
    let mut promoted = 0usize;

    // 1. Demote shortlisted listings that fail classifier rules or fall below threshold
    if !shortlisted.is_empty() {
        for db_row in &shortlisted {
            let raw = RawListing {
                source: db_row.source.clone(),
                external_id: db_row.external_id.clone(),
                title: db_row.title.clone(),
                company: db_row.company.clone(),
                location: db_row.location.clone(),
                url: db_row.url.clone(),
                description: db_row.description.clone(),
                raw_json: db_row.raw_json.clone(),
            };

            match careerai_match::classify(&raw, cfg, &rules) {
                careerai_match::Decision::Reject(reason) => {
                    queries::transition(
                        &pool,
                        &db_row.id,
                        ListingState::FilteredOut,
                        Some(&format!("rematch filter reject: {reason}")),
                    )
                    .await?;
                    demoted += 1;
                }
                careerai_match::Decision::Keep => {
                    let score = careerai_match::Scorer::score(&JaccardScorer, &profile_text, &raw);
                    let _ = queries::set_score(&pool, &db_row.id, f64::from(score)).await;
                    if score < threshold {
                        queries::transition(
                            &pool,
                            &db_row.id,
                            ListingState::FilteredOut,
                            Some(&format!(
                                "rematch: score {:.3} below threshold {:.3}",
                                score, threshold
                            )),
                        )
                        .await?;
                        demoted += 1;
                    }
                }
            }
        }
    }

    // 2. Promote filtered_out listings that meet threshold and pass rules
    if !filtered_out.is_empty() {
        for db_row in &filtered_out {
            let raw = RawListing {
                source: db_row.source.clone(),
                external_id: db_row.external_id.clone(),
                title: db_row.title.clone(),
                company: db_row.company.clone(),
                location: db_row.location.clone(),
                url: db_row.url.clone(),
                description: db_row.description.clone(),
                raw_json: db_row.raw_json.clone(),
            };

            // Must pass hard filter rules
            if let careerai_match::Decision::Keep = careerai_match::classify(&raw, cfg, &rules) {
                let score = careerai_match::Scorer::score(&JaccardScorer, &profile_text, &raw);
                let _ = queries::set_score(&pool, &db_row.id, f64::from(score)).await;
                if score >= threshold {
                    queries::transition(
                        &pool,
                        &db_row.id,
                        ListingState::Shortlisted,
                        Some(&format!(
                            "rematch: score {:.3} >= threshold {:.3}",
                            score, threshold
                        )),
                    )
                    .await?;
                    promoted += 1;
                }
            }
        }
    }

    info!(
        rescored,
        demoted,
        promoted,
        threshold,
        "rematch completed"
    );

    Ok(RematchReport {
        rescored,
        demoted,
        promoted,
    })
}
