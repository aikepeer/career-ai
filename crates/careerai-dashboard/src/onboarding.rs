//! F01: Guided first-session setup.
//!
//! Computes a structured onboarding state from the current pipeline snapshot
//! and config. The frontend uses this to walk a new user through:
//!   1. Import a profile.
//!   2. Confirm role, location, and remote preference.
//!   3. Preview five matched listings.
//!   4. See the next concrete action.
//!
//! Import errors preserve the previous profile because the import handler
//! writes to a draft and only promotes on explicit confirm. This module never
//! sends submissions or messages — it only computes guidance.

use serde::{Deserialize, Serialize};

use crate::view::{ConfigView, PipelineSnapshot};

/// Which onboarding phase the user is in.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingPhase {
    /// No profile imported yet.
    ImportProfile,
    /// Profile exists but roles/locations have not been confirmed.
    ConfirmPreferences,
    /// Preferences confirmed but no listings discovered/matched yet.
    PreviewMatches,
    /// At least one listing has been shortlisted for review.
    ReviewMatch,
    /// Onboarding complete — the user has reviewed at least one match.
    Complete,
}

/// A single matched listing in the 5-match preview.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PreviewMatch {
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub score: f32,
    pub state: String,
    pub is_remote: bool,
}

/// The structured first-session state returned by the API.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OnboardingState {
    pub phase: OnboardingPhase,
    pub has_profile: bool,
    pub has_roles: bool,
    pub has_locations: bool,
    pub remote_pref: Option<bool>,
    pub preview_matches: Vec<PreviewMatch>,
    pub next_action: String,
    pub next_action_tab: &'static str,
    /// Elapsed seconds from first profile import to first reviewed match.
    /// Present only when the user opted in to local timing and the match
    /// has been reviewed.
    pub time_to_first_match_seconds: Option<u64>,
}

/// User preferences submitted during the confirm step.
#[derive(Debug, Clone, Deserialize)]
pub struct OnboardingPreferences {
    pub target_roles: Vec<String>,
    pub locations: Vec<String>,
    pub remote_only: Option<bool>,
}

