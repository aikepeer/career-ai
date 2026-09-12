//! I/O worker adapters wiring hosted workers to existing crate logic.
//!
//! Each adapter wraps a call into careerai-profile, careerai-match,
//! careerai-tailor, or careerai-render, translating between the hosted
//! crate's serialized JSON job payloads and the concrete types the
//! existing crates expect.

use serde::{Deserialize, Serialize};

use careerai_match::score::{match_breakdown, JaccardScorer, Scorer};
use careerai_profile::Profile;
use careerai_sources::RawListing;

use super::preparation::{ListingEvidence, PreparationRequest};

// ── Profile import adapter ──────────────────────────────────────────

/// Request to import a profile from file paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileImportRequest {
    /// File paths to import (PDF, DOCX, or LinkedIn ZIP).
    pub paths: Vec<String>,
}

/// Result of a profile import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileImportResult {
    pub personal_name: String,
    pub summary: String,
    pub skill_count: usize,
    pub experience_count: usize,
    pub education_count: usize,
    pub project_count: usize,
}

/// Import a profile from file paths using careerai-profile's parser.
///
/// This calls `careerai_profile::import_paths` on the real files.
/// Returns a summary of the imported profile.
pub fn import_profile(req: &ProfileImportRequest) -> Result<ProfileImportResult, String> {
    let path_refs: Vec<&std::path::Path> = req.paths.iter().map(std::path::Path::new).collect();
    let profile = careerai_profile::import_paths(&path_refs)
        .map_err(|e| format!("profile import failed: {e}"))?;

    Ok(ProfileImportResult {
        personal_name: profile.personal.name,
        summary: profile.summary,
        skill_count: profile.skills.all_skill_names().count(),
        experience_count: profile.experience.len(),
        education_count: profile.education.len(),
        project_count: profile.projects.len(),
    })
}

// ── Match adapter ───────────────────────────────────────────────────

/// A listing to match against the profile, in serialized form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchListing {
    pub source: String,
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: String,
    pub description: String,
    pub raw_json: Option<String>,
}

/// Request to match listings against a profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchRequest {
    /// The flattened profile text for scoring.
    pub profile_text: String,
    /// Domain keywords to check for missing skills.
    pub domain_keywords: Vec<String>,
    /// Listings to score.
    pub listings: Vec<MatchListing>,
}

/// Result of matching a single listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub listing_id: String,
    pub score: f32,
    pub matched_keywords: Vec<String>,
    pub missing_keywords: Vec<String>,
}

/// Match listings against a profile using careerai-match's Jaccard scorer.
///
/// Returns scored listings with explainable match breakdowns.
#[must_use]
pub fn match_listings(req: &MatchRequest) -> Vec<MatchResult> {
    let scorer = JaccardScorer;
    let domain: Vec<String> = req.domain_keywords.clone();

    req.listings
        .iter()
        .map(|ml| {
            let raw = RawListing {
                source: ml.source.clone(),
                external_id: ml.external_id.clone(),
                title: ml.title.clone(),
                company: ml.company.clone(),
                location: ml.location.clone(),
                url: ml.url.clone(),
                description: ml.description.clone(),
                raw_json: ml.raw_json.clone(),
            };
            let score = scorer.score(&req.profile_text, &raw);
            let breakdown = match_breakdown(&req.profile_text, &raw, &domain);
            MatchResult {
                listing_id: ml.external_id.clone(),
                score,
                matched_keywords: breakdown.matched_keywords,
                missing_keywords: breakdown.missing_keywords,
            }
        })
        .collect()
}

// ── Tailor adapter ──────────────────────────────────────────────────

/// Request to tailor a resume for a listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailorRequest {
    /// The profile in YAML format (parsed by careerai-profile).
    pub profile_yaml: String,
    /// The listing to tailor for.
    pub listing: MatchListing,
    /// Drop threshold for bullet scoring (0.0-1.0).
    pub drop_threshold: f32,
}

/// Result of tailoring a resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailorResult {
    pub application_id: String,
    pub resume_view_json: String,
    pub cover_letter_body: String,
    pub diff_json: String,
}

