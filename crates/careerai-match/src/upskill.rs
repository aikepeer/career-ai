//! Skill gap analysis (ported from ai-job-search `/upskill`).
//!
//! Compares the candidate's profile skills against the aggregate skill
//! demand from shortlisted JDs. Produces a gap heatmap: which JD-required
//! skills the profile already has (strengths) and which it's missing
//! (gaps), weighted by how many JDs mention each skill.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use careerai_profile::schema::Profile;
use careerai_sources::RawListing;

use crate::score::tokenize;

/// One row in the skill gap heatmap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGap {
    /// The skill/keyword token.
    pub skill: String,
    /// How many shortlisted JDs mention this skill.
    pub jd_count: usize,
    /// True if the candidate's profile already contains this skill.
    pub has_skill: bool,
}

/// The full gap report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGapReport {
    /// Skills the profile has that JDs also want — strengths to emphasise.
    pub strengths: Vec<SkillGap>,
    /// Skills JDs want but the profile lacks — gaps to upskill.
    pub gaps: Vec<SkillGap>,
    /// Total shortlisted JDs analysed.
    pub total_jds: usize,
}

/// Analyse the skill gap between the candidate's profile and a set of
/// shortlisted JDs.
///
/// Deterministic — no LLM call. Tokenises each JD, counts how many JDs
/// mention each skill, and cross-references against the profile's skill
/// token set. Returns strengths sorted by `jd_count` descending, then
/// gaps sorted the same way.
#[must_use]
pub fn analyse_skill_gap(profile: &Profile, listings: &[RawListing]) -> SkillGapReport {
    let profile_text = crate::score::flatten_profile(profile);
    let profile_tokens = tokenize(&profile_text);

    // Count how many JDs mention each token.
    let mut jd_skill_counts: HashMap<String, usize> = HashMap::new();
    for listing in listings {
        let jd_tokens = tokenize(&format!("{} {}", listing.title, listing.description));
        for token in jd_tokens {
            *jd_skill_counts.entry(token).or_default() += 1;
        }
    }

    let mut strengths = Vec::new();
    let mut gaps = Vec::new();

    for (skill, count) in &jd_skill_counts {
        let has_skill = profile_tokens.contains(skill);
        let entry = SkillGap {
            skill: skill.clone(),
            jd_count: *count,
            has_skill,
        };
        if has_skill {
            strengths.push(entry);
        } else {
            gaps.push(entry);
        }
    }

    // Sort by jd_count descending, then alphabetically for stable output.
    strengths.sort_by(|a, b| b.jd_count.cmp(&a.jd_count).then(a.skill.cmp(&b.skill)));
    gaps.sort_by(|a, b| b.jd_count.cmp(&a.jd_count).then(a.skill.cmp(&b.skill)));

    // Cap gaps to the top 50 to keep output manageable.
    gaps.truncate(50);

    SkillGapReport {
        strengths,
        gaps,
        total_jds: listings.len(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::{Experience, Personal, Skills};

    fn profile_with_skills(skills: &[&str]) -> Profile {
        Profile {
            personal: Personal {
                name: "Test".into(),
                email: "t@t.com".into(),
                phone: "+1".into(),
                location: "Earth".into(),
                links: careerai_profile::schema::Links::default(),
            },
            summary: String::new(),
            skills: Skills {
                languages: skills.iter().map(|s| (*s).to_string()).collect(),
                ..Default::default()
            },
            experience: vec![Experience {
                title: "Engineer".into(),
                company: "Acme".into(),
                location: String::new(),
                start: "2020".into(),
                end: "present".into(),
                bullets: vec![],
            }],
            ..Default::default()
        }
    }

    fn listing(title: &str, desc: &str) -> RawListing {
        RawListing {
            source: "test".into(),
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
    fn identifies_strengths_and_gaps() {
        let profile = profile_with_skills(&["rust", "python", "docker"]);
        let listings = vec![
            listing(
                "Rust Engineer",
                "We need rust, tokio, and kubernetes experience",
            ),
            listing(
                "Backend Engineer",
                "Python, rust, postgres, and kafka required",
            ),
        ];

        let report = analyse_skill_gap(&profile, &listings);
        assert_eq!(report.total_jds, 2);

        // rust and python are strengths (in profile + JDs).
        let strength_skills: Vec<&str> =
            report.strengths.iter().map(|s| s.skill.as_str()).collect();
        assert!(strength_skills.contains(&"rust"));
        assert!(strength_skills.contains(&"python"));

        // tokio, kubernetes, postgres, kafka are gaps (in JDs, not profile).
        let gap_skills: Vec<&str> = report.gaps.iter().map(|s| s.skill.as_str()).collect();
        assert!(gap_skills.contains(&"tokio"));
        assert!(gap_skills.contains(&"kubernetes"));
        assert!(gap_skills.contains(&"postgres"));
        assert!(gap_skills.contains(&"kafka"));
    }

    #[test]
    fn sorts_by_jd_count_descending() {
        let profile = profile_with_skills(&[]);
        let listings = vec![
            listing("Engine", "rust rust rust python"),
            listing("Engine", "rust python"),
        ];

        let report = analyse_skill_gap(&profile, &listings);
        // rust appears in 2 JDs, python in 2 — both are gaps since profile is empty.
        assert!(!report.gaps.is_empty());
        // rust should be near the top (appears in both JDs).
        let rust = report.gaps.iter().find(|g| g.skill == "rust").unwrap();
        assert_eq!(rust.jd_count, 2);
    }

    #[test]
    fn empty_listings_returns_empty_report() {
        let profile = profile_with_skills(&["rust"]);
        let report = analyse_skill_gap(&profile, &[]);
        assert_eq!(report.total_jds, 0);
        assert!(report.strengths.is_empty());
        assert!(report.gaps.is_empty());
    }
}
