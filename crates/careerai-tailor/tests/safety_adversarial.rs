//! Tabular adversarial tests. Each row: (description, raw_diff_json,
//! profile_override, expected-variant matcher). Every row except the last
//! MUST fail validation; the last row is the positive control and MUST
//! pass.

mod common;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::common::{fixture_profile, VALID_DIFF_JSON};
    use careerai_profile::schema::Profile;
    use careerai_tailor::error::TailorError;
    use careerai_tailor::schema::parse_and_validate;

    enum Expect {
        Ok,
        Schema(&'static str),
        BadPath,
        InventedContent(&'static str),
        CoverLetterTooLong,
    }

    struct Row {
        desc: &'static str,
        raw: String,
        profile: Profile,
        expect: Expect,
    }

    #[allow(clippy::too_many_lines)]
    fn rows() -> Vec<Row> {
        let p = fixture_profile;
        let v: Vec<Row> = vec![
            // 1 — invented employer in reword
            Row {
                desc: "invented employer Google",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"reword","new_text":"Shipped at Google Cloud Run."},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::InventedContent("invented proper noun"),
            },
            // 2 — fabricated percentage (97% absent from profile)
            Row {
                desc: "fabricated 97% number",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"reword","new_text":"improved latency 97%"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::InventedContent("invented number"),
            },
            // 3 — cross-entry move
            Row {
                desc: "cross-entry move",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"move_before","target_path":"experience[1].bullets[0]"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::Schema("crosses entries"),
            },
            // 4 — unicode homoglyph in proper noun ("Gооgle" with Cyrillic о's)
            //     lowercased token won't match employer set.
            Row {
                desc: "unicode homoglyph employer",
                raw: "{\"prompt_version\":\"tailor.v1\",\"summary\":{\"op\":\"keep\"},\"ops\":[\
                    {\"path\":\"experience[0].bullets[0]\",\"op\":\"reword\",\"new_text\":\"Shipped at G\u{043E}\u{043E}gle systems.\"},\
                    {\"path\":\"experience[0].bullets[1]\",\"op\":\"keep\"},\
                    {\"path\":\"experience[1].bullets[0]\",\"op\":\"keep\"},\
                    {\"path\":\"projects[0].bullets[0]\",\"op\":\"keep\"}],\
                    \"cover_letter\":\"ok\"}".into(),
                profile: p(),
                expect: Expect::InventedContent("invented proper noun"),
            },
            // 5 — extra top-level JSON key (deny_unknown_fields)
            Row {
                desc: "extra top-level field",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"keep"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok",
                    "side_channel":"owned"}"#.into(),
                profile: p(),
                expect: Expect::Schema("json parse"),
            },
            // 6 — duplicate op path
            Row {
                desc: "duplicate op path",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"keep"},
                    {"path":"experience[0].bullets[0]","op":"keep"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::Schema("duplicate"),
            },
            // 7 — coverage gap
            Row {
                desc: "coverage gap",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"keep"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::Schema("not covered"),
            },
            // 8 — drops all bullets in experience[1]
            Row {
                desc: "drop-all-in-experience",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"keep"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"drop"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::Schema("emptied by drops"),
            },
            // 9 — cover letter over 350 words
            Row {
                desc: "cover letter over 350 words",
                raw: format!(
                    r#"{{"prompt_version":"tailor.v1","summary":{{"op":"keep"}},"ops":[
                        {{"path":"experience[0].bullets[0]","op":"keep"}},
                        {{"path":"experience[0].bullets[1]","op":"keep"}},
                        {{"path":"experience[1].bullets[0]","op":"keep"}},
                        {{"path":"projects[0].bullets[0]","op":"keep"}}],
                        "cover_letter":"{}"}}"#,
                    (0..400).map(|_| "w").collect::<Vec<_>>().join(" ")
                ),
                profile: p(),
                expect: Expect::CoverLetterTooLong,
            },
            // 10 — reword > 280 chars
            Row {
                desc: "reword over 280 chars",
                raw: format!(
                    r#"{{"prompt_version":"tailor.v1","summary":{{"op":"keep"}},"ops":[
                        {{"path":"experience[0].bullets[0]","op":"reword","new_text":"{}"}},
                        {{"path":"experience[0].bullets[1]","op":"keep"}},
                        {{"path":"experience[1].bullets[0]","op":"keep"}},
                        {{"path":"projects[0].bullets[0]","op":"keep"}}],
                        "cover_letter":"ok"}}"#,
                    "a".repeat(300)
                ),
                profile: p(),
                expect: Expect::InventedContent("bullet over 280 chars"),
            },
            // 11 — fabricated year ("2025" not in profile's year set {2018, 2021, 2022})
            Row {
                desc: "fabricated year 2099",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"reword","new_text":"delivered in 2099"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::InventedContent("invented year"),
            },
            // 12 — move_before target non-existent path in same entry
            Row {
                desc: "move_before target missing",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"move_before","target_path":"experience[0].bullets[9]"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::Schema("move_before target missing"),
            },
            // 13 — reword missing new_text (should hit JSON parse => Schema)
            Row {
                desc: "reword missing new_text",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"experience[0].bullets[0]","op":"reword"},
                    {"path":"experience[0].bullets[1]","op":"keep"},
                    {"path":"experience[1].bullets[0]","op":"keep"},
                    {"path":"projects[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::Schema("json parse"),
            },
            // 14 — bad path shape (unknown section `education`)
            Row {
                desc: "bad path shape",
                raw: r#"{"prompt_version":"tailor.v1","summary":{"op":"keep"},"ops":[
                    {"path":"education[0].bullets[0]","op":"keep"}],
                    "cover_letter":"ok"}"#.into(),
                profile: p(),
                expect: Expect::BadPath,
            },
            // 15 — positive control (must pass)
            Row {
                desc: "positive control",
                raw: VALID_DIFF_JSON.into(),
                profile: p(),
                expect: Expect::Ok,
            },
        ];
        v
    }

    #[test]
    fn adversarial_table() {
        for row in rows() {
            let result = parse_and_validate(&row.raw, &row.profile, "");
            match (&row.expect, result) {
                (Expect::Ok, Ok(_)) => {}
                (Expect::Ok, Err(e)) => {
                    panic!("[{}] expected Ok, got {e:?}", row.desc);
                }
                (expect, Ok(_)) => {
                    panic!(
                        "[{}] expected rejection ({:?}), got Ok",
                        row.desc,
                        expect_tag(expect)
                    );
                }
                (Expect::Schema(needle), Err(e)) => {
                    let ok = matches!(&e, TailorError::Schema(s) if s.contains(needle));
                    assert!(
                        ok,
                        "[{}] expected Schema(contains {needle:?}), got {e:?}",
                        row.desc
                    );
                }
                (Expect::BadPath, Err(e)) => {
                    assert!(
                        matches!(e, TailorError::BadPath(_)),
                        "[{}] expected BadPath, got {e:?}",
                        row.desc
                    );
                }
                (Expect::InventedContent(needle), Err(e)) => {
                    let ok = matches!(&e, TailorError::InventedContent { reason, .. } if reason == needle);
                    assert!(
                        ok,
                        "[{}] expected InventedContent reason={needle:?}, got {e:?}",
                        row.desc
                    );
                }
                (Expect::CoverLetterTooLong, Err(e)) => {
                    assert!(
                        matches!(e, TailorError::CoverLetterTooLong { .. }),
                        "[{}] expected CoverLetterTooLong, got {e:?}",
                        row.desc
                    );
                }
            }
        }
    }

    fn expect_tag(e: &Expect) -> &'static str {
        match e {
            Expect::Ok => "Ok",
            Expect::Schema(_) => "Schema",
            Expect::BadPath => "BadPath",
            Expect::InventedContent(_) => "InventedContent",
            Expect::CoverLetterTooLong => "CoverLetterTooLong",
        }
    }
}
