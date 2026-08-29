#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::error::TailorError;
use careerai_profile::schema::{Education, Experience, Personal, Profile, Project, Skills};

fn fixture() -> Profile {
    Profile {
        personal: Personal {
            name: "Ada".into(),
            ..Default::default()
        },
        summary: "Summary here".into(),
        skills: Skills {
            languages: vec!["Rust".into(), "Python".into(), "C#".into()],
            frameworks: vec!["Kubernetes".into(), "Tokio".into()],
            tools: vec![],
            ..Default::default()
        },
        experience: vec![Experience {
            title: "SWE".into(),
            company: "Acme Robotics".into(),
            location: String::new(),
            start: "2018-01".into(),
            end: "2023-06".into(),
            bullets: vec!["shipped 35% throughput win".into()],
        }],
        education: vec![Education {
            degree: "BS".into(),
            institution: "Waterloo".into(),
            start: "2014".into(),
            end: "2018".into(),
            ..Default::default()
        }],
        projects: vec![Project {
            name: "OpenLLM".into(),
            url: String::new(),
            bullets: vec!["tokenizer in Rust".into()],
        }],
        ..Default::default()
    }
}

#[test]
fn accepts_employer_from_profile() {
    let p = fixture();
    forbid_invented_entities("Shipped at Acme Robotics.", "", &p, "x").unwrap();
}

#[test]
fn accepts_common_english_adjective_lead() {
    // Sentence-start resume adjectives ("Experienced", "Skilled", ...)
    // are normal English, not invented proper nouns. Regression:
    // agy-backed tailoring of the live profile failed here — the model
    // reworded the summary to lead with "Experienced in embedded
    // Linux..." and the guardrail rejected the word as an invented
    // proper noun.
    let p = fixture();
    for lead in ["Experienced", "Skilled", "Proficient", "Seasoned"] {
        forbid_invented_entities(
            &format!("{lead} in building resilient cloud infrastructure."),
            "",
            &p,
            "x",
        )
        .unwrap_or_else(|e| {
            panic!("sentence-start {lead} must not be an invented proper noun: {e:?}")
        });
    }
}

#[test]
fn accepts_sentence_start_capitalization_of_bullet_word() {
    // A reword may capitalize a word that appears (lowercase) in the
    // original bullet — "curated" → "Curated" at sentence start.
    // Regression: live tailoring rejected a faithful reword because the
    // original-bullet carve-out only covered already-Capitalized runs.
    let p = fixture();
    forbid_invented_entities(
        "Curated signed Yocto images for TI AM665x platforms.",
        "built HMI tooling; curated signed Yocto images for TI AM665x",
        &p,
        "experience[1].bullets[0]",
    )
    .unwrap();
}

#[test]
fn still_rejects_invented_noun_even_when_bullet_word_capitalized() {
    // Control for `accepts_sentence_start_capitalization_of_bullet_word`:
    // capitalizing a word from the bullet is fine, but a genuinely new
    // Capitalized noun must still be rejected.
    let p = fixture();
    let err = forbid_invented_entities(
        "Curated signed Yocto images for AcmeCorp cloud.",
        "built HMI tooling; curated signed Yocto images for TI AM665x",
        &p,
        "experience[1].bullets[0]",
    )
    .unwrap_err();
    match err {
        TailorError::InventedContent {
            offending_token, ..
        } => assert_eq!(offending_token, "AcmeCorp"),
        other => panic!("expected InventedContent, got {other:?}"),
    }
}

#[test]
fn rejects_employer_not_in_profile() {
    let p = fixture();
    let err = forbid_invented_entities("Shipped at Google.", "", &p, "x").unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented proper noun"),
        "got {err:?}"
    );
}

#[test]
fn rejects_fabricated_number_above_profile_max() {
    let p = fixture();
    let err = forbid_invented_entities("increased throughput 40%", "", &p, "x").unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented number"),
        "got {err:?}"
    );
}

#[test]
fn accepts_number_from_original_bullet() {
    let p = fixture();
    forbid_invented_entities("drove 35% latency cut", "35% baseline", &p, "x").unwrap();
}

#[test]
fn accepts_skills_injection() {
    let p = fixture();
    forbid_invented_entities("shipped Rust and Kubernetes services", "", &p, "x").unwrap();
}

#[test]
fn accepts_csharp_skill_with_hash_kept_intact() {
    let p = fixture();
    forbid_invented_entities("built systems in C# and Rust", "", &p, "x").unwrap();
}

