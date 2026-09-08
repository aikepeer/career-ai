//! Match stage — filter rules + scoring against shortlisted listings.
//! Extracted from `lib.rs` to keep that file under the 300-LOC cap.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::{info, warn};

use careerai_core::config::CoreConfig;
use careerai_core::state::ListingState;
use careerai_db::queries;
use careerai_match::{
    analyze_skill_gaps, assess_legitimacy, classify, flatten_profile, match_breakdown, rank_all,
    score_histogram, split_at_threshold, Decision, FilterRules, JDSkillGapReport, JaccardScorer,
    LegitimacyScore, MatchBreakdown,
};
use careerai_notify::{NotifyEvent, Pipeline as NotifyPipeline, Severity};
use careerai_sources::RawListing;

use crate::{load_profile, open_pool};

#[derive(Debug, Default)]
pub struct MatchReport {
    pub filtered_out: usize,
    pub shortlisted: usize,
    pub also_filtered: usize,
    pub histogram: [(f32, usize); 10],
    pub skill_gap: Option<JDSkillGapReport>,
}

#[allow(clippy::too_many_lines)] // single orchestration step; splitting
                                 // into helpers would just shuffle state
                                 // through extra parameters.
pub async fn match_all(root: &Path, cfg: &CoreConfig, tune: bool) -> Result<MatchReport> {
    let pool = open_pool(root).await?;
    let profile = load_profile(root)?;
    let rules = FilterRules::load(root).context("load rules")?;

    // Surface the must_include_skills filter at the top of every match
    // run. Operators who typo a skill in local.yaml otherwise see
    // "filtered_out: 100" with zero diagnostic. This one log line tells
    // them which filter is active before any work starts.
    if !cfg.matching.must_include_skills.is_empty() {
        info!(
            skills = ?cfg.matching.must_include_skills,
            "must_include_skills filter active — listings missing all of these will be rejected before scoring",
        );
    }

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
    let profile_text = flatten_profile(&profile);
     // F02: Collect domain keywords once for match-breakdown computation.
    let domain_keywords: Vec<String> = cfg
        .domains
        .iter()
        .flat_map(|d| d.keywords_any.iter().cloned())
        .collect();

     // Apply hard filters first. When not in tune mode, rejected listings are
     // transitioned to `FilteredOut` so they are skipped on subsequent runs.
    let mut post_filter: Vec<(&careerai_db::models::Listing, RawListing)> = Vec::new();
    let mut filtered_out = 0usize;
    for (db_row, raw) in discovered.iter().zip(raws) {
        match classify(&raw, cfg, &rules) {
            Decision::Keep => post_filter.push((db_row, raw)),
            Decision::Reject(reason) => {
                filtered_out += 1;
                if !tune {
                    let breakdown = match_breakdown(&profile_text, &raw, &domain_keywords);
                    persist_match_reasons(
                        &pool, &db_row.id, 0.0, &breakdown, Some(reason), None,
                    )
                    .await?;
                    queries::transition(&pool, &db_row.id, ListingState::FilteredOut, Some(reason))
                        .await?;
                }
            }
        }
    }

    // Build a lookup map so scored listings can find their DB row
    // in O(1) instead of scanning post_filter linearly (O(n²) before).
    let lookup: HashMap<(String, String), &careerai_db::models::Listing> = post_filter
        .iter()
        .map(|(db, raw)| ((raw.source.clone(), raw.external_id.clone()), *db))
        .collect();
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
            skill_gap: None,
        });
    }

    let threshold = cfg.matching.score_threshold;
    let (keep, drop) = split_at_threshold(ranked, threshold);

    // Build the notify pipeline once per match run, not per listing.
    // `from_config` returning Err (e.g. malformed channel config) is
    // logged and treated as "no channels" so a config bug never kills
    // a match run; high-score notifications are best-effort.
    let notify = match NotifyPipeline::from_config(&cfg.notify) {
        Ok(p) => Some(p),
        Err(e) => {
            warn!(error = %e, "notify pipeline init failed; skipping high-score alerts");
            None
        }
    };
    let notify_threshold = cfg.matching.notify_threshold;

    for scored in &keep {
        let db_row = lookup
            .get(&(
                scored.listing.source.clone(),
                scored.listing.external_id.clone(),
            ))
            .copied()
            .context("bug: scored listing missing from post_filter map")?;

        // apply_once_at_company (AIHawk): a company with an existing
        // application is never shortlisted again — the operator already
        // has a live shot there.
        if cfg.matching.apply_once_at_company
            && queries::has_application_for_company(&pool, &scored.listing.company).await?
        {
            filtered_out += 1;
            queries::transition(
                &pool,
                &db_row.id,
                ListingState::FilteredOut,
                Some("apply-once company"),
            )
            .await?;
            continue;
        }

        queries::set_score(&pool, &db_row.id, f64::from(scored.score)).await?;

        // F02: Persist structured match reasons for explainable match cards.
        let breakdown = match_breakdown(&profile_text, scored.listing, &domain_keywords);
        let legit = assess_legitimacy(scored.listing);
        let (legit_tier, legit_score) = legitimacy_to_tier_score(legit);
        persist_match_reasons(
            &pool,
            &db_row.id,
            scored.score,
            &breakdown,
            None,
            Some((legit_tier, legit_score)),
        )
        .await?;

        queries::transition(
            &pool,
            &db_row.id,
            ListingState::Shortlisted,
            Some(&format!("score={:.3}", scored.score)),
        )
        .await?;

        // Fire HighScoreMatch when score crosses the configured
        // notify threshold. We only get here for listings that are
        // currently in `discovered` state (match_all only walks
        // discovered rows), so this naturally won't double-fire on
        // re-runs — once a listing is shortlisted, it's no longer
        // a candidate for re-scoring without manual rollback.
        if let Some(pipe) = &notify {
            fire_high_score_if_above(
                pipe,
                &db_row.id,
                &scored.listing.title,
                &scored.listing.company,
                scored.score,
                notify_threshold,
            )
            .await;
        }
    }

    for scored in &drop {
        let db_row = lookup
            .get(&(
                scored.listing.source.clone(),
                scored.listing.external_id.clone(),
            ))
            .copied()
            .context("bug: below-threshold listing missing")?;
        queries::set_score(&pool, &db_row.id, f64::from(scored.score)).await?;
        let breakdown = match_breakdown(&profile_text, scored.listing, &domain_keywords);
        let filter_reason = format!("below threshold ({:.3})", scored.score);
        persist_match_reasons(
            &pool,
            &db_row.id,
            scored.score,
            &breakdown,
            Some(&filter_reason),
            None,
        )
        .await?;
        queries::transition(
            &pool,
            &db_row.id,
            ListingState::FilteredOut,
            Some(&filter_reason),
        )
        .await?;
    }

    // Skill gap analysis: cross-reference JD texts from shortlisted
    // listings against the user's skills to surface missing skills.
    let user_skills: Vec<String> = profile.skills.all_skill_names().map(String::from).collect();
    let jd_texts: Vec<String> = keep.iter().map(|s| s.listing.description.clone()).collect();
    let skill_gap = analyze_skill_gaps(&jd_texts, &user_skills);

    Ok(MatchReport {
        filtered_out,
        shortlisted: keep.len(),
        also_filtered: drop.len(),
        histogram: [(0.0, 0); 10],
        skill_gap: Some(skill_gap),
    })
}

