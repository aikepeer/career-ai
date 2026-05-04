#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::validate::MAX_BULLET_CHARS;
use super::*;
use crate::TailorError;
use careerai_profile::schema::{Experience, Personal, Profile, Project, Skills};

mod validate_tests;

fn fixture_profile() -> Profile {
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
    }
}

fn minimal_doc(ops: Vec<DiffOp>) -> DiffDoc {
    DiffDoc {
        prompt_version: "tailor.v1".into(),
        summary: Some(SummaryOp::Keep),
        ops,
        cover_letter: "short letter".into(),
    }
}

fn keep(path: &str) -> DiffOp {
    DiffOp {
        path: path.into(),
        kind: OpKind::Keep,
    }
}

fn full_coverage_ops() -> Vec<DiffOp> {
    vec![
        keep("experience[0].bullets[0]"),
        keep("experience[0].bullets[1]"),
        keep("experience[1].bullets[0]"),
        keep("projects[0].bullets[0]"),
    ]
}

#[test]
fn rejects_path_outside_profile() {
    let profile = fixture_profile();
    // experience[0] only has 2 bullets — [2] is out of range.
    let mut ops = full_coverage_ops();
    ops.push(keep("experience[0].bullets[2]"));
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(matches!(err, TailorError::Schema(ref s) if s.contains("missing bullet")));
}

#[test]
fn rejects_invented_employer() {
    let profile = fixture_profile();
    let mut ops = full_coverage_ops();
    // Replace experience[0].bullets[0] with a reword mentioning Google.
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::Reword {
            new_text: "Shipped Rust pipeline at Google reducing latency 35%.".into(),
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented proper noun"),
        "got {err:?}"
    );
}

#[test]
fn rejects_fabricated_number() {
    let profile = fixture_profile();
    let mut ops = full_coverage_ops();
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::Reword {
            // 97% appears nowhere in the profile.
            new_text: "Shipped Rust pipeline reducing latency 97%.".into(),
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented number"),
        "got {err:?}"
    );
}

#[test]
fn rejects_cross_entry_move() {
    let profile = fixture_profile();
    let mut ops = full_coverage_ops();
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::MoveBefore {
            // Crosses into experience[1]
            target_path: "experience[1].bullets[0]".into(),
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("crosses entries")),
        "got {err:?}"
    );
}

#[test]
fn rejects_empty_experience_after_drops() {
    let profile = fixture_profile();
    // experience[1] has 1 bullet — dropping it empties the entry.
    let mut ops = full_coverage_ops();
    ops[2] = DiffOp {
        path: "experience[1].bullets[0]".into(),
        kind: OpKind::Drop,
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("emptied by drops")),
        "got {err:?}"
    );
}

#[test]
fn rejects_missing_coverage() {
    let profile = fixture_profile();
    // Omit experience[1].bullets[0].
    let ops = vec![
        keep("experience[0].bullets[0]"),
        keep("experience[0].bullets[1]"),
        keep("projects[0].bullets[0]"),
    ];
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("not covered")),
        "got {err:?}"
    );
}

#[test]
fn rejects_duplicate_op_paths() {
    let profile = fixture_profile();
    let mut ops = full_coverage_ops();
    ops.push(keep("experience[0].bullets[0]"));
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("duplicate op path")),
        "got {err:?}"
    );
}

#[test]
fn accepts_valid_reorder_and_keep() {
    let profile = fixture_profile();
    let ops = vec![
        DiffOp {
            path: "experience[0].bullets[1]".into(),
            kind: OpKind::MoveBefore {
                target_path: "experience[0].bullets[0]".into(),
            },
        },
        keep("experience[0].bullets[0]"),
        keep("experience[1].bullets[0]"),
        keep("projects[0].bullets[0]"),
    ];
    let doc = minimal_doc(ops);
    validate(&doc, &profile).unwrap();
    let view = apply(doc, profile).unwrap();
    // Moved bullet[1] to front of experience[0].
    assert_eq!(view.experience[0].bullets.len(), 2);
    assert!(view.experience[0].bullets[0].contains("Led team"));
}

#[test]
fn accepts_skill_injection_from_profile_skills() {
    let profile = fixture_profile();
    let mut ops = full_coverage_ops();
    // Reword the Python services bullet to mention Rust + Tokio (both
    // in profile.skills).
    ops[2] = DiffOp {
        path: "experience[1].bullets[0]".into(),
        kind: OpKind::Reword {
            new_text: "Built Rust Tokio services on Kubernetes.".into(),
        },
    };
    let doc = minimal_doc(ops);
    validate(&doc, &profile).unwrap();
}

#[test]
fn bullet_length_cap_enforced() {
    let profile = fixture_profile();

    // Under cap: 280 chars exactly (build a string of 280 'a's — but
    // this must still pass proper-noun + number guardrails; all
    // lowercase letters satisfy both.).
    let ok_text: String = "a".repeat(MAX_BULLET_CHARS);
    let mut ops = full_coverage_ops();
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::Reword { new_text: ok_text },
    };
    let doc = minimal_doc(ops);
    validate(&doc, &profile).unwrap();

    // Over cap: 281 chars.
    let long_text: String = "a".repeat(MAX_BULLET_CHARS + 1);
    let mut ops = full_coverage_ops();
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::Reword {
            new_text: long_text,
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { reason, .. } if reason == "bullet over 280 chars"),
        "got {err:?}"
    );
}

#[test]
fn rejects_empty_reword_new_text() {
    let profile = fixture_profile();
    let mut ops = full_coverage_ops();
    // Whitespace-only reword should also reject (trim().is_empty()).
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::Reword {
            new_text: "   \t\n  ".into(),
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("reword new_text is empty")),
        "got {err:?}"
    );
}

#[test]
fn rejects_move_before_target_equals_source() {
    let profile = fixture_profile();
    let mut ops = full_coverage_ops();
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::MoveBefore {
            target_path: "experience[0].bullets[0]".into(),
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("move_before target equals source")),
        "got {err:?}"
    );
}

#[test]
fn rejects_cover_letter_over_char_cap_even_with_low_word_count() {
    let profile = fixture_profile();
    // One long hyphenated "word" that split_whitespace counts as 1
    // but total chars vastly exceeds the cap — the char guard kicks.
    let huge: String = "x".repeat(4000);
    let doc = DiffDoc {
        prompt_version: "tailor.v1".into(),
        summary: Some(SummaryOp::Keep),
        ops: full_coverage_ops(),
        cover_letter: huge,
    };
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::CoverLetterTooLong { .. }),
        "got {err:?}"
    );
}

#[test]
fn bullet_path_parse_roundtrip() {
    for s in [
        "experience[0].bullets[0]",
        "experience[12].bullets[34]",
        "projects[1].bullets[2]",
    ] {
        let bp = BulletPath::parse(s).unwrap();
        assert_eq!(bp.format(), s);
    }
}

#[test]
fn bullet_path_parse_rejects_garbage() {
    for s in [
        "summary",
        "experience[0]",
        "experience[a].bullets[0]",
        "experience[0].bullets[a]",
        "education[0].bullets[0]",
    ] {
        let err = BulletPath::parse(s).unwrap_err();
        assert!(matches!(err, TailorError::BadPath(_)));
    }
}
