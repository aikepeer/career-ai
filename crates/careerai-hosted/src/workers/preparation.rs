//! Preparation program generation with evidence-linked tasks (PR 6).
//!
//! Given a curated company record and listing evidence, produces a
//! structured preparation program: fit map, STAR stories, interview
//! questions, recruiter questions, benefits checklist, and manual-review
//! tasks for stale/empty sections.

use serde::{Deserialize, Serialize};

/// A curated company record used to ground preparation programs.
///
/// In production this comes from a reviewed allowlist; in tests it is
/// a fixture. Stale or empty records produce `unavailable` sections
/// with manual-review tasks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompanyRecord {
    pub name: String,
    pub values: Vec<String>,
    pub interview_process: Vec<String>,
    pub benefits: Vec<String>,
    pub known_questions: Vec<String>,
}

/// A piece of evidence from a job listing used to ground tasks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListingEvidence {
    pub listing_id: String,
    pub title: String,
    pub required_skills: Vec<String>,
    pub description_snippet: String,
    pub score: Option<f64>,
}

/// Request to generate a preparation program.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreparationRequest {
    pub company_record: CompanyRecord,
    pub listing_evidence: Vec<ListingEvidence>,
    pub profile_skills: Vec<String>,
}

/// Status of a preparation section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionStatus {
    /// Section is populated with grounded content.
    Available,
    /// Section cannot be generated due to stale/empty input.
    Unavailable,
}

/// A single evidence-linked task in the preparation program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparationTask {
    pub title: String,
    pub description: String,
    pub evidence_ref: Option<String>,
    pub task_type: TaskType,
}

/// Type of preparation task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    FitMap,
    StarStory,
    InterviewQuestion,
    RecruiterQuestion,
    BenefitsChecklist,
    ManualReview,
}

/// A section of the preparation program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparationSection {
    pub title: String,
    pub status: SectionStatus,
    pub tasks: Vec<PreparationTask>,
}

/// The full preparation program.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreparationProgram {
    pub company_name: String,
    pub sections: Vec<PreparationSection>,
}

/// Generate a preparation program from a request.
///
/// Each section is grounded in the company record and listing evidence.
/// Stale or empty inputs produce `unavailable` sections with manual-review
/// tasks rather than fabricated content.
pub fn generate_program(req: &PreparationRequest) -> PreparationProgram {
    let sections = vec![
        generate_fit_map(req),
        generate_star_stories(req),
        generate_interview_questions(req),
        generate_recruiter_questions(req),
        generate_benefits_checklist(req),
    ];

    PreparationProgram {
        company_name: req.company_record.name.clone(),
        sections,
    }
}

fn generate_fit_map(req: &PreparationRequest) -> PreparationSection {
    let mut tasks = Vec::new();

    for evidence in &req.listing_evidence {
        let matched: Vec<&String> = evidence
            .required_skills
            .iter()
            .filter(|s| req.profile_skills.iter().any(|p| p.eq_ignore_ascii_case(s)))
            .collect();

        let gaps: Vec<&String> = evidence
            .required_skills
            .iter()
            .filter(|s| !req.profile_skills.iter().any(|p| p.eq_ignore_ascii_case(s)))
            .collect();

        let title = format!("Fit analysis: {}", evidence.title);
        let desc = if matched.is_empty() && gaps.is_empty() {
            "No skill data available for analysis.".to_string()
        } else {
            format!(
                "Matched: {}. Gaps: {}.",
                matched.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                gaps.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            )
        };

        tasks.push(PreparationTask {
            title,
            description: desc,
            evidence_ref: Some(evidence.listing_id.clone()),
            task_type: TaskType::FitMap,
        });
    }

    let status = if tasks.is_empty() {
        SectionStatus::Unavailable
    } else {
        SectionStatus::Available
    };

    if tasks.is_empty() {
        tasks.push(manual_review_task(
            "Fit map unavailable — no listing evidence provided",
        ));
    }

    PreparationSection {
        title: "Fit Map".to_string(),
        status,
        tasks,
    }
}

