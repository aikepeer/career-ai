//! Application quality scoring.
//!
//! Produces a [`QualityScore`] that predicts how strong an application
//! package is before submission. Higher scores are better; every
//! sub-score is in `[0.0, 1.0]`.

#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use serde::Serialize;

use crate::model::ResumeView;

/// A small, common English stopword set used when tokenizing JD text.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "her", "was", "one",
    "our", "out", "has", "have", "from", "this", "that", "with", "will", "your", "they", "them",
    "then", "than", "into", "over", "also", "what", "when", "who", "how", "why", "which", "their",
    "there", "where", "would", "could", "should", "been", "were", "some", "such", "very", "more",
    "most", "other", "its", "his", "she", "him", "had", "did", "does", "done",
];

/// Break text into lowercase keyword tokens: split on non-alphanumeric,
/// drop stopwords and tokens shorter than 3 chars.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3 && !STOPWORDS.contains(t))
        .map(String::from)
        .collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QualityScore {
    pub overall: f32,
    pub jd_relevance: f32,
    pub skill_coverage: f32,
    pub cover_letter_depth: f32,
    pub bullet_density: f32,
    pub recommendations: Vec<String>,
}

/// Flatten every experience bullet across all entries into one list.
fn all_bullets(resume_view: &ResumeView) -> Vec<&String> {
    resume_view
        .experience
        .iter()
        .flat_map(|exp| exp.bullets.iter())
        .collect()
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// Compute the JD-relevance sub-score: fraction of bullets that contain
/// at least one JD keyword.
fn score_jd_relevance(bullets: &[&String], jd_keywords: &[String]) -> f32 {
    if bullets.is_empty() {
        return 0.0;
    }
    let jd_set: std::collections::HashSet<&str> = jd_keywords.iter().map(String::as_str).collect();
    let hits = bullets
        .iter()
        .filter(|b| {
            let lower = b.to_lowercase();
            jd_set.iter().any(|kw| lower.contains(kw))
        })
        .count();
    clamp01(hits as f32 / bullets.len() as f32)
}

/// Compute skill coverage: fraction of resume skills present in the JD.
/// Returns 1.0 when the JD mentions no resume skills (vacuously satisfied).
fn score_skill_coverage(resume_view: &ResumeView, jd_lower: &str) -> f32 {
    let skills: Vec<String> = resume_view
        .skills
        .all_skill_names()
        .map(|s| s.to_lowercase())
        .collect();
    if skills.is_empty() {
        return 1.0;
    }
    let present = skills
        .iter()
        .filter(|s| jd_lower.contains(s.as_str()))
        .count();
    clamp01(present as f32 / skills.len() as f32)
}

/// Compute cover-letter depth based on length and personalization markers.
fn score_cover_letter_depth(body: &str) -> f32 {
    let len = body.trim().len();
    if len < 100 {
        return 0.0;
    }
    let mut score = if len < 200 { 0.3 } else { 0.5 };

    // Company name present (+0.2) — heuristic: capitalized word > 3 chars.
    if body
        .split_whitespace()
        .any(|w| w.len() > 3 && w.chars().next().is_some_and(char::is_uppercase))
    {
        score += 0.2;
    }

    // Specific project / role mentioned (+0.2) — heuristic: mentions a role
    // keyword or the word "project".
    let lower = body.to_lowercase();
    let role_terms = [
        "engineer",
        "developer",
        "scientist",
        "architect",
        "lead",
        "manager",
        "project",
        "role",
        "position",
    ];
    if role_terms.iter().any(|t| lower.contains(t)) {
        score += 0.2;
    }

    // Length > 300 chars (+0.15).
    if len > 300 {
        score += 0.15;
    }

    // No generic-only content (+0.15) — penalize when the body is *just* a
    // boilerplate opener with nothing substantive after it.
    let generic = "i am writing to apply for";
    if !lower.starts_with(generic) || lower.len() > generic.len() + 50 {
        score += 0.15;
    }

    clamp01(score)
}

/// Compute bullet density: fraction of bullets containing metrics
/// (numbers, percentages, or dollar signs).
fn score_bullet_density(bullets: &[&String]) -> f32 {
    if bullets.is_empty() {
        return 0.0;
    }
    let metric_bullets = bullets
        .iter()
        .filter(|b| {
            b.chars()
                .any(|c| c.is_ascii_digit() || c == '%' || c == '$')
        })
        .count();
    clamp01(metric_bullets as f32 / bullets.len() as f32)
}

/// Score an application package against the JD and cover letter.
#[allow(clippy::too_many_lines)]
pub fn score_application_quality(
    resume_view: &ResumeView,
    jd_text: &str,
    cover_letter_body: &str,
) -> QualityScore {
    let jd_keywords = tokenize(jd_text);
    let jd_lower = jd_text.to_lowercase();
    let bullets = all_bullets(resume_view);

    let jd_relevance = score_jd_relevance(&bullets, &jd_keywords);
    let skill_coverage = score_skill_coverage(resume_view, &jd_lower);
    let cover_letter_depth = score_cover_letter_depth(cover_letter_body);
    let bullet_density = score_bullet_density(&bullets);

    let overall = jd_relevance * 0.35
        + skill_coverage * 0.25
        + cover_letter_depth * 0.2
        + bullet_density * 0.2;

    let mut recommendations = Vec::new();
    if jd_relevance < 0.5 {
        recommendations.push("Resume bullets don't mention enough JD keywords".to_string());
    }
    if skill_coverage < 0.5 {
        recommendations.push("Many JD-required skills are missing from the resume".to_string());
    }
    if cover_letter_depth < 0.5 {
        recommendations.push("Cover letter needs more personalization and depth".to_string());
    }
    if bullet_density < 0.3 {
        recommendations.push("Add more quantifiable metrics to resume bullets".to_string());
    }

    QualityScore {
        overall: clamp01(overall),
        jd_relevance,
        skill_coverage,
        cover_letter_depth,
        bullet_density,
        recommendations,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::model::{ExperienceView, ResumeView};
    use careerai_profile::schema::{Personal, Skills};

    fn make_view(bullets: Vec<String>, skills: Skills) -> ResumeView {
        ResumeView {
            personal: Personal::default(),
            summary: String::new(),
            skills,
            experience: vec![ExperienceView {
                title: "Engineer".into(),
                company: "Corp".into(),
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
    fn test_high_quality_application() {
        let skills = Skills {
            languages: vec!["Rust".into(), "Python".into()],
            ..Default::default()
        };
        let bullets = vec![
            "Built Rust microservices serving 1M requests per second".into(),
            "Reduced latency by 40% using Tokio async runtime".into(),
            "Deployed Python ML pipeline processing 500k events".into(),
        ];
        let view = make_view(bullets, skills);
        let jd = "We need a Rust engineer with Python experience. \
                  Must build high-throughput microservices and async runtime systems.";
        let cover = "Dear Acme Corp, I am excited about the Senior Engineer position. \
                     At my previous role I led the Rust platform team, delivering a \
                     distributed system that processed 1M events. I would love to bring \
                     my expertise in Rust and Python to your engineering team.";

        let score = score_application_quality(&view, jd, cover);
        assert!(score.overall > 0.7, "overall={}", score.overall);
    }

    #[test]
    fn test_low_jd_relevance() {
        let skills = Skills::default();
        let bullets = vec![
            "Managed a community garden and organized local events".into(),
            "Taught piano lessons to young children on weekends".into(),
            "Painted murals for neighborhood beautification project".into(),
        ];
        let view = make_view(bullets, skills);
        let jd = "We need a Kubernetes engineer with deep distributed systems experience \
                  and infrastructure automation using Terraform and Helm.";

        let score = score_application_quality(&view, jd, "x".repeat(200).as_str());
        assert!(
            score.jd_relevance < 0.3,
            "jd_relevance={}",
            score.jd_relevance
        );
    }

    #[test]
    fn test_short_cover_letter() {
        let skills = Skills::default();
        let view = make_view(vec!["Built systems".into()], skills);
        let jd = "Software engineer role";

        let score = score_application_quality(&view, jd, "Hi, I want this job.");
        assert_eq!(score.cover_letter_depth, 0.0);
    }

    #[test]
    fn test_empty_bullets() {
        let skills = Skills::default();
        let view = make_view(vec![], skills);
        let jd = "Software engineer role";

        let score = score_application_quality(&view, jd, "x".repeat(200).as_str());
        assert_eq!(score.bullet_density, 0.0);
    }

    #[test]
    fn test_recommendations_generated() {
        let skills = Skills::default();
        let bullets = vec!["Did some work at the office".into()];
        let view = make_view(bullets, skills);
        let jd = "We need a chef with culinary expertise.";
        let cover = "I want this job.";

        let score = score_application_quality(&view, jd, cover);
        assert!(
            !score.recommendations.is_empty(),
            "should have recommendations for weak application"
        );
    }
}
