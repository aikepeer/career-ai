//! Template-based cover letter skeleton engine (Phase 3 of LLM Reduction).
//!
//! Pre-computes and matches domain-specific cover letter skeletons
//! (e.g. Embedded Systems, Robotics/AI, High-Throughput Backend, Fullstack).
//! Automatically extracts keywords and fills slot markers ({{company}}, {{title}},
//! {{top_skills}}, {{applicant_name}}) in <1ms without LLM dependencies.

use careerai_db::models::Listing;
use careerai_match::bullet_score::BulletScorer;
use careerai_profile::schema::Profile;
use serde::{Deserialize, Serialize};

/// Pre-computed cover letter skeleton for a specific technical domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverSkeleton {
    pub domain: String,
    pub keywords: Vec<String>,
    pub template: String,
}

/// Slot parameters to fill into a skeleton template.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverSlots {
    pub company: String,
    pub title: String,
    pub applicant_name: String,
    pub top_skills: String,
    pub keywords_summary: String,
}

/// Built-in curated domain skeletons.
#[must_use]
pub fn default_skeletons() -> Vec<CoverSkeleton> {
    vec![
        CoverSkeleton {
            domain: "Embedded & Robotics".into(),
            keywords: vec![
                "embedded".into(),
                "firmware".into(),
                "linux".into(),
                "rtos".into(),
                "c++".into(),
                "rust".into(),
                "yocto".into(),
                "robotics".into(),
                "ros2".into(),
                "bsp".into(),
            ],
            template: "Dear Hiring Team at {{company}},\n\nI am excited to apply for the {{title}} position. With extensive hands-on experience in {{top_skills}} and low-level systems engineering, I have developed robust firmware and optimized real-time components at scale.\n\nMy background aligns closely with your focus on {{keywords_summary}}. I welcome the opportunity to discuss how my technical expertise can contribute to your engineering milestones.\n\nBest regards,\n{{applicant_name}}".into(),
        },
        CoverSkeleton {
            domain: "Backend & Distributed Systems".into(),
            keywords: vec![
                "backend".into(),
                "distributed".into(),
                "microservices".into(),
                "database".into(),
                "grpc".into(),
                "tokio".into(),
                "throughput".into(),
                "latency".into(),
                "cloud".into(),
                "scale".into(),
            ],
            template: "Dear Hiring Team at {{company}},\n\nI am writing to express my strong enthusiasm for the {{title}} role. As a software engineer specializing in {{top_skills}}, I focus on designing high-throughput, fault-tolerant backend architectures.\n\nGiven your team's emphasis on {{keywords_summary}}, I am confident in my ability to build scalable services that meet your performance requirements.\n\nThank you for your time and consideration.\n\nSincerely,\n{{applicant_name}}".into(),
        },
        CoverSkeleton {
            domain: "AI & Machine Learning".into(),
            keywords: vec![
                "ai".into(),
                "ml".into(),
                "llm".into(),
                "vision".into(),
                "inference".into(),
                "models".into(),
                "pipeline".into(),
                "pytorch".into(),
            ],
            template: "Dear Hiring Team at {{company}},\n\nI am thrilled to apply for the {{title}} role at {{company}}. Combining a deep background in {{top_skills}} with practical model optimization, I build intelligent systems that bridge research and production reliability.\n\nI look forward to discussing how my experience with {{keywords_summary}} can support your mission.\n\nBest regards,\n{{applicant_name}}".into(),
        },
        CoverSkeleton {
            domain: "General Software Engineering".into(),
            keywords: vec!["software".into(), "engineer".into(), "developer".into()],
            template: "Dear Hiring Team at {{company}},\n\nI am writing to submit my application for the {{title}} position at {{company}}. With proven proficiency in {{top_skills}}, I bring a solid track record of delivering clean, well-tested code.\n\nI would love the chance to discuss how my skills align with your team's goals.\n\nBest regards,\n{{applicant_name}}".into(),
        },
    ]
}

/// Extract slot variables from a candidate profile and target listing.
#[must_use]
pub fn extract_slots(profile: &Profile, listing: &Listing) -> CoverSlots {
    let top_skills = if profile.skills.languages.is_empty() {
        "software engineering".to_string()
    } else {
        profile
            .skills
            .languages
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };

    let keywords_summary = format!("{} and systems architecture", listing.title);

    CoverSlots {
        company: listing.company.clone(),
        title: listing.title.clone(),
        applicant_name: profile.personal.name.clone(),
        top_skills,
        keywords_summary,
    }
}

/// Slot-fill a skeleton template with dynamic variables.
#[must_use]
pub fn fill_skeleton(skeleton: &CoverSkeleton, slots: &CoverSlots) -> String {
    skeleton
        .template
        .replace("{{company}}", &slots.company)
        .replace("{{title}}", &slots.title)
        .replace("{{applicant_name}}", &slots.applicant_name)
        .replace("{{top_skills}}", &slots.top_skills)
        .replace("{{keywords_summary}}", &slots.keywords_summary)
}

/// Match a job description to the best domain skeleton based on keyword overlap.
#[must_use]
pub fn pick_best_skeleton<'a>(
    skeletons: &'a [CoverSkeleton],
    jd_text: &str,
    scorer: &dyn BulletScorer,
) -> &'a CoverSkeleton {
    skeletons
        .iter()
        .map(|s| {
            let kw_text = s.keywords.join(" ");
            let score = scorer.score(&kw_text, jd_text);
            (s, score)
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(s, _)| s)
        .unwrap_or(&skeletons[0])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_match::bullet_score::JaccardBulletScorer;
    use careerai_profile::schema::{Personal, Skills};

    fn sample_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "Bob Builder".into(),
                ..Default::default()
            },
            skills: Skills {
                languages: vec!["Rust".into(), "C++".into(), "Python".into()],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn sample_listing(title: &str, desc: &str) -> Listing {
        Listing {
            id: "list-1".into(),
            source: "greenhouse".into(),
            external_id: "ext-1".into(),
            title: title.into(),
            company: "RoboCorp".into(),
            location: Some("Remote".into()),
            url: "https://example.com/job".into(),
            description: desc.into(),
            raw_json: None,
            state: "shortlisted".into(),
            score: Some(0.9),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn matches_embedded_skeleton_for_firmware_jd() {
        let skeletons = default_skeletons();
        let scorer = JaccardBulletScorer;
        let jd = "Senior Firmware Engineer: embedded Linux, Yocto, RTOS, and board bringup.";

        let best = pick_best_skeleton(&skeletons, jd, &scorer);
        assert_eq!(best.domain, "Embedded & Robotics");
    }

    #[test]
    fn matches_backend_skeleton_for_distributed_jd() {
        let skeletons = default_skeletons();
        let scorer = JaccardBulletScorer;
        let jd = "Distributed Systems Engineer: high throughput microservices, Tokio, and database scaling.";

        let best = pick_best_skeleton(&skeletons, jd, &scorer);
        assert_eq!(best.domain, "Backend & Distributed Systems");
    }

    #[test]
    fn fills_slots_accurately() {
        let p = sample_profile();
        let l = sample_listing("Embedded Linux Engineer", "Rust and C++ firmware");
        let skeletons = default_skeletons();
        let slots = extract_slots(&p, &l);

        let letter = fill_skeleton(&skeletons[0], &slots);
        assert!(letter.contains("RoboCorp"));
        assert!(letter.contains("Embedded Linux Engineer"));
        assert!(letter.contains("Bob Builder"));
        assert!(letter.contains("Rust, C++, Python"));
    }
}
