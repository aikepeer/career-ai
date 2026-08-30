//! Deterministic relevance-weighted bullet reduction (ported from
//! ai-job-search's "Relevance-weighted CV cutting").
//!
//! When a resume overflows its bullet budget, we do not cut mechanically
//! from the "oldest" section. Each bullet is scored on three axes and the
//! lowest-total bullet is cut first, regardless of section:
//!
//! (a) **relevance** — Jaccard overlap with THIS posting's text; an
//!     older-role bullet that hits posting keywords survives ahead of a
//!     recent-role bullet that does not.
//! (b) **uniqueness** — is the claim duplicated elsewhere in the
//!     document? A claim repeated in three bullets is cheaper to cut.
//! (c) **dependency** — does the cover letter reference it? Cutting a
//!     bullet the letter leans on would orphan a claim in the final
//!     application.
//!
//! Safety invariant: this module ONLY removes existing bullets — it never
//! invents or rewrites content, so it is trivially compatible with the
//! constrained-diff grammar.

use careerai_match::bullet_score::{BulletScorer, JaccardBulletScorer};

use crate::model::ResumeView;

/// Bullet budgets per entry type.
#[derive(Debug, Clone, Copy)]
pub struct ReduceConfig {
    /// Maximum bullets kept per experience entry.
    pub max_experience_bullets: usize,
    /// Maximum bullets kept per project entry.
    pub max_project_bullets: usize,
}

impl Default for ReduceConfig {
    fn default() -> Self {
        Self {
            max_experience_bullets: 8,
            max_project_bullets: 6,
        }
    }
}

/// One bullet's reduction scores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BulletScore {
    /// Jaccard overlap with the JD text, `[0, 1]`.
    pub relevance: f32,
    /// `[0, 1]` — 1.0 when no other bullet shares a significant token.
    pub uniqueness: f32,
    /// `[0, 1]` — 1.0 when the cover letter references a significant
    /// token of this bullet.
    pub dependency: f32,
    /// Weighted sum — the cut order key.
    pub total: f32,
}

/// Weighted total — the cut order key. Relevance is the primary axis:
/// the tiebreakers can shift the total by at most 0.25, so a real
/// relevance difference (Jaccard deltas between keyword-hitting and
/// non-hitting bullets are typically >= 0.25) always wins. Uniqueness
/// and dependency only decide near-ties.
pub fn total_score(relevance: f32, uniqueness: f32, dependency: f32) -> f32 {
    relevance + 0.125 * uniqueness + 0.125 * dependency
}

/// Uniqueness of `all_bullets[idx]` within the whole document: the
/// fraction of the bullet's significant tokens (length >= 5) that appear
/// in no other bullet. Index-based, so two bullets with identical text
/// are still "others" for each other. A bullet with no significant
/// tokens scores 0.5 (neutral).
pub fn uniqueness_score(all_bullets: &[String], idx: usize) -> f32 {
    let bullet = &all_bullets[idx];
    let toks = significant_tokens(bullet);
    if toks.is_empty() {
        return 0.5;
    }
    let others: Vec<String> = all_bullets
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != idx)
        .flat_map(|(_, b)| significant_tokens(b))
        .collect();
    let unique = toks.iter().filter(|t| !others.contains(t)).count();
    #[allow(clippy::cast_precision_loss)] // bullet token counts are tiny
    {
        unique as f32 / toks.len() as f32
    }
}

/// Cover-letter dependency of a bullet: 1.0 when at least one significant
/// token appears in the letter (the letter leans on the claim), else 0.0.
pub fn dependency_score(bullet: &str, cover_letter: &str) -> f32 {
    let letter_tokens = significant_tokens(cover_letter);
    if letter_tokens.is_empty() {
        return 0.0;
    }
    if significant_tokens(bullet)
        .iter()
        .any(|t| letter_tokens.contains(t))
    {
        1.0
    } else {
        0.0
    }
}

/// What a reduction pass did.
#[derive(Debug, Default)]
pub struct ReduceReport {
    /// (entry label, cut bullet text) pairs, cut order first-to-last.
    pub cut: Vec<(String, String)>,
}

