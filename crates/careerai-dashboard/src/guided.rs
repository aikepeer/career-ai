//! Numbered operator path: do step 1, then step 2, and so on.
//!
//! Independent of the punch-list `next_steps` engine. This always
//! returns the full sequence so the dashboard can animate progress.

use serde::Serialize;

use crate::view::{ConfigView, PipelineSnapshot};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GuidedStatus {
    Done,
    Current,
    Upcoming,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GuidedStep {
    pub number: u8,
    pub title: &'static str,
    pub hint: &'static str,
    pub command: &'static str,
    pub tab: &'static str,
    pub status: GuidedStatus,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GuidedPath {
    pub current_number: u8,
    pub headline: String,
    pub steps: Vec<GuidedStep>,
}

pub fn compute(snap: &PipelineSnapshot, config: &ConfigView) -> GuidedPath {
    let counts = &snap.state_counts;
    // Each stage counts as reached when it has rows or any later stage
    // does — `today_discovered` is a daily counter, so downstream
    // progress must imply it, or the path would regress after midnight.
    let applied = snap.kpi.applied_lifetime > 0;
    let rendered = counts.rendered > 0 || applied;
    let tailored = counts.tailored > 0 || counts.drafted > 0 || rendered;
    let shortlisted = counts.shortlisted > 0 || tailored;
    let discovered = snap.kpi.today_discovered > 0 || shortlisted;
    let has_profile = config.profile.is_some();
    let has_keywords = config.keywords.iter().any(|k| k.enabled);

    let done = [
        has_profile,
        has_keywords,
        discovered,
        shortlisted,
        tailored,
        rendered,
        applied,
    ];
    let meta = step_meta();
    let all_done = done.iter().all(|d| *d);
    let current_idx = done.iter().position(|d| !*d).unwrap_or(done.len() - 1);

    let steps = meta
        .iter()
        .enumerate()
        .map(|(i, (title, hint, command, tab))| {
            let status = if done[i] {
                GuidedStatus::Done
            } else if i == current_idx {
                GuidedStatus::Current
            } else {
                GuidedStatus::Upcoming
            };
            GuidedStep {
                number: u8::try_from(i + 1).unwrap_or(u8::MAX),
                title,
                hint,
                command,
                tab,
                status,
            }
        })
        .collect();

    let headline = if all_done {
        "All caught up — the pipeline has run end to end. New discoveries resume at Step 3."
            .to_string()
    } else {
        format!("Step {} of 7 — {}", current_idx + 1, meta[current_idx].0)
    };

    GuidedPath {
        current_number: u8::try_from(current_idx + 1).unwrap_or(u8::MAX),
        headline,
        steps,
    }
}

fn step_meta() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        (
            "Import your profile",
            "Upload a resume PDF/DOCX or LinkedIn export on the Config tab.",
            "careerai profile import <files>",
            "config",
        ),
        (
            "Configure keywords & sources",
            "Generate discovery keywords from the profile, then review sources.",
            "careerai config generate",
            "config",
        ),
        (
            "Discover jobs",
            "Pull fresh listings from every configured source.",
            "careerai discover",
            "commands",
        ),
        (
            "Match & shortlist",
            "Filter and rank discoveries against the profile.",
            "careerai match",
            "commands",
        ),
        (
            "Tailor resumes",
            "LLM-tailor the master resume to each shortlisted listing.",
            "careerai tailor <listing-id>",
            "explorer",
        ),
        (
            "Render applications",
            "Render tailored resumes to DOCX + PDF via pandoc.",
            "careerai render <application-id>",
            "explorer",
        ),
        (
            "Apply & track",
            "Submit prepared applications — dry-run unless auto_submit is on.",
            "careerai apply --all",
            "actions",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{KpiStrip, ProfileView, StateCounts};

    fn empty_snap() -> PipelineSnapshot {
        PipelineSnapshot {
            kpi: KpiStrip {
                today_discovered: 0,
                shortlisted_active: 0,
                applied_lifetime: 0,
                response_rate_pct: None,
                response_rate_label: "—".into(),
            },
            columns: vec![],
            state_counts: StateCounts::default(),
            source_lag_hours: vec![],
            linkedin_cookie_days_left: None,
            profile_age_days: None,
        }
    }

    fn empty_config() -> ConfigView {
        ConfigView {
            score_threshold: 0.7,
            must_include_skills: vec![],
            keywords: vec![],
            sources: vec![],
            llm_provider: "auto".into(),
            llm_model: String::new(),
            llm_status: "unknown".into(),
            rate_limit_per_min: 60,
            prompt_version: "v1".into(),
            profile: None,
            llm_backend: "auto".into(),
            llm_strategy: "local".into(),
            llm_api_base: None,
            llm_timeout_seconds: 300,
        }
    }

    fn named_profile() -> ProfileView {
        ProfileView {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            phone: String::new(),
            location: String::new(),
            github: String::new(),
            linkedin: String::new(),
            portfolio: String::new(),
            summary: String::new(),
            target_roles: vec![],
            languages: vec![],
            frameworks: vec![],
            tools: vec![],
            platforms: vec![],
            devops: vec![],
            debugging: vec![],
            protocols: vec![],
            skill_count: 0,
            experience_count: 0,
            education_count: 0,
            career_story: vec![],
            raw_yaml: String::new(),
        }
    }

    #[test]
    fn empty_workspace_starts_at_step_1_import_profile() {
        let path = compute(&empty_snap(), &empty_config());
        assert_eq!(path.steps.len(), 7);
        assert_eq!(path.current_number, 1);
        assert_eq!(path.steps[0].title, "Import your profile");
        assert_eq!(path.steps[0].status, GuidedStatus::Current);
        assert!(path.headline.contains("Step 1"));
        for step in &path.steps[1..] {
            assert_eq!(step.status, GuidedStatus::Upcoming);
        }
    }

    #[test]
    fn profile_without_keywords_advances_to_configure() {
        let mut cfg = empty_config();
        cfg.profile = Some(named_profile());
        let path = compute(&empty_snap(), &cfg);
        assert_eq!(path.current_number, 2);
        assert_eq!(path.steps[0].status, GuidedStatus::Done);
        assert_eq!(path.steps[1].status, GuidedStatus::Current);
        assert!(path.headline.contains("Step 2"));
    }

    #[test]
    fn shortlisted_listings_point_at_tailor() {
        let mut cfg = empty_config();
        cfg.profile = Some(named_profile());
        cfg.keywords = vec![crate::view::KeywordStatus {
            name: "Rust".into(),
            enabled: true,
        }];
        let mut snap = empty_snap();
        snap.kpi.today_discovered = 4;
        snap.state_counts.shortlisted = 2;
        let path = compute(&snap, &cfg);
        assert_eq!(path.current_number, 5);
        assert_eq!(path.steps[4].title, "Tailor resumes");
        assert_eq!(path.steps[4].status, GuidedStatus::Current);
        assert!(path.headline.contains("Step 5"));
    }

    #[test]
    fn applied_lifetime_marks_the_path_complete() {
        let mut cfg = empty_config();
        cfg.profile = Some(named_profile());
        cfg.keywords = vec![crate::view::KeywordStatus {
            name: "Rust".into(),
            enabled: true,
        }];
        let mut snap = empty_snap();
        snap.kpi.today_discovered = 4;
        snap.kpi.applied_lifetime = 1;
        snap.state_counts.shortlisted = 1;
        snap.state_counts.tailored = 1;
        snap.state_counts.rendered = 1;
        let path = compute(&snap, &cfg);
        assert_eq!(path.current_number, 7);
        assert!(path.steps.iter().all(|s| s.status == GuidedStatus::Done));
        assert!(path.headline.to_lowercase().contains("caught up"));
    }
}
