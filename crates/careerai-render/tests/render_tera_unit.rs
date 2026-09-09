//! Unit tests for the Tera renderer — deterministic, no subprocess.
//!
//! The resume fixture is snapshot-tested via `insta`. Cover letter is
//! validated by substring check to keep the snapshot surface small.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_profile::schema::{Education, Links, Personal, Skills};
use careerai_render::templates::{render_cover_letter, render_resume, render_resume_html};
use careerai_tailor::model::{CoverLetter, ExperienceView, ProjectView, ResumeView};

fn fixture_view() -> ResumeView {
    ResumeView {
        personal: Personal {
            name: "Jane Doe".into(),
            email: "jane@example.com".into(),
            phone: "+1-555-0100".into(),
            location: "Remote".into(),
            links: Links {
                github: "https://github.com/jane".into(),
                linkedin: "https://linkedin.com/in/jane".into(),
                portfolio: String::new(),
            },
        },
        summary: "Backend engineer focused on distributed systems.".into(),
        skills: Skills {
            languages: vec!["Rust".into(), "Python".into(), "Go".into()],
            frameworks: vec!["Tokio".into(), "Actix".into()],
            tools: vec!["Docker".into(), "Kubernetes".into(), "Postgres".into()],
            ..Default::default()
        },
        experience: vec![
            ExperienceView {
                title: "Senior Engineer".into(),
                company: "Acme Corp".into(),
                location: Some("Remote".into()),
                start: "2022-01".into(),
                end: "present".into(),
                bullets: vec![
                    "Shipped production pipelines".into(),
                    "Owned core infra".into(),
                ],
            },
            ExperienceView {
                title: "Engineer".into(),
                company: "Globex".into(),
                location: None,
                start: "2019-06".into(),
                end: "2021-12".into(),
                bullets: vec![
                    "Reduced latency by 40%".into(),
                    "Led migration to Rust".into(),
                    "Mentored three engineers".into(),
                ],
            },
        ],
        education: vec![
            Education {
                degree: "BSc Computer Science".into(),
                institution: "State University".into(),
                start: "2015".into(),
                end: "2019".into(),
                ..Default::default()
            },
            Education {
                degree: "Advanced Algorithms Cert".into(),
                institution: "MOOC Academy".into(),
                start: "2020".into(),
                end: "2020".into(),
                ..Default::default()
            },
        ],
        projects: vec![ProjectView {
            name: "careerai".into(),
            url: Some("https://example.com/careerai".into()),
            bullets: vec!["Open-source job automation in Rust".into()],
        }],
    }
}

#[test]
fn resume_markdown_snapshot() {
    let view = fixture_view();
    let md = render_resume(&view, &view.personal.name).unwrap();
    insta::assert_snapshot!(md);
}

#[test]
fn resume_markdown_contains_core_structure() {
    let view = fixture_view();
    let md = render_resume(&view, &view.personal.name).unwrap();
    assert!(md.contains("# Jane Doe"));
    assert!(md.contains("## Summary"));
    assert!(md.contains("## Experience"));
    assert!(md.contains("## Education"));
    assert!(md.contains("## Skills"));
    assert!(md.contains("Senior Engineer"));
    assert!(md.contains("Acme Corp"));
    assert!(md.contains("Globex"));
    assert!(md.contains("BSc Computer Science"));
    assert!(md.contains("Rust, Python, Go"));
}

#[test]
fn resume_renders_without_github_link_when_empty() {
    let mut view = fixture_view();
    view.personal.links.github = String::new();
    let md = render_resume(&view, &view.personal.name).unwrap();
    assert!(!md.contains("[GitHub]"), "GitHub link should be omitted");
    // LinkedIn still present.
    assert!(md.contains("[LinkedIn]"));
}

#[test]
fn cover_letter_contains_key_substrings() {
    let letter = CoverLetter {
        body: "I am excited to apply for this role.".into(),
    };
    let md = render_cover_letter(&letter, "Jane Doe", "Acme Corp", "2026-04-24").unwrap();
    assert!(md.starts_with("2026-04-24"));
    assert!(md.contains("Hiring Team"));
    assert!(md.contains("Acme Corp"));
    assert!(md.contains("Dear Hiring Team,"));
    assert!(md.contains("I am excited to apply for this role."));
    assert!(md.contains("Best,\nJane Doe"));
}

#[test]
fn resume_html_contains_core_structure() {
    let view = fixture_view();
    let html = render_resume_html(
        &view,
        &view.personal.name,
        Some("Senior Embedded Architect"),
    )
    .unwrap();
    assert!(html.contains("Jane Doe"));
    assert!(html.contains("Senior Embedded Architect"));
    assert!(html.contains("Profile Summary"));
    assert!(html.contains("Experience"));
    assert!(html.contains("Technical Skills"));
    assert!(html.contains("Education"));
}

#[test]
fn resume_html_uses_experience_title_when_headline_is_none() {
    let view = fixture_view();
    let html = render_resume_html(&view, &view.personal.name, None).unwrap();
    assert!(html.contains("Jane Doe"));
    assert!(html.contains(r#"<div class="candidate-title">Senior Engineer</div>"#));
    assert!(html.contains(
        r#"<div class="summary-text">Backend engineer focused on distributed systems.</div>"#
    ));
}

#[test]
fn resume_renders_honors_and_achievements() {
    let mut view = fixture_view();
    view.education[0].achievements = vec![
        "Secured All India 33rd rank in HackerEarth Deep Learning Challenge".into(),
        "Capgemini Best Performance Award".into(),
    ];
    let md = render_resume(&view, &view.personal.name).unwrap();
    assert!(md.contains("## Honors & Achievements"));
    assert!(md.contains("Secured All India 33rd rank in HackerEarth Deep Learning Challenge"));
    assert!(md.contains("Capgemini Best Performance Award"));

    let html = render_resume_html(&view, &view.personal.name, None).unwrap();
    assert!(html.contains("Honors & Achievements"));
    assert!(html.contains("Secured All India 33rd rank in HackerEarth Deep Learning Challenge"));
    assert!(html.contains("Capgemini Best Performance Award"));
}