/// Reduce `view` to the configured budgets. Only cuts bullets; entries
/// are never removed and no bullet list drops below one element.
pub fn reduce_view(
    view: ResumeView,
    jd_text: &str,
    cover_letter: &str,
    cfg: &ReduceConfig,
) -> (ResumeView, ReduceReport) {
    let scorer = JaccardBulletScorer;
    let mut report = ReduceReport::default();
    let mut view = view;

    let all_bullets: Vec<String> = view
        .experience
        .iter()
        .flat_map(|e| e.bullets.clone())
        .chain(view.projects.iter().flat_map(|p| p.bullets.clone()))
        .collect();

    let mut base = 0usize;
    for (i, entry) in view.experience.iter_mut().enumerate() {
        let budget = cfg.max_experience_bullets.max(1);
        cut_entry(
            &mut entry.bullets,
            budget,
            jd_text,
            cover_letter,
            &all_bullets,
            scorer,
            base,
            &format!("experience[{i}]"),
            &mut report,
        );
        base += entry.bullets.len();
    }
    for (i, project) in view.projects.iter_mut().enumerate() {
        let budget = cfg.max_project_bullets.max(1);
        cut_entry(
            &mut project.bullets,
            budget,
            jd_text,
            cover_letter,
            &all_bullets,
            scorer,
            base,
            &format!("projects[{i}]"),
            &mut report,
        );
        base += project.bullets.len();
    }

    (view, report)
}

#[allow(clippy::too_many_arguments)]
fn cut_entry(
    bullets: &mut Vec<String>,
    budget: usize,
    jd_text: &str,
    cover_letter: &str,
    all_bullets: &[String],
    scorer: JaccardBulletScorer,
    base: usize,
    label: &str,
    report: &mut ReduceReport,
) {
    while bullets.len() > budget && bullets.len() > 1 {
        // Score each remaining bullet. `base + idx` maps the entry-local
        // index to the document-wide bullet list (a pre-cut snapshot,
        // keeping uniqueness comparisons stable across the pass).
        let mut ranked: Vec<(usize, f32)> = (0..bullets.len())
            .map(|idx| {
                let relevance = scorer.score(&bullets[idx], jd_text);
                let uniqueness = uniqueness_score(all_bullets, base + idx);
                let dependency = dependency_score(&bullets[idx], cover_letter);
                (idx, total_score(relevance, uniqueness, dependency))
            })
            .collect();
        // Cut the lowest total; stable tie-break by index so the result
        // is deterministic.
        ranked.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let (cut_idx, _) = ranked[0];
        let removed = bullets.remove(cut_idx);
        report.cut.push((label.to_string(), removed));
    }
}