fn generate_star_stories(req: &PreparationRequest) -> PreparationSection {
    let mut tasks = Vec::new();

    for evidence in &req.listing_evidence {
        tasks.push(PreparationTask {
            title: format!("STAR story for {}", evidence.title),
            description: format!(
                "Prepare a STAR (Situation, Task, Action, Result) story \
                 demonstrating experience with: {}",
                evidence.required_skills.join(", ")
            ),
            evidence_ref: Some(evidence.listing_id.clone()),
            task_type: TaskType::StarStory,
        });
    }

    let status = if tasks.is_empty() {
        SectionStatus::Unavailable
    } else {
        SectionStatus::Available
    };

    if tasks.is_empty() {
        tasks.push(manual_review_task(
            "STAR stories unavailable — no listing evidence provided",
        ));
    }

    PreparationSection {
        title: "STAR Stories".to_string(),
        status,
        tasks,
    }
}

fn generate_interview_questions(req: &PreparationRequest) -> PreparationSection {
    let mut tasks = Vec::new();
    let company = &req.company_record;

    for q in &company.known_questions {
        tasks.push(PreparationTask {
            title: format!("Prepare answer: {q}"),
            description: format!(
                "Known interview question for {}. Prepare a structured answer.",
                company.name
            ),
            evidence_ref: None,
            task_type: TaskType::InterviewQuestion,
        });
    }

    for evidence in &req.listing_evidence {
        for skill in &evidence.required_skills {
            tasks.push(PreparationTask {
                title: format!("Technical question on {skill}"),
                description: format!(
                    "Listing {} requires {skill}. Prepare a technical \
                     explanation and a project example.",
                    evidence.title
                ),
                evidence_ref: Some(evidence.listing_id.clone()),
                task_type: TaskType::InterviewQuestion,
            });
        }
    }

    let status = if tasks.is_empty() {
        SectionStatus::Unavailable
    } else {
        SectionStatus::Available
    };

    if tasks.is_empty() {
        tasks.push(manual_review_task(
            "Interview questions unavailable — no company record or listing evidence",
        ));
    }

    PreparationSection {
        title: "Interview Questions".to_string(),
        status,
        tasks,
    }
}

fn generate_recruiter_questions(req: &PreparationRequest) -> PreparationSection {
    let mut tasks = Vec::new();
    let company = &req.company_record;

    for benefit in &company.benefits {
        tasks.push(PreparationTask {
            title: format!("Ask about: {benefit}"),
            description: format!(
                "Benefit to discuss with recruiter at {}.",
                company.name
            ),
            evidence_ref: None,
            task_type: TaskType::RecruiterQuestion,
        });
    }

    for value in &company.values {
        tasks.push(PreparationTask {
            title: format!("Discuss company value: {value}"),
            description: format!(
                "Align your experience with {}'s stated value: {value}.",
                company.name
            ),
            evidence_ref: None,
            task_type: TaskType::RecruiterQuestion,
        });
    }

    let status = if tasks.is_empty() {
        SectionStatus::Unavailable
    } else {
        SectionStatus::Available
    };

    if tasks.is_empty() {
        tasks.push(manual_review_task(
            "Recruiter questions unavailable — no company benefits or values on record",
        ));
    }

    PreparationSection {
        title: "Recruiter Questions".to_string(),
        status,
        tasks,
    }
}

fn generate_benefits_checklist(req: &PreparationRequest) -> PreparationSection {
    let mut tasks = Vec::new();
    let company = &req.company_record;

    for benefit in &company.benefits {
        tasks.push(PreparationTask {
            title: format!("Verify: {benefit}"),
            description: format!(
                "Confirm the details and eligibility for: {benefit} at {}.",
                company.name
            ),
            evidence_ref: None,
            task_type: TaskType::BenefitsChecklist,
        });
    }

    let status = if tasks.is_empty() {
        SectionStatus::Unavailable
    } else {
        SectionStatus::Available
    };

    if tasks.is_empty() {
        tasks.push(manual_review_task(
            "Benefits checklist unavailable — no company benefits on record",
        ));
    }

    PreparationSection {
        title: "Benefits Checklist".to_string(),
        status,
        tasks,
    }
}

fn manual_review_task(desc: &str) -> PreparationTask {
    PreparationTask {
        title: "Manual review needed".to_string(),
        description: desc.to_string(),
        evidence_ref: None,
        task_type: TaskType::ManualReview,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "preparation_tests.rs"]
mod tests;
