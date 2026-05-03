#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::schema::{Experience, Personal, Profile, Skills};

fn exp(company: &str, title: &str, start: &str, bullets: &[&str]) -> Experience {
    Experience {
        title: title.into(),
        company: company.into(),
        start: start.into(),
        end: "present".into(),
        bullets: bullets.iter().map(|b| (*b).to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn merges_personal_fields_preferring_base_then_filling_empties() {
    let base = Profile {
        personal: Personal {
            name: "Alice".into(),
            email: String::new(),
            ..Default::default()
        },
        ..Default::default()
    };
    let other = Profile {
        personal: Personal {
            name: "Ignored".into(),
            email: "a@example.com".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let merged = merge_pair(base, other);
    assert_eq!(merged.personal.name, "Alice");
    assert_eq!(merged.personal.email, "a@example.com");
}

#[test]
fn dedups_experience_and_merges_bullets() {
    let base = Profile {
        experience: vec![exp("Acme", "Engineer", "2022-01", &["A"])],
        ..Default::default()
    };
    let other = Profile {
        experience: vec![exp("ACME", "engineer", "2022-01", &["A", "B"])],
        ..Default::default()
    };
    let merged = merge_pair(base, other);
    assert_eq!(merged.experience.len(), 1);
    assert_eq!(merged.experience[0].bullets, vec!["A", "B"]);
}

#[test]
fn keeps_distinct_positions() {
    let base = Profile {
        experience: vec![exp("Acme", "Engineer", "2022-01", &[])],
        ..Default::default()
    };
    let other = Profile {
        experience: vec![exp("Beta", "Engineer", "2019-06", &[])],
        ..Default::default()
    };
    let merged = merge_pair(base, other);
    assert_eq!(merged.experience.len(), 2);
}

#[test]
fn dedups_skills_case_insensitive_keeping_first_casing() {
    let base = Profile {
        skills: Skills {
            languages: vec!["Rust".into(), "python".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let other = Profile {
        skills: Skills {
            languages: vec!["RUST".into(), "Go".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let merged = merge_pair(base, other);
    assert_eq!(merged.skills.languages, vec!["Rust", "python", "Go"]);
}

#[test]
fn summary_prefers_longer() {
    let base = Profile {
        summary: "short".into(),
        ..Default::default()
    };
    let other = Profile {
        summary: "a much longer summary".into(),
        ..Default::default()
    };
    let merged = merge_pair(base, other);
    assert_eq!(merged.summary, "a much longer summary");
}

#[test]
fn bullets_with_different_casing_are_kept_distinct() {
    let base = Profile {
        experience: vec![exp("Acme", "Engineer", "2022-01", &["Designed API"])],
        ..Default::default()
    };
    let other = Profile {
        experience: vec![exp(
            "ACME",
            "engineer",
            "2022-01",
            &["Designed api", "New work"],
        )],
        ..Default::default()
    };
    let merged = merge_pair(base, other);
    assert_eq!(merged.experience.len(), 1);
    assert_eq!(
        merged.experience[0].bullets,
        vec!["Designed API", "Designed api", "New work"]
    );
}

#[test]
fn empty_start_entries_are_never_deduped() {
    let base = Profile {
        experience: vec![exp("Acme", "Engineer", "", &["Early work"])],
        ..Default::default()
    };
    let other = Profile {
        experience: vec![exp("Acme", "Engineer", "", &["Later work"])],
        ..Default::default()
    };
    let merged = merge_pair(base, other);
    assert_eq!(merged.experience.len(), 2);
}

#[test]
fn linkedin_title_wins_over_pdf_date_garbage() {
    let linkedin = Profile {
        experience: vec![exp(
            "Acme Robotics",
            "Senior Embedded Developer",
            "2022-01",
            &["Led migration"],
        )],
        ..Default::default()
    };
    let pdf_garbage = Profile {
        experience: vec![exp(
            "Acme Robotics",
            "Jan 2025",
            "2022-01",
            &["Cut latency"],
        )],
        ..Default::default()
    };
    let merged = merge_pair(linkedin, pdf_garbage);
    assert_eq!(merged.experience.len(), 1);
    assert_eq!(merged.experience[0].title, "Senior Embedded Developer");
    assert!(merged.experience[0]
        .bullets
        .iter()
        .any(|b| b.contains("migration")));
    assert!(merged.experience[0]
        .bullets
        .iter()
        .any(|b| b.contains("latency")));
}

#[test]
fn pdf_title_wins_when_linkedin_is_garbage() {
    let bad_base = Profile {
        experience: vec![exp("Acme", "Jan 2022", "2022-01", &[])],
        ..Default::default()
    };
    let good_other = Profile {
        experience: vec![exp("Acme", "Engineer", "2022-01", &[])],
        ..Default::default()
    };
    let merged = merge_pair(bad_base, good_other);
    assert_eq!(merged.experience.len(), 1);
    assert_eq!(merged.experience[0].title, "Engineer");
}

#[test]
fn looks_like_date_garbage_classifies_known_shapes() {
    assert!(looks_like_date_garbage(""));
    assert!(looks_like_date_garbage("Jan 2025"));
    assert!(looks_like_date_garbage("January 2025"));
    assert!(looks_like_date_garbage("2022"));
    assert!(looks_like_date_garbage("2022-01"));
    assert!(looks_like_date_garbage("2022 - Present"));
    assert!(looks_like_date_garbage("Jan 2022 - Dec 2024"));
    assert!(!looks_like_date_garbage("Senior Embedded Developer"));
    assert!(!looks_like_date_garbage("Engineer III"));
    assert!(!looks_like_date_garbage("Marketing Lead"));
}
