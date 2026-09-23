//! Deterministic clustering for shortlisted job descriptions.

use careerai_db::models::Listing;
use std::cmp::Ordering;
use std::collections::HashSet;

use crate::bullet_score::{BulletScorer, JaccardBulletScorer};

#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    /// Index of the highest-scoring listing in `listings`.
    pub representative_index: usize,
    /// Input indices assigned to this cluster, representative first.
    pub member_indices: Vec<usize>,
    /// Lowest representative-to-member similarity in this cluster.
    pub cohesion: f32,
}

/// Group listings whose title/company/JD token sets meet `threshold`.
///
/// The implementation intentionally uses Jaccard rather than an embedding
/// model: it is deterministic, has no model download, and lets operators
/// validate reuse before enabling an embedding-backed strategy.
#[must_use]
pub fn cluster_jds(listings: &[Listing], threshold: f32) -> Vec<Cluster> {
    if listings.is_empty() {
        return Vec::new();
    }
    let threshold = threshold.clamp(0.0, 1.0);
    let scorer = JaccardBulletScorer;
    let texts: Vec<String> = listings.iter().map(jd_text).collect();
    let mut remaining: HashSet<usize> = (0..listings.len()).collect();
    let mut clusters = Vec::new();

    while let Some(&seed) = remaining.iter().min() {
        let representative = remaining
            .iter()
            .copied()
            .max_by(|a, b| {
                listings[*a]
                    .score
                    .unwrap_or_default()
                    .partial_cmp(&listings[*b].score.unwrap_or_default())
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| b.cmp(a))
            })
            .unwrap_or(seed);
        let mut member_indices = vec![representative];
        let mut cohesion: f32 = 1.0;
        let candidates: Vec<usize> = remaining.iter().copied().collect();
        for index in candidates {
            if index == representative {
                continue;
            }
            let similarity = scorer.score(&texts[representative], &texts[index]);
            if similarity >= threshold {
                member_indices.push(index);
                cohesion = cohesion.min(similarity);
            }
        }
        member_indices.sort_unstable();
        // Keep the representative first to make the reuse contract explicit.
        let representative_position = member_indices
            .iter()
            .position(|i| *i == representative)
            .unwrap_or(0);
        member_indices.swap(0, representative_position);
        for index in &member_indices {
            remaining.remove(index);
        }
        clusters.push(Cluster {
            representative_index: representative,
            member_indices,
            cohesion,
        });
    }
    clusters
}

fn jd_text(listing: &Listing) -> String {
    format!(
        "{} {} {}",
        listing.title, listing.company, listing.description
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn listing(id: &str, title: &str, description: &str, score: f64) -> Listing {
        Listing {
            id: id.into(),
            source: "greenhouse".into(),
            external_id: id.into(),
            title: title.into(),
            company: "Acme".into(),
            location: None,
            url: format!("https://example.test/{id}"),
            description: description.into(),
            raw_json: None,
            state: "shortlisted".into(),
            score: Some(score),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn groups_near_duplicate_jds_and_keeps_best_score_representative() {
        let listings = vec![
            listing(
                "a",
                "Rust backend engineer",
                "Rust Tokio distributed services",
                0.7,
            ),
            listing(
                "b",
                "Rust backend engineer",
                "Rust Tokio distributed services",
                0.9,
            ),
            listing(
                "c",
                "Embedded firmware engineer",
                "C++ Linux BSP drivers",
                0.8,
            ),
        ];
        let clusters = cluster_jds(&listings, 0.85);
        assert_eq!(clusters.len(), 2);
        let backend = clusters
            .iter()
            .find(|cluster| cluster.member_indices.contains(&0))
            .unwrap();
        assert_eq!(backend.member_indices, vec![1, 0]);
        assert_eq!(backend.representative_index, 1);
    }

    #[test]
    fn empty_input_has_no_clusters() {
        assert!(cluster_jds(&[], 0.85).is_empty());
    }
}
