#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::diff::SummaryOp;
use crate::TailorError;

/// Rule 8 — summary reword rejects an invented proper noun (Google
/// is not in the fixture profile).
#[test]
fn rejects_summary_reword_invented_employer() {
    let profile = super::fixture_profile();
    let doc = DiffDoc {
        prompt_version: "tailor.v1".into(),
        summary: Some(SummaryOp::Reword {
            new_text: "Engineer at Google.".into(),
        }),
        ops: super::full_coverage_ops(),
        cover_letter: "short".into(),
    };
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::InventedContent { .. }),
        "got {err:?}"
    );
}

/// Rule 8 — empty summary reword is rejected.
#[test]
fn rejects_summary_reword_empty() {
    let profile = super::fixture_profile();
    let doc = DiffDoc {
        prompt_version: "tailor.v1".into(),
        summary: Some(SummaryOp::Reword {
            new_text: "  ".into(),
        }),
        ops: super::full_coverage_ops(),
        cover_letter: "short".into(),
    };
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("summary reword new_text is empty")),
        "got {err:?}"
    );
}

/// Rule 9 — cover letter over 350 words is rejected even if under
/// the 3500-char cap.
#[test]
fn rejects_cover_letter_over_word_cap() {
    let profile = super::fixture_profile();
    // 351 words × 4 chars each = 1404 chars (well under 3500-char cap).
    let words: String = std::iter::repeat("word")
        .take(351)
        .collect::<Vec<_>>()
        .join(" ");
    let doc = DiffDoc {
        prompt_version: "tailor.v1".into(),
        summary: Some(SummaryOp::Keep),
        ops: super::full_coverage_ops(),
        cover_letter: words,
    };
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::CoverLetterTooLong { .. }),
        "got {err:?}"
    );
}

/// Rule 2 — MoveBefore with a garbage target_path fails at path
/// parse time (not silently ignored).
#[test]
fn rejects_move_before_unparseable_target() {
    let profile = super::fixture_profile();
    let mut ops = super::full_coverage_ops();
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::MoveBefore {
            target_path: "garbage".into(),
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(matches!(err, TailorError::BadPath(_)), "got {err:?}");
}

/// Rule 3 — path references a missing entry (entry_index out of
/// range), not just a missing bullet.
#[test]
fn rejects_path_missing_entry() {
    let profile = super::fixture_profile();
    // Profile has only 2 experiences — [5] is out of range.
    let mut ops = super::full_coverage_ops();
    ops.push(keep("experience[5].bullets[0]"));
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("missing entry")),
        "got {err:?}"
    );
}

/// Rule 3 — MoveBefore target references a missing bullet, not just
/// a missing entry.
#[test]
fn rejects_move_before_target_missing_bullet() {
    let profile = super::fixture_profile();
    let mut ops = super::full_coverage_ops();
    ops[0] = DiffOp {
        path: "experience[0].bullets[0]".into(),
        kind: OpKind::MoveBefore {
            // experience[1] has only 1 bullet — [5] doesn't exist.
            target_path: "experience[1].bullets[5]".into(),
        },
    };
    let doc = minimal_doc(ops);
    let err = validate(&doc, &profile).unwrap_err();
    assert!(
        matches!(err, TailorError::Schema(ref s) if s.contains("missing bullet")),
        "got {err:?}"
    );
}