#[test]
fn accepts_bare_number_when_profile_has_percent_form() {
    let p = fixture();
    forbid_invented_entities("managed 35 engineers", "35% baseline", &p, "x").unwrap();
}

#[test]
fn rejects_fabricated_year() {
    let p = fixture();
    let err = forbid_invented_entities("hired in 2025", "hired in 2020", &p, "x").unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented year"),
        "got {err:?}"
    );
}

#[test]
fn rejects_invented_location_proper_noun() {
    let p = fixture();
    let err = forbid_invented_entities("worked in Paris", "", &p, "x").unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented proper noun"),
        "got {err:?}"
    );
}

#[test]
fn accepts_proper_noun_from_original_bullet() {
    let p = fixture();
    forbid_invented_entities(
        "Owned the Symbot6 platform from boot to telemetry.",
        "Architected the Symbot6 platform on Linux.",
        &p,
        "x",
    )
    .unwrap();
}

#[test]
fn accepts_common_resume_action_verbs_at_sentence_start() {
    let p = fixture();
    for new_text in [
        "Architected the system end-to-end.",
        "Engineered low-latency pipelines.",
        "Optimized boot time across targets.",
        "Migrated services to a new platform.",
        "Developed core middleware.",
        "Refactored the storage layer.",
        "Scaled the cluster horizontally.",
        "Automated the release process.",
        "Integrated the legacy stack.",
        "Established service-level objectives.",
        "Achieved deterministic throughput.",
        "Reduced cold-start latency.",
        "Increased fleet uptime.",
        "Coordinated cross-team launches.",
        "Mentored a team of engineers.",
        "Spearheaded the redesign effort.",
        "Defined the service contract.",
        "Authored the kernel module.",
        "Modernized the build system.",
        "Hardened the boot sequence.",
    ] {
        forbid_invented_entities(new_text, "noop bullet text", &p, "x").unwrap_or_else(|e| {
            panic!("guardrail rejected resume-action verb in {new_text:?}: {e:?}")
        });
    }
}

#[test]
fn accepts_proper_noun_from_same_bullet_original() {
    let p = Profile {
        personal: Personal {
            name: "Test".into(),
            email: "t@example.com".into(),
            ..Default::default()
        },
        summary: "Embedded engineer.".into(),
        skills: Skills {
            languages: vec!["C".into()],
            frameworks: vec![],
            tools: vec![],
            ..Default::default()
        },
        experience: vec![Experience {
            title: "Engineer".into(),
            company: "Acme".into(),
            location: String::new(),
            start: "2020".into(),
            end: "2023".into(),
            bullets: vec!["Ported display servers from X11 to Wayland on embedded boards.".into()],
        }],
        education: vec![],
        projects: vec![],
        ..Default::default()
    };

    forbid_invented_entities(
        "Migrated the Wayland compositor for the new platform.",
        "Ported display servers from X11 to Wayland on embedded boards.",
        &p,
        "x",
    )
    .unwrap();
}

#[test]
fn rejects_proper_noun_only_in_other_bullet() {
    let p = Profile {
        personal: Personal {
            name: "Test".into(),
            email: "t@example.com".into(),
            ..Default::default()
        },
        summary: "Embedded engineer.".into(),
        skills: Skills {
            languages: vec!["C".into()],
            frameworks: vec![],
            tools: vec![],
            ..Default::default()
        },
        experience: vec![Experience {
            title: "Engineer".into(),
            company: "Acme".into(),
            location: String::new(),
            start: "2020".into(),
            end: "2023".into(),
            bullets: vec!["Ported display servers from X11 to Wayland on embedded boards.".into()],
        }],
        education: vec![],
        projects: vec![],
        ..Default::default()
    };

    let err = forbid_invented_entities(
        "Stabilized Wayland compositor performance.",
        "different bullet original here",
        &p,
        "x",
    )
    .expect_err("expected rejection of cross-bullet proper noun");
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("wayland") || msg.to_lowercase().contains("invented"),
        "expected error to mention the leaked token; got: {msg}"
    );
}

#[test]
fn accepts_proper_noun_from_summary() {
    let p = Profile {
        personal: Personal {
            name: "Test".into(),
            email: "t@example.com".into(),
            ..Default::default()
        },
        summary: "Embedded engineer focused on Wayland and Mesa stacks.".into(),
        skills: Skills {
            languages: vec!["C".into()],
            frameworks: vec![],
            tools: vec![],
            ..Default::default()
        },
        experience: vec![Experience {
            title: "Engineer".into(),
            company: "Acme".into(),
            location: String::new(),
            start: "2020".into(),
            end: "2023".into(),
            bullets: vec!["Built display servers.".into()],
        }],
        education: vec![],
        projects: vec![],
        ..Default::default()
    };

    forbid_invented_entities(
        "Stabilized Wayland compositor performance.",
        "Built display servers.",
        &p,
        "x",
    )
    .unwrap();
}

