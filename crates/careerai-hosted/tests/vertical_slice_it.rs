//! Vertical slice integration test: match → tailor → preparation.
//!
//! Exercises the full hosted pipeline with curated company-record
//! fixtures, driving real crate logic through the adapter functions.
//! No mocks — every step calls the shipped production code path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod fixtures;
use fixtures::*;

use careerai_hosted::workers::adapters::{
    build_preparation_from_matches, match_listings, tailor_resume, MatchRequest, TailorRequest,
};
use careerai_hosted::workers::preparation::{generate_program, SectionStatus, TaskType};
use careerai_profile::Profile;

fn profile_text(profile: &Profile) -> String {
    format!(
        "{} {} {}",
        profile.summary,
        profile
            .skills
            .all_skill_names()
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
        profile
            .experience
            .iter()
            .flat_map(|e| e.bullets.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn vertical_slice_google_match_tailor_prepare() {
    let profile_yaml = sample_profile_yaml();
    let profile: Profile = serde_yaml::from_str(&profile_yaml).unwrap();
    let ptext = profile_text(&profile);

    let listings = vec![google_listing(), anthropic_listing(), mismatch_listing()];

    // 1. MATCH
    let match_req = MatchRequest {
        profile_text: ptext,
        domain_keywords: vec![
            "rust".to_string(),
            "python".to_string(),
            "pytorch".to_string(),
            "kubernetes".to_string(),
            "ml".to_string(),
        ],
        listings: listings.clone(),
    };
    let match_results = match_listings(&match_req);

    assert_eq!(match_results.len(), 3, "should score all 3 listings");
    assert!(
        match_results[0].score > 0.0,
        "Google listing should have positive score"
    );
    assert!(
        match_results[1].score > 0.0,
        "Anthropic listing should have positive score"
    );

    let google_score = match_results
        .iter()
        .find(|m| m.listing_id == "goog-001")
        .unwrap()
        .score;
    let mismatch_score = match_results
        .iter()
        .find(|m| m.listing_id == "mismatch-001")
        .unwrap()
        .score;
    assert!(
        google_score > mismatch_score,
        "Google ({google_score}) should outscore mismatch ({mismatch_score})"
    );

    // 2. TAILOR
    let google_lst = listings
        .iter()
        .find(|l| l.external_id == "goog-001")
        .unwrap();
    let tailor_req = TailorRequest {
        profile_yaml: profile_yaml.clone(),
        listing: google_lst.clone(),
        drop_threshold: 0.1,
    };
    let tailor_result = tailor_resume(&tailor_req).unwrap();

    let rv: serde_json::Value = serde_json::from_str(&tailor_result.resume_view_json).unwrap();
    assert!(
        rv.get("personal").is_some(),
        "resume view should have personal section"
    );
    assert!(
        rv.get("experience").is_some(),
        "resume view should have experience section"
    );
    assert!(
        rv.get("skills").is_some(),
        "resume view should have skills section"
    );
    assert!(
        !tailor_result.cover_letter_body.is_empty(),
        "cover letter should be generated"
    );

    let diff: serde_json::Value = serde_json::from_str(&tailor_result.diff_json).unwrap();
    assert!(diff.get("ops").is_some(), "diff should have ops array");

    // 3. PREPARE
    let mut prep_req =
        build_preparation_from_matches(&profile, &match_results, &listings, "Google");
    prep_req.company_record = google_company_record();
    let program = generate_program(&prep_req);

    assert_eq!(program.company_name, "Google");
    assert_eq!(program.sections.len(), 5, "should have 5 sections");

    for section in &program.sections {
        assert_eq!(
            section.status,
            SectionStatus::Available,
            "section '{}' should be available with curated data",
            section.title
        );
    }

    // Fit map should cite listing evidence
    let fit_map = &program.sections[0];
    assert!(
        fit_map.tasks.iter().any(|t| t.evidence_ref.is_some()),
        "fit map should reference listing evidence"
    );

    // Interview questions should include known questions
    let interview_q = &program.sections[2];
    assert!(interview_q
        .tasks
        .iter()
        .any(|t| t.title.contains("ambiguity")));
    assert!(interview_q
        .tasks
        .iter()
        .any(|t| t.title.contains("1B users")));

    // Recruiter questions should reference benefits and values
    let recruiter_q = &program.sections[3];
    assert!(recruiter_q
        .tasks
        .iter()
        .any(|t| t.title.contains("Relocation")));
    assert!(recruiter_q
        .tasks
        .iter()
        .any(|t| t.title.contains("Focus on the user")));

    // Benefits checklist should have verify tasks
    let benefits = &program.sections[4];
    assert!(benefits
        .tasks
        .iter()
        .all(|t| t.task_type == TaskType::BenefitsChecklist));
    assert!(benefits
        .tasks
        .iter()
        .any(|t| t.title.contains("Stock refreshers")));
    assert!(benefits
        .tasks
        .iter()
        .any(|t| t.title.contains("Education stipend")));
}

#[test]
fn vertical_slice_anthropic_match_tailor_prepare() {
    let profile_yaml = sample_profile_yaml();
    let profile: Profile = serde_yaml::from_str(&profile_yaml).unwrap();
    let ptext = profile_text(&profile);

    let listings = vec![anthropic_listing()];

    let match_req = MatchRequest {
        profile_text: ptext,
        domain_keywords: vec![
            "rust".to_string(),
            "python".to_string(),
            "pytorch".to_string(),
        ],
        listings: listings.clone(),
    };
    let match_results = match_listings(&match_req);
    assert_eq!(match_results.len(), 1);
    assert!(
        match_results[0].score > 0.0,
        "Anthropic listing should match"
    );

    // Tailor
    let tailor_req = TailorRequest {
        profile_yaml,
        listing: listings[0].clone(),
        drop_threshold: 0.1,
    };
    let tailor_result = tailor_resume(&tailor_req).unwrap();
    assert!(!tailor_result.cover_letter_body.is_empty());

    // Prepare with Anthropic company record
    let mut prep_req =
        build_preparation_from_matches(&profile, &match_results, &listings, "Anthropic");
    prep_req.company_record = anthropic_company_record();
    let program = generate_program(&prep_req);

    assert_eq!(program.company_name, "Anthropic");

    let interview_q = &program.sections[2];
    assert!(interview_q.tasks.iter().any(|t| t.title.contains("align")));

    let recruiter_q = &program.sections[3];
    assert!(recruiter_q
        .tasks
        .iter()
        .any(|t| t.title.contains("Safety first")));
}

#[test]
fn vertical_slice_stale_company_produces_manual_review() {
    let profile_yaml = sample_profile_yaml();
    let profile: Profile = serde_yaml::from_str(&profile_yaml).unwrap();
    let ptext = profile_text(&profile);

    let listings = vec![google_listing()];
    let match_req = MatchRequest {
        profile_text: ptext,
        domain_keywords: vec!["rust".to_string()],
        listings: listings.clone(),
    };
    let match_results = match_listings(&match_req);

    let prep_req = build_preparation_from_matches(&profile, &match_results, &listings, "UnknownCo");
    let program = generate_program(&prep_req);

    assert_eq!(program.company_name, "UnknownCo");

    // Interview questions available from listing evidence even with stale company
    let interview_q = &program.sections[2];
    assert_eq!(interview_q.status, SectionStatus::Available);

    // Recruiter questions unavailable (no values or benefits)
    let recruiter_q = &program.sections[3];
    assert_eq!(recruiter_q.status, SectionStatus::Unavailable);
    assert!(recruiter_q
        .tasks
        .iter()
        .any(|t| t.task_type == TaskType::ManualReview));

    // Benefits unavailable
    let benefits = &program.sections[4];
    assert_eq!(benefits.status, SectionStatus::Unavailable);
}

#[test]
fn vertical_slice_tailor_produces_constrained_diff() {
    let profile_yaml = sample_profile_yaml();
    let listing = google_listing();

    let req = TailorRequest {
        profile_yaml,
        listing,
        drop_threshold: 0.05,
    };
    let result = tailor_resume(&req).unwrap();

    let diff: serde_json::Value = serde_json::from_str(&result.diff_json).unwrap();
    let ops = diff.get("ops").and_then(|o| o.as_array()).unwrap();

    for op in ops {
        let op_type = op.get("op").and_then(|t| t.as_str()).unwrap_or("");
        assert_eq!(
            op_type, "reorder",
            "diff should only contain reorder ops, found: {op_type}"
        );
    }

    let rv: serde_json::Value = serde_json::from_str(&result.resume_view_json).unwrap();
    let name = rv
        .get("personal")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    assert_eq!(
        name, "Jane Developer",
        "resume should preserve original name"
    );
}