/// Fire a `HighScoreMatch` notification when `score >= threshold`.
/// Pulled out so the threshold check is unit-testable against a
/// mock channel without standing up a full DB + match run.
/// Returns whether an event was fired.
async fn fire_high_score_if_above(
    pipe: &NotifyPipeline,
    listing_id: &str,
    title: &str,
    company: &str,
    score: f32,
    threshold: f32,
) -> bool {
    if score < threshold {
        return false;
    }
    pipe.fire(
        NotifyEvent::HighScoreMatch {
            listing_id: listing_id.to_string(),
            title: title.to_string(),
            company: company.to_string(),
            score,
        },
        Severity::Warning,
    )
    .await;
    true
}

/// Per-source match wrapper used by the scheduler's cron tick.
///
/// Matching is naturally global — `match_all` walks every `discovered`-state
/// row regardless of source, and the filter/score logic doesn't read the
/// `source` column. So `match_one` just delegates to `match_all`. The
/// `_source` argument is accepted for symmetry with `discover_one` and to
/// give the scheduler a place to attach span context per tick.
pub async fn match_one(root: &Path, cfg: &CoreConfig, _source: &str) -> Result<MatchReport> {
    match_all(root, cfg, false).await
}

/// F02: Persist structured match reasons to the `match_reasons` table.
/// Called for every listing that passes through the match pipeline —
/// shortlisted, below-threshold, and filtered-out — so the dashboard
/// can always explain *why* a listing got its score or was rejected.
async fn persist_match_reasons(
    pool: &sqlx::SqlitePool,
    listing_id: &str,
    score: f32,
    breakdown: &MatchBreakdown,
    filter_reason: Option<&str>,
    legitimacy: Option<(&str, f32)>,
) -> Result<()> {
    let matched_json = serde_json::to_string(&breakdown.matched_keywords)?;
    let missing_json = serde_json::to_string(&breakdown.missing_keywords)?;
    let (legit_tier, legit_score) = legitimacy.unzip();
    queries::upsert_match_reasons(
        pool,
        listing_id,
        score,
        &matched_json,
        &missing_json,
        filter_reason,
        legit_tier,
        legit_score,
        None,
    )
    .await?;
    Ok(())
}

