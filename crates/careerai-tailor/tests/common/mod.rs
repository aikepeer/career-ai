//! Shared fixtures for integration tests.

#![allow(dead_code)]

use careerai_profile::schema::{Experience, Personal, Profile, Project, Skills};

pub fn fixture_profile() -> Profile {
    Profile {
        personal: Personal {
            name: "Ada Lovelace".into(),
            email: "ada@example.com".into(),
            ..Default::default()
        },
        summary: "Senior Rust engineer with embedded experience.".into(),
        skills: Skills {
            languages: vec!["Rust".into(), "Python".into()],
            frameworks: vec!["Tokio".into(), "Kubernetes".into()],
            tools: vec!["SQLite".into()],
            ..Default::default()
        },
        experience: vec![
            Experience {
                title: "Senior Engineer".into(),
                company: "Acme Robotics".into(),
                location: "Remote".into(),
                start: "2022-01".into(),
                end: "present".into(),
                bullets: vec![
                    "Shipped Rust LLM pipeline reducing latency 35%.".into(),
                    "Led team of 4 on embedded perception.".into(),
                ],
            },
            Experience {
                title: "Engineer".into(),
                company: "Widget Corp".into(),
                location: String::new(),
                start: "2018-06".into(),
                end: "2021-12".into(),
                bullets: vec!["Built Python services on Kubernetes.".into()],
            },
        ],
        education: vec![],
        projects: vec![Project {
            name: "openLLM".into(),
            url: "https://example.com/openllm".into(),
            bullets: vec!["Tokenizer in Rust supporting 5 languages.".into()],
        }],
        ..Default::default()
    }
}

pub const VALID_DIFF_JSON: &str = r#"{
    "prompt_version":"tailor.v1",
    "summary":{"op":"keep"},
    "ops":[
        {"path":"experience[0].bullets[0]","op":"keep"},
        {"path":"experience[0].bullets[1]","op":"keep"},
        {"path":"experience[1].bullets[0]","op":"keep"},
        {"path":"projects[0].bullets[0]","op":"keep"}
    ],
    "cover_letter":"short"
}"#;
