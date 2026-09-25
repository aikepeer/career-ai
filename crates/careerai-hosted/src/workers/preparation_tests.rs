use super::*;

fn fixture_company() -> CompanyRecord {
    CompanyRecord {
        name: "Acme Corp".to_string(),
        values: vec!["Ownership".to_string()],
        interview_process: vec!["Phone screen → onsite".to_string()],
        benefits: vec!["Remote-first".to_string(), "Learning stipend".to_string()],
        known_questions: vec!["Tell me about a challenging project.".to_string()],
    }
}

fn fixture_evidence() -> ListingEvidence {
    ListingEvidence {
        listing_id: "lst-001".to_string(),
        title: "Senior ML Engineer".to_string(),
        required_skills: vec!["Python".to_string(), "PyTorch".to_string()],
        description_snippet: "Build ML systems.".to_string(),
        score: Some(0.85),
    }
}

#[test]
fn program_has_five_sections() {
    let req = PreparationRequest {
        company_record: fixture_company(),
        listing_evidence: vec![fixture_evidence()],
        profile_skills: vec!["Python".to_string()],
    };
    let program = generate_program(&req);
    assert_eq!(program.sections.len(), 5);
    assert_eq!(program.company_name, "Acme Corp");
}

#[test]
fn fit_map_cites_listing_evidence() {
    let req = PreparationRequest {
        company_record: fixture_company(),
        listing_evidence: vec![fixture_evidence()],
        profile_skills: vec!["Python".to_string()],
    };
    let program = generate_program(&req);
    let fit = &program.sections[0];
    assert_eq!(fit.status, SectionStatus::Available);
    assert_eq!(fit.tasks.len(), 1);
    assert_eq!(fit.tasks[0].evidence_ref.as_deref(), Some("lst-001"));
    assert!(fit.tasks[0].description.contains("Matched: Python"));
    assert!(fit.tasks[0].description.contains("Gaps: PyTorch"));
}

#[test]
fn stale_company_produces_unavailable_sections() {
    let req = PreparationRequest {
        company_record: CompanyRecord::default(),
        listing_evidence: vec![],
        profile_skills: vec![],
    };
    let program = generate_program(&req);
    for section in &program.sections {
        assert_eq!(
            section.status,
            SectionStatus::Unavailable,
            "{} should be unavailable",
            section.title
        );
        assert!(!section.tasks.is_empty());
        assert_eq!(section.tasks[0].task_type, TaskType::ManualReview);
    }
}

#[test]
fn interview_questions_include_known_and_technical() {
    let req = PreparationRequest {
        company_record: fixture_company(),
        listing_evidence: vec![fixture_evidence()],
        profile_skills: vec![],
    };
    let program = generate_program(&req);
    let iq = program
        .sections
        .iter()
        .find(|s| s.title == "Interview Questions")
        .unwrap();
    assert!(iq
        .tasks
        .iter()
        .any(|t| t.title.contains("challenging project")));
    assert!(iq
        .tasks
        .iter()
        .any(|t| t.title.contains("Technical question on Python")));
}

#[test]
fn benefits_checklist_has_verify_tasks() {
    let req = PreparationRequest {
        company_record: fixture_company(),
        listing_evidence: vec![],
        profile_skills: vec![],
    };
    let program = generate_program(&req);
    let bc = program
        .sections
        .iter()
        .find(|s| s.title == "Benefits Checklist")
        .unwrap();
    assert_eq!(bc.status, SectionStatus::Available);
    assert_eq!(bc.tasks.len(), 2);
    assert!(bc.tasks[0].title.starts_with("Verify:"));
}
