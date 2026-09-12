//! Curated company-record fixtures for vertical slice integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use careerai_hosted::workers::adapters::{MatchListing};
use careerai_hosted::workers::preparation::CompanyRecord;

pub fn google_company_record() -> CompanyRecord {
    CompanyRecord {
        name: "Google".to_string(),
        values: vec![
            "Focus on the user".to_string(),
            "Do the right thing".to_string(),
        ],
        interview_process: vec![
            "Recruiter screen".to_string(),
            "Technical phone screen".to_string(),
            "Onsite: 4 rounds".to_string(),
        ],
        benefits: vec![
            "Relocation bonus".to_string(),
            "Stock refreshers".to_string(),
            "Education stipend".to_string(),
        ],
        known_questions: vec![
            "Tell me about a time you dealt with ambiguity".to_string(),
            "Design a system that scales to 1B users".to_string(),
        ],
    }
}

pub fn anthropic_company_record() -> CompanyRecord {
    CompanyRecord {
        name: "Anthropic".to_string(),
        values: vec![
            "Safety first".to_string(),
            "Iterative deployment".to_string(),
        ],
        interview_process: vec![
            "Recruiter call".to_string(),
            "Take-home exercise".to_string(),
            "Virtual onsite".to_string(),
        ],
        benefits: vec![
            "Unlimited PTO".to_string(),
            "Commuter benefits".to_string(),
        ],
        known_questions: vec![
            "How would you align an AI system to human values?".to_string(),
        ],
    }
}

pub fn sample_profile_yaml() -> String {
    r#"
personal:
  name: "Jane Developer"
  email: "jane@example.com"
  phone: "+1-555-0100"
summary: "Senior ML engineer with 8 years building production systems."
skills:
  languages:
    - Rust
    - Python
    - C++
  frameworks:
    - PyTorch
    - Tokio
  platforms:
    - Linux
    - Kubernetes
  devops:
    - Docker
    - CI/CD
experience:
  - title: "Senior ML Engineer"
    company: "TechCorp"
    start: "2021-01"
    end: "present"
    bullets:
      - "Built distributed training pipeline in Rust serving 10M requests/day"
      - "Reduced inference latency by 40% using model quantization"
      - "Led migration from Python to Rust for critical inference path"
education:
  - institution: "Stanford University"
    degree: "MS Computer Science"
    start: "2014"
    end: "2016"
projects:
  - name: "Edge Inference Engine"
    description: "ONNX runtime for embedded devices in Rust"
"#
    .to_string()
}

pub fn google_listing() -> MatchListing {
    MatchListing {
        source: "test".to_string(),
        external_id: "goog-001".to_string(),
        title: "Senior ML Infrastructure Engineer".to_string(),
        company: "Google".to_string(),
        location: Some("Mountain View, CA".to_string()),
        url: "https://careers.google.com/1".to_string(),
        description: "Build scalable ML infrastructure using Rust and Python. \
            Experience with distributed systems, Kubernetes, and PyTorch required."
            .to_string(),
        raw_json: None,
    }
}

pub fn anthropic_listing() -> MatchListing {
    MatchListing {
        source: "test".to_string(),
        external_id: "anthropic-001".to_string(),
        title: "ML Safety Research Engineer".to_string(),
        company: "Anthropic".to_string(),
        location: Some("San Francisco, CA".to_string()),
        url: "https://anthropic.com/careers/1".to_string(),
        description: "Research AI alignment and safety. Strong Rust and Python skills \
            needed. Experience with PyTorch and distributed training required."
            .to_string(),
        raw_json: None,
    }
}

pub fn mismatch_listing() -> MatchListing {
    MatchListing {
        source: "test".to_string(),
        external_id: "mismatch-001".to_string(),
        title: "Frontend React Developer".to_string(),
        company: "StartupCo".to_string(),
        location: None,
        url: "https://startup.com/1".to_string(),
        description: "Build React web applications with TypeScript and Tailwind CSS."
            .to_string(),
        raw_json: None,
    }
}