/// Map a [`LegitimacyScore`] to a display tier string and a heuristic
/// numeric score for the match-reasons card. The numeric score is a
/// rough ordering aid, not a calibrated probability.
fn legitimacy_to_tier_score(legit: LegitimacyScore) -> (&'static str, f32) {
    match legit {
        LegitimacyScore::HighConfidence => ("legit", 0.9),
        LegitimacyScore::ProceedWithCaution => ("caution", 0.5),
        LegitimacyScore::Suspicious => ("suspicious", 0.15),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod high_score_notify_tests {
    use super::*;
    use async_trait::async_trait;
    use careerai_notify::Notifier;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct CapturingNotifier {
        captured: Mutex<Vec<NotifyEvent>>,
    }

    #[async_trait]
    impl Notifier for CapturingNotifier {
        fn name(&self) -> &'static str {
            "capturing"
        }
        async fn notify(
            &self,
            event: &NotifyEvent,
            _severity: Severity,
        ) -> std::result::Result<(), careerai_notify::NotifyError> {
            self.captured.lock().expect("lock").push(event.clone());
            Ok(())
        }
    }

    fn pipeline_with_capture() -> (NotifyPipeline, Arc<CapturingNotifier>) {
        let cap = Arc::new(CapturingNotifier::default());
        let pipe = NotifyPipeline::with_channels(vec![cap.clone()], Severity::Info);
        (pipe, cap)
    }

    #[tokio::test]
    async fn score_above_threshold_fires_event() {
        let (pipe, cap) = pipeline_with_capture();
        let fired =
            fire_high_score_if_above(&pipe, "lst-1", "ML Engineer", "Acme", 0.95, 0.85).await;
        assert!(fired);
        let captured = cap.captured.lock().expect("lock");
        assert_eq!(captured.len(), 1);
        match &captured[0] {
            NotifyEvent::HighScoreMatch {
                listing_id,
                title,
                company,
                score,
            } => {
                assert_eq!(listing_id, "lst-1");
                assert_eq!(title, "ML Engineer");
                assert_eq!(company, "Acme");
                assert!((score - 0.95).abs() < 1e-6);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn score_at_threshold_fires_event() {
        let (pipe, cap) = pipeline_with_capture();
        let fired = fire_high_score_if_above(&pipe, "lst-2", "T", "C", 0.85, 0.85).await;
        assert!(fired);
        assert_eq!(cap.captured.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn score_below_threshold_does_not_fire() {
        let (pipe, cap) = pipeline_with_capture();
        let fired = fire_high_score_if_above(&pipe, "lst-3", "T", "C", 0.84, 0.85).await;
        assert!(!fired);
        assert!(cap.captured.lock().expect("lock").is_empty());
    }
}
