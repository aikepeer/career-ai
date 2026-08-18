//! Profile ↔ listing scoring.
//!
//! v1 uses a weighted Jaccard over lowercased alpha-tokens. Honest baseline
//! that needs no ML tooling; embedding-based scoring lands as a new
//! [`Scorer`] impl when the keyword approach visibly caps out.

use std::collections::HashSet;

use careerai_profile::Profile;
use careerai_sources::RawListing;

/// Strategy hook so tests (and future embedding work) can swap scorers
/// without touching the rank/pipeline layer.
pub trait Scorer: Send + Sync {
    fn score(&self, profile_text: &str, listing: &RawListing) -> f32;

    /// Score a whole batch. Implementations may pre-tokenize the profile
    /// once (the profile is typically much larger than any single listing);
    /// the default delegates to [`Scorer::score`] per listing.
    fn score_many(&self, profile_text: &str, listings: &[RawListing]) -> Vec<f32> {
        listings
            .iter()
            .map(|listing| self.score(profile_text, listing))
            .collect()
    }
}

/// Flatten a structured [`Profile`] into the one big blob we hand to the
/// scorer. Order is irrelevant — both scorers work on bags of tokens.
#[must_use]
pub fn flatten_profile(p: &Profile) -> String {
    let mut out = String::new();
    out.push_str(&p.personal.name);
    out.push('\n');
    out.push_str(&p.summary);
    out.push('\n');
    for s in p
        .skills
        .languages
        .iter()
        .chain(&p.skills.frameworks)
        .chain(&p.skills.tools)
    {
        out.push_str(s);
        out.push(' ');
    }
    out.push('\n');
    for e in &p.experience {
        out.push_str(&e.title);
        out.push(' ');
        out.push_str(&e.company);
        out.push('\n');
        for b in &e.bullets {
            out.push_str(b);
            out.push('\n');
        }
    }
    for proj in &p.projects {
        out.push_str(&proj.name);
        out.push('\n');
        for b in &proj.bullets {
            out.push_str(b);
            out.push('\n');
        }
    }
    out
}

/// Weighted Jaccard: the title gets 2x weight by virtue of being scored
/// separately and averaged with the body. Score is always in `[0.0, 1.0]`.
#[derive(Debug, Default, Clone, Copy)]
pub struct JaccardScorer;

impl JaccardScorer {
    fn score_with_tokens(profile_tokens: &HashSet<String>, listing: &RawListing) -> f32 {
        let title_score = jaccard(profile_tokens, &tokenize(&listing.title));
        let body_score = jaccard(profile_tokens, &tokenize(&listing.description));
        // Title is a strong signal (curated by the poster), weight 2x.
        (2.0 * title_score + body_score) / 3.0
    }
}

impl Scorer for JaccardScorer {
    fn score(&self, profile_text: &str, listing: &RawListing) -> f32 {
        let profile_tokens = tokenize(profile_text);
        if profile_tokens.is_empty() {
            return 0.0;
        }
        Self::score_with_tokens(&profile_tokens, listing)
    }

    fn score_many(&self, profile_text: &str, listings: &[RawListing]) -> Vec<f32> {
        let profile_tokens = tokenize(profile_text);
        if profile_tokens.is_empty() {
            return vec![0.0; listings.len()];
        }
        listings
            .iter()
            .map(|listing| Self::score_with_tokens(&profile_tokens, listing))
            .collect()
    }
}

#[allow(clippy::cast_precision_loss)]
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    // usize→f32 is safe here: set cardinalities are bounded by token counts
    // in a single document (thousands, not 2^24). Precision loss is nil.
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '+' && c != '#')
        .filter_map(|t| {
            let lower = t.trim().to_lowercase();
            if lower.len() < 2 || STOPWORDS.contains(&lower.as_str()) {
                None
            } else {
                Some(lower)
            }
        })
        .collect()
}

/// Deliberately small stopword list — we want domain tokens (rust, ros,
/// embedded, llm) intact.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "have", "in", "is",
    "it", "of", "on", "or", "that", "the", "to", "was", "were", "will", "with", "we", "you",
    "your", "this", "our", "their",
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn l(title: &str, desc: &str) -> RawListing {
        RawListing {
            source: "t".into(),
            external_id: "1".into(),
            title: title.into(),
            company: "Acme".into(),
            location: Some("Remote".into()),
            url: "https://x".into(),
            description: desc.into(),
            raw_json: None,
        }
    }

    #[test]
    fn preserves_technical_tokens_with_hash_and_plus() {
        let tokens = tokenize("I use C++ and C# and F# and Rust.");
        assert!(tokens.contains("c++"));
        assert!(tokens.contains("c#"));
        assert!(tokens.contains("f#"));
        assert!(tokens.contains("rust"));
    }

    #[test]
    fn score_is_zero_when_no_overlap() {
        let s = JaccardScorer;
        let score = s.score(
            "frontend react typescript design systems",
            &l("Robotics Engineer", "ROS2, C++, SLAM, perception"),
        );
        assert!(score < 0.05, "got {score}");
    }

    #[test]
    fn score_is_high_when_title_and_body_overlap() {
        let s = JaccardScorer;
        let profile = "rust engineer embedded robotics firmware ros2 slam";
        let jd = l(
            "Embedded Robotics Engineer",
            "Build firmware for autonomous robots using Rust and ROS2.",
        );
        let score = s.score(profile, &jd);
        assert!(score > 0.2, "got {score}");
    }

    #[test]
    fn title_weight_beats_body_only_match() {
        let s = JaccardScorer;
        let profile = "robotics rust embedded";
        // title_match: strong title overlap, unrelated body
        let title_match = s.score(
            profile,
            &l("Robotics Rust Embedded Engineer", "unrelated blah"),
        );
        // body_only: generic title, body words identical to profile
        let body_only = s.score(profile, &l("Engineer", "robotics rust embedded"));
        assert!(
            title_match > body_only,
            "expected title_match ({title_match}) > body_only ({body_only})",
        );
    }
}
