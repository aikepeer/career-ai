//! Rank many listings against a profile. Produces a scored, sorted result
//! and separates the above-threshold shortlist from the below-threshold
//! rejects so both can be persisted with the right state.

use careerai_sources::RawListing;

use crate::score::Scorer;

#[derive(Debug, Clone)]
pub struct Scored<'a> {
    pub listing: &'a RawListing,
    pub score: f32,
}

/// Score every listing, return them sorted highest-first.
pub fn rank_all<'a, S: Scorer>(
    scorer: &S,
    profile_text: &str,
    listings: &'a [RawListing],
) -> Vec<Scored<'a>> {
    let mut out: Vec<Scored<'a>> = listings
        .iter()
        .map(|listing| Scored {
            listing,
            score: scorer.score(profile_text, listing),
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Returns `(keep, drop)` split at `threshold` on a pre-sorted vec.
/// Input should come from [`rank_all`].
#[must_use]
pub fn split_at_threshold(
    ranked: Vec<Scored<'_>>,
    threshold: f32,
) -> (Vec<Scored<'_>>, Vec<Scored<'_>>) {
    ranked.into_iter().partition(|s| s.score >= threshold)
}

/// Bucket scores into 10 `[0.0, 1.0)` buckets for `match --tune` output.
/// Returns `[(bucket_lower_bound, count); 10]`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[must_use]
pub fn score_histogram(ranked: &[Scored<'_>]) -> [(f32, usize); 10] {
    let mut buckets = [0usize; 10];
    for s in ranked {
        // Scores are clamped in [0.0, 1.0] by Scorer contract; multiplying by
        // 10 and truncating is a safe bucket index in 0..=9.
        let mut idx = (s.score * 10.0) as usize;
        if idx >= 10 {
            idx = 9;
        }
        buckets[idx] += 1;
    }
    let mut out = [(0.0_f32, 0_usize); 10];
    #[allow(clippy::cast_precision_loss)]
    for (i, count) in buckets.iter().enumerate() {
        out[i] = (i as f32 / 10.0, *count);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::score::JaccardScorer;

    fn l(id: &str, title: &str, desc: &str) -> RawListing {
        RawListing {
            source: "t".into(),
            external_id: id.into(),
            title: title.into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: format!("https://x/{id}"),
            description: desc.into(),
            raw_json: None,
        }
    }

    #[test]
    fn rank_orders_highest_first() {
        let profile = "rust embedded robotics firmware ros2";
        let listings = vec![
            l("a", "Frontend Engineer", "React Typescript CSS"),
            l("b", "Embedded Engineer", "Rust firmware for robots"),
            l("c", "Data Engineer", "ETL pipelines in Python"),
        ];
        let ranked = rank_all(&JaccardScorer, profile, &listings);
        assert_eq!(ranked[0].listing.external_id, "b");
        assert!(ranked[0].score > ranked[1].score);
    }

    #[test]
    fn split_at_threshold_partitions() {
        let profile = "rust embedded robotics firmware ros2";
        let listings = vec![
            l("hit", "Embedded Robotics Engineer", "Rust firmware robots"),
            l("miss", "Customer Success Manager", "SaaS sales"),
        ];
        let ranked = rank_all(&JaccardScorer, profile, &listings);
        let (keep, drop) = split_at_threshold(ranked, 0.1);
        assert_eq!(keep.len(), 1);
        assert_eq!(drop.len(), 1);
        assert_eq!(keep[0].listing.external_id, "hit");
    }

    #[test]
    fn histogram_buckets_sum_to_input_size() {
        let profile = "a b c";
        let listings = (0..15)
            .map(|i| l(&i.to_string(), "x", "y"))
            .collect::<Vec<_>>();
        let ranked = rank_all(&JaccardScorer, profile, &listings);
        let hist = score_histogram(&ranked);
        let sum: usize = hist.iter().map(|(_, c)| *c).sum();
        assert_eq!(sum, 15);
    }
}