#[test]
fn accepts_punctuated_employer_name_from_company_field() {
    let p = Profile {
        personal: Personal {
            name: "Test".into(),
            email: "t@example.com".into(),
            ..Default::default()
        },
        summary: "Engineer.".into(),
        skills: Skills {
            languages: vec![],
            frameworks: vec![],
            tools: vec![],
            ..Default::default()
        },
        experience: vec![Experience {
            title: "Engineer".into(),
            company: "AT&T".into(),
            location: String::new(),
            start: "2018".into(),
            end: "2020".into(),
            bullets: vec!["Worked on telecom infrastructure.".into()],
        }],
        education: vec![],
        projects: vec![],
        ..Default::default()
    };

    forbid_invented_entities(
        "Migrated AT&T billing systems to a new platform.",
        "Worked on telecom infrastructure.",
        &p,
        "x",
    )
    .unwrap();
}

#[test]
fn accepts_common_sentence_start_adverbs() {
    let p = fixture();
    for new_text in [
        "Currently architecting the next-gen platform.",
        "Previously shipped the legacy stack.",
        "Recently optimized cold-start latency.",
        "Successfully delivered five major releases.",
        "Additionally drove cross-team alignment.",
        "However the team pivoted late.",
        "Therefore reduced scope to ship on time.",
        "Specifically tuned the boot sequence.",
        "Initially established the testing framework.",
        "Eventually owned the deployment pipeline.",
    ] {
        forbid_invented_entities(new_text, "noop bullet text", &p, "x").unwrap_or_else(|e| {
            panic!("guardrail rejected sentence-start adverb in {new_text:?}: {e:?}")
        });
    }
}

#[test]
fn rejects_invented_token_riding_along_in_whitelisted_span() {
    let p = fixture();
    let err = forbid_invented_entities(
        "Architected Google Ads platform.",
        "Architected the platform end-to-end.",
        &p,
        "x",
    )
    .unwrap_err();
    assert!(
        matches!(&err, TailorError::InventedContent { reason, offending_token, .. }
            if *reason == "invented proper noun"
            && (offending_token.contains("Google") || offending_token.contains("Ads"))),
        "expected invented-noun rejection naming Google/Ads; got {err:?}"
    );
}

#[test]
fn still_rejects_proper_noun_absent_from_both_profile_and_original() {
    let p = fixture();
    let err = forbid_invented_entities(
        "Owned the Symbot7 platform end-to-end.",
        "Architected the Symbot6 platform on Linux.",
        &p,
        "x",
    )
    .unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented proper noun"),
        "got {err:?}"
    );
}

#[test]
fn accepts_jd_proper_noun_via_jd_token_sets() {
    // Regression: the "tailor top 20" dashboard button failed 100% of
    // runs because reworded bullets used JD terminology (e.g. "Fleet"
    // from "fleet management") that was not in the profile. The fix
    // adds JD-derived tokens to the guardrail allowlist.
    let p = fixture();
    let sets = build_token_sets(&p);
    let jd_text = "Senior Software Engineer, Android Automotive at Waymo. \
                   Build fleet management systems for autonomous vehicles.";
    let mut sets = sets;
    sets.jd_proper_nouns = build_jd_token_sets(jd_text);
    forbid_invented_entities_with(
        "Fleet Management: Architected fleet telemetry for Waymo vehicles.",
        "shipped 35% throughput win",
        &sets,
        "x",
    )
    .unwrap();
}

#[test]
fn still_rejects_invented_noun_not_in_jd_or_profile() {
    let p = fixture();
    let sets = build_token_sets(&p);
    let jd_text = "Senior Software Engineer at Waymo. Build fleet systems.";
    let mut sets = sets;
    sets.jd_proper_nouns = build_jd_token_sets(jd_text);
    let err = forbid_invented_entities_with(
        "Built the Google Cloud platform.",
        "shipped 35% throughput win",
        &sets,
        "x",
    )
    .unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, offending_token, .. }
            if reason == "invented proper noun"
            && (offending_token.contains("Google") || offending_token.contains("Cloud"))),
        "got {err:?}"
    );
}
