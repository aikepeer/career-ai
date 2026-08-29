//! End-to-end exercises of `schema::parse_and_validate` — raw JSON string
//! through tolerant fence strip through 9-rule validator.
//!
//! Tests are intentionally noisy with `unwrap`/`expect` since this is the
//! tests module; clippy allowlist follows the convention at
//! `crates/careerai-sources/src/remoteok.rs:110`.

mod common;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::common::{fixture_profile, VALID_DIFF_JSON};
    use careerai_tailor::error::TailorError;
    use careerai_tailor::schema::parse_and_validate;

    #[test]
    fn rule1_coverage_positive_control_passes() {
        parse_and_validate(VALID_DIFF_JSON, &fixture_profile(), "").unwrap();
    }

    #[test]
    fn rule1_missing_coverage_rejected() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[
                {"path":"experience[0].bullets[0]","op":"keep"}
            ],
            "cover_letter":"short"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("not covered")),
            "got {err:?}"
        );
    }

    #[test]
    fn rule1_duplicate_op_path_rejected() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[
                {"path":"experience[0].bullets[0]","op":"keep"},
                {"path":"experience[0].bullets[0]","op":"keep"},
                {"path":"experience[0].bullets[1]","op":"keep"},
                {"path":"experience[1].bullets[0]","op":"keep"},
                {"path":"projects[0].bullets[0]","op":"keep"}
            ],
            "cover_letter":"short"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("duplicate")),
            "got {err:?}"
        );
    }

    #[test]
    fn rule2_bad_path_rejected() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[
                {"path":"education[0].bullets[0]","op":"keep"}
            ],
            "cover_letter":"short"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile(), "").unwrap_err();
        assert!(matches!(err, TailorError::BadPath(_)), "got {err:?}");
    }

    #[test]
    fn rule3_out_of_range_bullet_index_rejected() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[
                {"path":"experience[0].bullets[0]","op":"keep"},
                {"path":"experience[0].bullets[1]","op":"keep"},
                {"path":"experience[0].bullets[7]","op":"keep"},
                {"path":"experience[1].bullets[0]","op":"keep"},
                {"path":"projects[0].bullets[0]","op":"keep"}
            ],
            "cover_letter":"short"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("missing bullet")),
            "got {err:?}"
        );
    }

    #[test]
    fn rule4_cross_entry_move_rejected() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[
                {"path":"experience[0].bullets[0]","op":"move_before","target_path":"experience[1].bullets[0]"},
                {"path":"experience[0].bullets[1]","op":"keep"},
                {"path":"experience[1].bullets[0]","op":"keep"},
                {"path":"projects[0].bullets[0]","op":"keep"}
            ],
            "cover_letter":"short"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("crosses entries")),
            "got {err:?}"
        );
    }

    #[test]
    fn rule5_drop_empties_experience_rejected() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[
                {"path":"experience[0].bullets[0]","op":"keep"},
                {"path":"experience[0].bullets[1]","op":"keep"},
                {"path":"experience[1].bullets[0]","op":"drop"},
                {"path":"projects[0].bullets[0]","op":"keep"}
            ],
            "cover_letter":"short"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("emptied by drops")),
            "got {err:?}"
        );
    }

    #[test]
    fn rule6_reword_over_length_cap_rejected() {
        let long: String = "a".repeat(281);
        let raw = format!(
            r#"{{
                "prompt_version":"tailor.v1",
                "summary":{{"op":"keep"}},
                "ops":[
                    {{"path":"experience[0].bullets[0]","op":"reword","new_text":"{long}"}},
                    {{"path":"experience[0].bullets[1]","op":"keep"}},
                    {{"path":"experience[1].bullets[0]","op":"keep"}},
                    {{"path":"projects[0].bullets[0]","op":"keep"}}
                ],
                "cover_letter":"short"
            }}"#
        );
        let err = parse_and_validate(&raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::InventedContent { reason, .. } if reason == "bullet over 280 chars"),
            "got {err:?}"
        );
    }

    #[test]
    fn rule7_invented_employer_rejected() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[
                {"path":"experience[0].bullets[0]","op":"reword","new_text":"Shipped at Google improving perf."},
                {"path":"experience[0].bullets[1]","op":"keep"},
                {"path":"experience[1].bullets[0]","op":"keep"},
                {"path":"projects[0].bullets[0]","op":"keep"}
            ],
            "cover_letter":"short"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::InventedContent { reason, .. } if reason == "invented proper noun"),
            "got {err:?}"
        );
    }

    #[test]
    fn rule9_cover_letter_over_cap_rejected() {
        let long: String = (0..400).map(|_| "w").collect::<Vec<_>>().join(" ");
        let raw = format!(
            r#"{{
                "prompt_version":"tailor.v1",
                "summary":{{"op":"keep"}},
                "ops":[
                    {{"path":"experience[0].bullets[0]","op":"keep"}},
                    {{"path":"experience[0].bullets[1]","op":"keep"}},
                    {{"path":"experience[1].bullets[0]","op":"keep"}},
                    {{"path":"projects[0].bullets[0]","op":"keep"}}
                ],
                "cover_letter":"{long}"
            }}"#
        );
        let err = parse_and_validate(&raw, &fixture_profile(), "").unwrap_err();
        assert!(
            matches!(err, TailorError::CoverLetterTooLong { words, cap } if words == 400 && cap == 350),
            "got {err:?}"
        );
    }

    #[test]
    fn malformed_json_surfaces_schema() {
        let err = parse_and_validate("{not json}", &fixture_profile(), "").unwrap_err();
        assert!(matches!(err, TailorError::Schema(_)), "got {err:?}");
    }

    #[test]
    fn fence_wrapped_json_accepted() {
        let wrapped = format!("```json\n{VALID_DIFF_JSON}\n```");
        parse_and_validate(&wrapped, &fixture_profile(), "").unwrap();
    }
}
