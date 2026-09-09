//! Tailored resume DTOs.
//!
//! A `ResumeView` mirrors the shape of `careerai_profile::Profile` but
//! carries already-reordered and reworded bullets. We deliberately do not
//! reuse `Profile` here because the view is a snapshot of a *tailored*
//! resume, not the canonical master profile. Static subfields that never
//! get tailored (`Personal`, `Skills`, `Education`) are re-exported
//! verbatim from `careerai-profile`.

use serde::{Deserialize, Serialize};

pub use careerai_profile::schema::{Education, Personal, Skills};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeView {
    pub personal: Personal,
    pub summary: String,
    pub skills: Skills,
    pub experience: Vec<ExperienceView>,
    pub education: Vec<Education>,
    pub projects: Vec<ProjectView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperienceView {
    pub title: String,
    pub company: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub start: String,
    pub end: String,
    pub bullets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectView {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub bullets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetter {
    pub body: String,
}

/// Bundle returned by `tailor_for_listing`.
#[derive(Debug, Clone)]
pub struct TailorOutcome {
    pub application_id: String,
    pub resume_view: ResumeView,
    pub cover_letter: CoverLetter,
    pub diff_raw_json: String,
    pub quality_score: Option<crate::quality::QualityScore>,
    pub tone_label: Option<String>,
}