/// Tailor a resume for a listing using careerai-tailor's local scorer.
///
/// This calls `careerai_tailor::local::tailor_local` which produces a
/// constrained resume view — it can only reorder or rewrite existing
/// bullets, never invent new content. The cover letter is generated
/// locally via skeleton slot-filling (no LLM required).
pub fn tailor_resume(req: &TailorRequest) -> Result<TailorResult, String> {
    let profile: Profile = serde_yaml::from_str(&req.profile_yaml)
        .map_err(|e| format!("profile YAML parse failed: {e}"))?;

    let listing = careerai_db::models::Listing {
        id: String::new(),
        source: req.listing.source.clone(),
        external_id: req.listing.external_id.clone(),
        title: req.listing.title.clone(),
        company: req.listing.company.clone(),
        location: req.listing.location.clone(),
        url: req.listing.url.clone(),
        description: req.listing.description.clone(),
        raw_json: req.listing.raw_json.clone(),
        state: "shortlisted".to_string(),
        score: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let scorer = careerai_match::JaccardBulletScorer;
    let resume_view =
        careerai_tailor::local::tailor_local(&profile, &listing, &scorer, req.drop_threshold)
            .map_err(|e| format!("tailoring failed: {e}"))?;

    let application_id = uuid::Uuid::new_v4().to_string();
    let resume_view_json = serde_json::to_string(&resume_view)
        .map_err(|e| format!("resume view serialization failed: {e}"))?;

    // Generate cover letter locally via skeleton slot-filling
    let skeletons = careerai_tailor::cover_skeleton::default_skeletons();
    let slots = careerai_tailor::cover_skeleton::extract_slots(&profile, &listing);
    let best = careerai_tailor::cover_skeleton::pick_best_skeleton(
        &skeletons,
        &format!("{} {}", listing.title, listing.description),
        &scorer,
    );
    let cover_body = careerai_tailor::cover_skeleton::fill_skeleton(best, &slots);

    // Build a diff JSON showing reorder ops (constrained grammar)
    let diff_ops: Vec<serde_json::Value> = resume_view
        .experience
        .iter()
        .flat_map(|exp| {
            exp.bullets
                .iter()
                .enumerate()
                .map(|(i, _)| serde_json::json!({"op": "reorder", "index": i}))
        })
        .collect();
    let diff_json = serde_json::json!({ "ops": diff_ops }).to_string();

    Ok(TailorResult {
        application_id,
        resume_view_json,
        cover_letter_body: cover_body,
        diff_json,
    })
}

// ── Render adapter ──────────────────────────────────────────────────

/// Request to render a tailored resume + cover letter to DOCX/PDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderRequest {
    /// The resume view JSON (from tailor_resume).
    pub resume_view_json: String,
    /// The cover letter body text.
    pub cover_letter_body: String,
    /// Application ID for the output directory.
    pub application_id: String,
    /// Personal name for the resume header.
    pub personal_name: String,
    /// Listing company name.
    pub listing_company: String,
}

/// Result of rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderResult {
    pub resume_md_path: String,
    pub resume_docx_path: String,
    pub resume_pdf_path: String,
    pub cover_md_path: String,
    pub cover_docx_path: String,
    pub content_digest: String,
}

/// Render a tailored resume + cover letter to DOCX/PDF using
/// careerai-render's pandoc pipeline.
///
/// This calls `careerai_render::render_application` which invokes
/// pandoc as a subprocess. Pandoc must be on PATH.
pub async fn render_artifacts(
    req: &RenderRequest,
    artifacts_dir: &str,
) -> Result<RenderResult, String> {
    let resume_view: careerai_tailor::model::ResumeView =
        serde_json::from_str(&req.resume_view_json)
            .map_err(|e| format!("resume view parse failed: {e}"))?;

    let cover_letter = careerai_tailor::model::CoverLetter {
        body: req.cover_letter_body.clone(),
    };

    let cfg = careerai_render::RenderConfig {
        artifacts_dir: std::path::PathBuf::from(artifacts_dir),
        ..Default::default()
    };

    let rendered = careerai_render::render_application(
        &cfg,
        &req.application_id,
        &resume_view,
        &cover_letter,
        &req.personal_name,
        &req.listing_company,
    )
    .await
    .map_err(|e| format!("rendering failed: {e}"))?;

    let resume_md_bytes = std::fs::read(&rendered.resume_md)
        .map_err(|e| format!("failed to read rendered resume: {e}"))?;
    let digest = super::artifacts::content_digest(&resume_md_bytes);

    Ok(RenderResult {
        resume_md_path: rendered.resume_md.to_string_lossy().to_string(),
        resume_docx_path: rendered.resume_docx.to_string_lossy().to_string(),
        resume_pdf_path: rendered.resume_pdf.to_string_lossy().to_string(),
        cover_md_path: rendered.cover_md.to_string_lossy().to_string(),
        cover_docx_path: rendered.cover_docx.to_string_lossy().to_string(),
        content_digest: digest,
    })
}

