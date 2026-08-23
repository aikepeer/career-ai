//! Parse + validate the LLM's raw JSON response.
//!
//! Tolerates two forms LLMs emit in practice: bare JSON, or a JSON object
//! wrapped in a single ```json ... ``` (or bare ``` ... ```) markdown
//! fence. Anything else surfaces as `TailorError::Schema` so the caller
//! can trigger one retry with a stricter suffix.

use careerai_profile::schema::Profile;

use crate::diff::{self, DiffDoc};
use crate::error::{Result, TailorError};

/// Parse raw LLM output and run all 9 validator rules against the
/// profile. On success the returned `DiffDoc` is safe to hand to
/// `diff::apply`.
pub fn parse_and_validate(raw: &str, profile: &Profile) -> Result<DiffDoc> {
    let stripped = strip_json_fences(raw.trim());
    let doc: DiffDoc = serde_json::from_str(stripped)
        .map_err(|e| TailorError::Schema(format!("json parse: {e}; raw_len={}", raw.len())))?;
    diff::validate(&doc, profile)?;
    Ok(doc)
}

/// Strip a single leading ```` ```json ```` or ```` ``` ```` fence plus the
/// matching trailing fence. Tolerates optional whitespace / newlines. When
/// no fence is present the input is returned unchanged.
fn strip_json_fences(input: &str) -> &str {
    let trimmed = input.trim();
    // Leading fence
    let (body, has_leading) = if let Some(rest) = strip_leading_fence(trimmed) {
        (rest, true)
    } else {
        (trimmed, false)
    };
    if !has_leading {
        return trimmed;
    }
    // Trailing fence
    strip_trailing_fence(body).unwrap_or(body)
}

fn strip_leading_fence(s: &str) -> Option<&str> {
    // At least 3 backticks.
    let bytes = s.as_bytes();
    if bytes.len() < 3 || !bytes.iter().take(3).all(|b| *b == b'`') {
        return None;
    }
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b'`' {
        i += 1;
    }
    // Optional language tag (word chars) up to newline or next space.
    let after_ticks = &s[i..];
    let after_lang = after_ticks
        .strip_prefix("json")
        .or_else(|| after_ticks.strip_prefix("JSON"))
        .unwrap_or(after_ticks);
    // Expect whitespace/newline before content.
    Some(after_lang.trim_start())
}

fn strip_trailing_fence(s: &str) -> Option<&str> {
    let trimmed = s.trim_end();
    // Count trailing backticks.
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let mut end = bytes.len();
    let mut tick_count = 0;
    while end > 0 && bytes[end - 1] == b'`' {
        end -= 1;
        tick_count += 1;
    }
    if tick_count < 3 {
        return None;
    }
    Some(trimmed[..end].trim_end())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::{Experience, Personal, Profile, Project, Skills};

    fn fixture_profile() -> Profile {
        Profile {
            personal: Personal {
                name: "A".into(),
                ..Default::default()
            },
            summary: "s".into(),
            skills: Skills::default(),
            experience: vec![Experience {
                title: "T".into(),
                company: "Acme".into(),
                location: String::new(),
                start: "2020".into(),
                end: "2023".into(),
                bullets: vec!["b0".into()],
            }],
            education: vec![],
            projects: vec![Project {
                name: "P".into(),
                url: String::new(),
                bullets: vec!["pb0".into()],
            }],
            ..Default::default()
        }
    }

    const VALID_JSON: &str = r#"{
        "prompt_version":"tailor.v1",
        "summary":{"op":"keep"},
        "ops":[
            {"path":"experience[0].bullets[0]","op":"keep"},
            {"path":"projects[0].bullets[0]","op":"keep"}
        ],
        "cover_letter":"short"
    }"#;

    #[test]
    fn accepts_bare_json() {
        parse_and_validate(VALID_JSON, &fixture_profile()).unwrap();
    }

    #[test]
    fn accepts_json_fence_wrapped() {
        let wrapped = format!("```json\n{VALID_JSON}\n```");
        parse_and_validate(&wrapped, &fixture_profile()).unwrap();
    }

    #[test]
    fn accepts_bare_fence_wrapped() {
        let wrapped = format!("```\n{VALID_JSON}\n```");
        parse_and_validate(&wrapped, &fixture_profile()).unwrap();
    }

    #[test]
    fn accepts_leading_trailing_whitespace() {
        let wrapped = format!("   \n{VALID_JSON}\n   ");
        parse_and_validate(&wrapped, &fixture_profile()).unwrap();
    }

    #[test]
    fn rejects_invalid_json_surfaces_schema() {
        let bad = r"{not json";
        let err = parse_and_validate(bad, &fixture_profile()).unwrap_err();
        assert!(
            matches!(err, TailorError::Schema(ref s) if s.contains("json parse")),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let raw = r#"{
            "prompt_version":"tailor.v1",
            "summary":{"op":"keep"},
            "ops":[{"path":"experience[0].bullets[0]","op":"keep"},
                   {"path":"projects[0].bullets[0]","op":"keep"}],
            "cover_letter":"short",
            "extra_key":"nope"
        }"#;
        let err = parse_and_validate(raw, &fixture_profile()).unwrap_err();
        assert!(matches!(err, TailorError::Schema(_)), "got {err:?}");
    }
}
