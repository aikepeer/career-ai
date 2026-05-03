#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

const SAMPLE: &str = "Alice Kumar
alice.k@example.com
+91-9876543210
https://github.com/alicek
https://www.linkedin.com/in/alicek

SUMMARY
Backend engineer with 6 years of Rust/Python.

SKILLS
Rust, Python, Tokio, SQLx, Linux

EXPERIENCE
Senior Engineer — Acme Robotics
Jan 2022 - Present
- Led migration to async Rust stack.
- Cut p99 latency 40%.

Engineer — BetaCorp
Jun 2019 - Dec 2021
- Built data ingest pipeline.

EDUCATION
B.Tech Computer Science — IIT Delhi
2015 - 2019

PROJECTS
ros2-rust-bridge https://github.com/alicek/ros2-rust-bridge
- ROS2 control plane with async Rust.
";

#[test]
fn classifies_section_headers() {
    assert_eq!(classify_header("EXPERIENCE"), Some("experience"));
    assert_eq!(classify_header("  Work Experience  "), Some("experience"));
    assert_eq!(classify_header("Projects"), Some("projects"));
    assert_eq!(classify_header("Not a header"), None);
    assert_eq!(classify_header("alice@example.com"), None);
}

#[test]
fn personal_block_extracts_contact_info() {
    let p = parse(SAMPLE);
    assert_eq!(p.personal.name, "Alice Kumar");
    assert_eq!(p.personal.email, "alice.k@example.com");
    assert_eq!(p.personal.links.github, "https://github.com/alicek");
    assert!(p.personal.phone.starts_with("+91"));
}

#[test]
fn experience_blocks_split_on_blank_lines() {
    let p = parse(SAMPLE);
    assert_eq!(p.experience.len(), 2);
    assert_eq!(p.experience[0].title, "Senior Engineer");
    assert_eq!(p.experience[0].company, "Acme Robotics");
    assert_eq!(p.experience[0].end, "present");
    assert_eq!(p.experience[0].bullets.len(), 2);
    assert!(p.experience[0].bullets[0].starts_with("Led migration"));
}

#[test]
fn education_block_extracted() {
    let p = parse(SAMPLE);
    assert_eq!(p.education.len(), 1);
    assert_eq!(p.education[0].degree, "B.Tech Computer Science");
    assert_eq!(p.education[0].institution, "IIT Delhi");
}

#[test]
fn projects_block_extracted_with_url() {
    let p = parse(SAMPLE);
    assert_eq!(p.projects.len(), 1);
    assert_eq!(p.projects[0].name, "ros2-rust-bridge");
    assert!(p.projects[0].url.starts_with("https://github.com"));
}

#[test]
fn summary_line_trimmed() {
    let p = parse(SAMPLE);
    assert!(p.summary.contains("Backend engineer"));
}