/// Compute the onboarding state from the current snapshot and config.
///
/// `preview_matches` is capped at 5 entries by the caller (the handler pulls
/// from the explorer query). This function does not query the database.
#[must_use]
pub fn compute(
    snap: &PipelineSnapshot,
    config: &ConfigView,
    preview_matches: Vec<PreviewMatch>,
    time_to_first_match_seconds: Option<u64>,
) -> OnboardingState {
    let has_profile = config.profile.is_some();
    let has_roles = config
        .profile
        .as_ref()
        .is_some_and(|p| !p.target_roles.is_empty());
    // Locations live in the config keywords/sources, not the profile. We
    // treat "has locations" as true when any source has a non-empty location
    // or the profile has a location set. This is a heuristic; the confirm
    // step is where the user makes it explicit.
    let has_locations = config
        .profile
        .as_ref()
        .is_some_and(|p| !p.location.is_empty())
        || config.sources.iter().any(|s| {
            s.kind == "greenhouse"
                || s.kind == "lever"
                || s.kind == "ashby"
                || s.kind == "naukri"
        });
    let remote_pref = None; // Resolved from saved preferences, not config view.

    let has_shortlisted = snap.state_counts.shortlisted > 0;
    let has_tailored = snap.state_counts.tailored > 0 || snap.state_counts.drafted > 0;

    let (phase, next_action, next_action_tab) = if !has_profile {
        (
            OnboardingPhase::ImportProfile,
            "Import your resume PDF/DOCX or LinkedIn export on the Config tab."
                .to_string(),
            "config",
        )
    } else if !has_roles || !has_locations {
        (
            OnboardingPhase::ConfirmPreferences,
            "Confirm your target roles, locations, and remote preference.".to_string(),
            "config",
        )
    } else if !has_shortlisted {
        (
            OnboardingPhase::PreviewMatches,
            "Review the five preview matches and shortlist the ones that fit.".to_string(),
            "explorer",
        )
    } else if !has_tailored {
        (
            OnboardingPhase::ReviewMatch,
            "Open the top shortlisted match and review the tailored resume.".to_string(),
            "explorer",
        )
    } else {
        (
            OnboardingPhase::Complete,
            "Onboarding complete — continue your daily workflow from the dashboard.".to_string(),
            "actions",
        )
    };

    OnboardingState {
        phase,
        has_profile,
        has_roles,
        has_locations,
        remote_pref,
        preview_matches: preview_matches
            .into_iter()
            .take(5)
            .collect(),
        next_action,
        next_action_tab,
        time_to_first_match_seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{
        ConfigView, KpiStrip, PipelineSnapshot, ProfileView, StateCounts,
    };

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

    fn profile_with_roles() -> ProfileView {
        ProfileView {
            name: "Ada".into(),
            email: "ada@example.com".into(),
            phone: String::new(),
            location: "Remote".into(),
            github: String::new(),
            linkedin: String::new(),
            portfolio: String::new(),
            summary: String::new(),
            target_roles: vec!["AI Platform Engineer".into()],
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
    fn empty_workspace_starts_at_import_profile() {
        let state = compute(&empty_snap(), &empty_config(), vec![], None);
        assert_eq!(state.phase, OnboardingPhase::ImportProfile);
        assert!(!state.has_profile);
        assert_eq!(state.next_action_tab, "config");
        assert!(state.next_action.contains("Import"));
    }

    #[test]
    fn profile_without_roles_advances_to_confirm_preferences() {
        let mut cfg = empty_config();
        let mut profile = profile_with_roles();
        profile.target_roles = vec![];
        cfg.profile = Some(profile);
        let state = compute(&empty_snap(), &cfg, vec![], None);
        assert_eq!(state.phase, OnboardingPhase::ConfirmPreferences);
        assert!(state.has_profile);
        assert!(!state.has_roles);
        assert_eq!(state.next_action_tab, "config");
    }

    #[test]
    fn profile_with_roles_and_locations_advances_to_preview() {
        let mut cfg = empty_config();
        cfg.profile = Some(profile_with_roles());
        let state = compute(&empty_snap(), &cfg, vec![], None);
        assert_eq!(state.phase, OnboardingPhase::PreviewMatches);
        assert!(state.has_roles);
        assert!(state.has_locations);
        assert_eq!(state.next_action_tab, "explorer");
    }

    #[test]
    fn shortlisted_match_advances_to_review() {
        let mut cfg = empty_config();
        cfg.profile = Some(profile_with_roles());
        let mut snap = empty_snap();
        snap.state_counts.shortlisted = 1;
        let state = compute(&snap, &cfg, vec![], None);
        assert_eq!(state.phase, OnboardingPhase::ReviewMatch);
        assert_eq!(state.next_action_tab, "explorer");
    }

    #[test]
    fn tailored_match_completes_onboarding() {
        let mut cfg = empty_config();
        cfg.profile = Some(profile_with_roles());
        let mut snap = empty_snap();
        snap.state_counts.shortlisted = 1;
        snap.state_counts.tailored = 1;
        let state = compute(&snap, &cfg, vec![], None);
        assert_eq!(state.phase, OnboardingPhase::Complete);
        assert!(state.next_action.contains("complete"));
    }

    #[test]
    fn preview_matches_capped_at_five() {
        let matches: Vec<PreviewMatch> = (0..10)
            .map(|i| PreviewMatch {
                listing_id: format!("listing-{i}"),
                title: format!("Title {i}"),
                company: format!("Company {i}"),
                score: 0.9,
                state: "discovered".into(),
                is_remote: true,
            })
            .collect();
        let state = compute(&empty_snap(), &empty_config(), matches, None);
        assert_eq!(state.preview_matches.len(), 5);
        assert_eq!(state.preview_matches[0].listing_id, "listing-0");
        assert_eq!(state.preview_matches[4].listing_id, "listing-4");
    }

    #[test]
    fn time_to_first_match_preserved_when_opted_in() {
        let state = compute(&empty_snap(), &empty_config(), vec![], Some(42));
        assert_eq!(state.time_to_first_match_seconds, Some(42));
    }
}