// ── Preparation program from profile + listings ─────────────────────

/// Build a preparation request from a profile and matched listings.
///
/// This bridges the match results into the preparation program generator
/// by extracting skill evidence from the matched listings.
#[must_use]
pub fn build_preparation_from_matches(
    profile: &Profile,
    matches: &[MatchResult],
    listings: &[MatchListing],
    company_name: &str,
) -> PreparationRequest {
    let profile_skills: Vec<String> = profile.skills.all_skill_names().cloned().collect();

    let listing_evidence: Vec<ListingEvidence> = matches
        .iter()
        .filter_map(|m| {
            let listing = listings.iter().find(|l| l.external_id == m.listing_id)?;
            Some(ListingEvidence {
                listing_id: listing.external_id.clone(),
                title: listing.title.clone(),
                required_skills: m.matched_keywords.clone(),
                description_snippet: listing.description.chars().take(200).collect(),
                score: Some(f64::from(m.score)),
            })
        })
        .collect();

    PreparationRequest {
        company_record: super::preparation::CompanyRecord {
            name: company_name.to_string(),
            ..Default::default()
        },
        listing_evidence,
        profile_skills,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn match_listings_scores_and_explains() {
        let req = MatchRequest {
            profile_text: "rust embedded linux python machine learning".to_string(),
            domain_keywords: vec!["rust".to_string(), "embedded".to_string()],
            listings: vec![MatchListing {
                source: "test".to_string(),
                external_id: "lst-001".to_string(),
                title: "Senior Rust Engineer".to_string(),
                company: "Acme".to_string(),
                location: None,
                url: "https://example.com".to_string(),
                description: "Build embedded systems in Rust and Python".to_string(),
                raw_json: None,
            }],
        };
        let results = match_listings(&req);
        assert_eq!(results.len(), 1);
        assert!(results[0].score > 0.0);
        assert!(results[0].matched_keywords.contains(&"rust".to_string()));
        assert!(results[0]
            .matched_keywords
            .contains(&"embedded".to_string()));
    }

    #[test]
    fn match_listings_reports_missing_keywords() {
        let req = MatchRequest {
            profile_text: "python web".to_string(),
            domain_keywords: vec!["rust".to_string(), "embedded".to_string()],
            listings: vec![MatchListing {
                source: "test".to_string(),
                external_id: "lst-001".to_string(),
                title: "Engineer".to_string(),
                company: "Acme".to_string(),
                location: None,
                url: "https://example.com".to_string(),
                description: "Build things".to_string(),
                raw_json: None,
            }],
        };
        let results = match_listings(&req);
        assert!(results[0].missing_keywords.contains(&"rust".to_string()));
        assert!(results[0]
            .missing_keywords
            .contains(&"embedded".to_string()));
    }

    #[test]
    fn build_preparation_extracts_skills_from_profile() {
        let mut profile = Profile::default();
        profile.personal.name = "Test User".to_string();
        profile.skills.languages.push("Rust".to_string());
        profile.skills.languages.push("Python".to_string());

        let matches = vec![MatchResult {
            listing_id: "lst-001".to_string(),
            score: 0.5,
            matched_keywords: vec!["rust".to_string()],
            missing_keywords: vec![],
        }];
        let listings = vec![MatchListing {
            source: "test".to_string(),
            external_id: "lst-001".to_string(),
            title: "Rust Engineer".to_string(),
            company: "Acme".to_string(),
            location: None,
            url: "https://example.com".to_string(),
            description: "Build things in Rust".to_string(),
            raw_json: None,
        }];

        let prep_req = build_preparation_from_matches(&profile, &matches, &listings, "Acme");
        assert_eq!(prep_req.company_record.name, "Acme");
        assert_eq!(prep_req.listing_evidence.len(), 1);
        assert!(prep_req.profile_skills.len() >= 2);
        assert_eq!(prep_req.listing_evidence[0].listing_id, "lst-001");
        assert!(prep_req.listing_evidence[0]
            .required_skills
            .contains(&"rust".to_string()));
    }

    #[test]
    fn import_profile_rejects_nonexistent_path() {
        let req = ProfileImportRequest {
            paths: vec!["/nonexistent/file.pdf".to_string()],
        };
        let err = import_profile(&req).unwrap_err();
        assert!(err.contains("profile import failed"));
    }

    #[test]
    fn match_listings_empty_input_returns_empty() {
        let req = MatchRequest {
            profile_text: String::new(),
            domain_keywords: vec![],
            listings: vec![],
        };
        let results = match_listings(&req);
        assert!(results.is_empty());
    }
}