/// Lowercased alnum tokens of length >= 5 — distinctive-enough tokens
/// for uniqueness/dependency comparison.
fn significant_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 5)
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn view_with_experience(bullets: Vec<String>) -> ResumeView {
        ResumeView {
            personal: crate::model::Personal::default(),
            summary: "Summary".into(),
            skills: crate::model::Skills::default(),
            experience: vec![crate::model::ExperienceView {
                title: "Senior Engineer".into(),
                company: "Acme".into(),
                location: None,
                start: "2020".into(),
                end: "2025".into(),
                bullets,
            }],
            education: vec![],
            projects: vec![],
        }
    }

    #[test]
    fn total_score_relevance_dominates() {
        // A real relevance difference beats perfect tiebreakers.
        assert!(total_score(0.9, 0.0, 0.0) > total_score(0.6, 1.0, 1.0));
        // Near-ties are decided by the tiebreakers.
        assert!(total_score(0.5, 1.0, 1.0) > total_score(0.5, 0.0, 0.0));
        assert!(total_score(0.5, 1.0, 0.0) > total_score(0.5, 0.0, 0.0));
        assert!(total_score(0.5, 0.0, 1.0) > total_score(0.5, 0.0, 0.0));
        // Tiebreaker band is bounded: 0.25 max combined.
        assert!(total_score(0.5, 1.0, 1.0) < total_score(0.8, 0.0, 0.0));
    }

    #[test]
    fn cuts_lowest_relevance_bullet_first() {
        let jd = "Senior Rust engineer building firmware with Yocto and CAN bus.";
        let view = view_with_experience(vec![
            "Designed Rust firmware and BSP components using Yocto on Linux.".into(),
            "Organized team offsites and ordered office snacks.".into(),
            "Mentored two junior engineers on code review practice.".into(),
        ]);
        let (reduced, report) = reduce_view(
            view,
            jd,
            "",
            &ReduceConfig {
                max_experience_bullets: 2,
                ..ReduceConfig::default()
            },
        );
        assert_eq!(reduced.experience[0].bullets.len(), 2);
        assert_eq!(report.cut.len(), 1);
        assert!(report.cut[0].1.contains("snacks"));
        assert!(reduced.experience[0].bullets[0].contains("Rust"));
    }

    #[test]
    fn never_cuts_below_one_bullet() {
        let jd = "Anything";
        let view = view_with_experience(vec!["Only bullet".into()]);
        let cfg = ReduceConfig {
            max_experience_bullets: 0, // budget floor kicks in
            ..ReduceConfig::default()
        };
        let (reduced, report) = reduce_view(view, jd, "", &cfg);
        assert_eq!(reduced.experience[0].bullets.len(), 1);
        assert!(report.cut.is_empty());
    }

    #[test]
    fn respects_per_entry_type_budgets() {
        let jd = "Rust firmware engineer.";
        let view = ResumeView {
            personal: crate::model::Personal::default(),
            summary: "Summary".into(),
            skills: crate::model::Skills::default(),
            experience: vec![crate::model::ExperienceView {
                title: "T".into(),
                company: "C".into(),
                location: None,
                start: "2020".into(),
                end: "2025".into(),
                bullets: (0..5)
                    .map(|i| format!("Bullet number {i} with rust firmware words"))
                    .collect(),
            }],
            education: vec![],
            projects: vec![crate::model::ProjectView {
                name: "P".into(),
                url: None,
                bullets: (0..5)
                    .map(|i| format!("Project bullet {i} with rust firmware words"))
                    .collect(),
            }],
        };
        let cfg = ReduceConfig {
            max_experience_bullets: 3,
            max_project_bullets: 2,
        };
        let (reduced, report) = reduce_view(view, jd, "", &cfg);
        assert_eq!(reduced.experience[0].bullets.len(), 3);
        assert_eq!(reduced.projects[0].bullets.len(), 2);
        assert_eq!(report.cut.len(), 5);
    }

    #[test]
    fn no_op_when_within_budget() {
        let jd = "Rust firmware.";
        let view = view_with_experience(vec!["Rust".into(), "Firmware".into()]);
        let (reduced, report) = reduce_view(view, jd, "", &ReduceConfig::default());
        assert_eq!(reduced.experience[0].bullets.len(), 2);
        assert!(report.cut.is_empty());
    }

    #[test]
    fn uniqueness_lowers_score_of_duplicated_claims() {
        let bullets = vec![
            "Built RAG pipelines in Rust".to_string(),
            "Built RAG pipelines in Rust".to_string(),
            "Architected CAN bus firmware".to_string(),
        ];
        let dup = uniqueness_score(&bullets, 0);
        let uniq = uniqueness_score(&bullets, 2);
        assert!(uniq > dup, "duplicate claim must score lower on uniqueness");
        assert!((uniq - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn cover_letter_dependency_saves_referenced_bullet() {
        // Two bullets with equal JD relevance; the letter references one
        // of them. The referenced one must survive.
        let jd = "telemetry pipelines engineer rust mobile apps";
        let letter = "Your telemetry pipelines work at Acme stood out.";
        let view = view_with_experience(vec![
            "Built telemetry pipelines at Acme".into(),
            "Shipped mobile apps at Beta".into(),
        ]);
        let (reduced, report) = reduce_view(
            view,
            jd,
            letter,
            &ReduceConfig {
                max_experience_bullets: 1,
                ..ReduceConfig::default()
            },
        );
        assert_eq!(reduced.experience[0].bullets.len(), 1);
        assert!(reduced.experience[0].bullets[0].contains("telemetry"));
        assert!(report.cut[0].1.contains("mobile"));
    }

    #[test]
    fn older_relevant_bullet_beats_recent_irrelevant_one() {
        // ai-job-search's headline rule: an older-role bullet that hits
        // posting keywords survives ahead of a recent-role bullet that
        // does not. Entries are not ordered by recency here — the rule
        // is about relevance beating recency-based cut order.
        let jd = "Waymo autonomous vehicle fleet management";
        let view = view_with_experience(vec![
            "Managed waymo fleet telemetry for autonomous vehicles".into(),
            "Led quarterly planning ceremonies".into(),
            "Handled vendor contracts and renewals".into(),
        ]);
        let (reduced, _) = reduce_view(
            view,
            jd,
            "",
            &ReduceConfig {
                max_experience_bullets: 1,
                ..ReduceConfig::default()
            },
        );
        assert!(reduced.experience[0].bullets[0].contains("waymo"));
    }
}
