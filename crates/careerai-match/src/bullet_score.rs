//! Bullet-level scoring against job descriptions (Phase 0 of LLM reduction).
//!
//! Evaluates the semantic overlap between an individual resume bullet and
//! a job description or target technical keywords.

use std::collections::HashSet;

use crate::score::{jaccard, tokenize};

/// Strategy trait for scoring a single resume bullet against a job description.
pub trait BulletScorer: Send + Sync {
    /// Score a single bullet against a job description text. Score is in `[0.0, 1.0]`.
    fn score(&self, bullet: &str, jd: &str) -> f32;

    /// Batch score multiple bullets against a job description text.
    fn score_many(&self, bullets: &[String], jd: &str) -> Vec<f32> {
        let jd_tokens = tokenize(jd);
        bullets
            .iter()
            .map(|b| self.score_with_jd_tokens(b, &jd_tokens))
            .collect()
    }

    /// Score a bullet against pre-computed JD tokens.
    fn score_with_jd_tokens(&self, bullet: &str, jd_tokens: &HashSet<String>) -> f32;
}

/// Fast, deterministic Jaccard scorer over technical and alpha-numeric tokens.
#[derive(Debug, Default, Clone, Copy)]
pub struct JaccardBulletScorer;

impl BulletScorer for JaccardBulletScorer {
    fn score(&self, bullet: &str, jd: &str) -> f32 {
        let jd_tokens = tokenize(jd);
        self.score_with_jd_tokens(bullet, &jd_tokens)
    }

    fn score_with_jd_tokens(&self, bullet: &str, jd_tokens: &HashSet<String>) -> f32 {
        let bullet_tokens = tokenize(bullet);
        jaccard(&bullet_tokens, jd_tokens)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn scores_high_on_exact_technical_match() {
        let scorer = JaccardBulletScorer;
        let jd = "Senior Embedded Linux Engineer building Rust firmware with Yocto and CAN bus.";
        let bullet = "Designed Rust firmware and BSP components using Yocto on Linux.";
        let score = scorer.score(bullet, jd);
        assert!(score > 0.25, "expected high overlap score, got {score}");
    }

    #[test]
    fn scores_zero_on_unrelated_content() {
        let scorer = JaccardBulletScorer;
        let jd = "Embedded Linux Engineer building firmware with C and RTOS.";
        let bullet = "Managed social media marketing campaigns and created graphic assets.";
        let score = scorer.score(bullet, jd);
        assert!(score < f32::EPSILON, "expected zero, got {score}");
    }

    #[test]
    fn batch_scoring_ranks_relevant_bullets_higher() {
        let scorer = JaccardBulletScorer;
        let jd = "Autonomous Mobile Robotics: ROS2, SLAM, Python, and C++ navigation.";
        let bullets = vec![
            "Implemented SLAM navigation and motion planning using ROS2 and C++.".to_string(),
            "Organized weekly team standup meetings and sprint retrospectives.".to_string(),
            "Built Python telemetry tools to monitor sensor state in real-time.".to_string(),
        ];

        let ranked = scorer.score_many(&bullets, jd);
        assert_eq!(ranked.len(), 3);
        assert!(
            ranked[0] > ranked[1],
            "bullet 0 should outscore generic bullet 1"
        );
        assert!(
            ranked[2] > ranked[1],
            "bullet 2 should outscore generic bullet 1"
        );
    }
}
