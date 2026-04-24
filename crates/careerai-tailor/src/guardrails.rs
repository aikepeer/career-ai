//! Entity-level reword guardrails.
//!
//! The safety invariant: LLM rewords may not invent employers, numbers,
//! years, or proper nouns that don't appear in the profile (or,
//! narrowly, in the original bullet). Violations surface as
//! `TailorError::InventedContent` with the exact offending token so logs
//! and tests can show users what fired and why.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use careerai_profile::schema::Profile;

use crate::error::{Result, TailorError};

fn number_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::unwrap_used)]
    RE.get_or_init(|| Regex::new(r"\b\d+(?:\.\d+)?%?\b").unwrap())
}

fn year_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::unwrap_used)]
    RE.get_or_init(|| Regex::new(r"\b(?:19|20)\d{2}\b").unwrap())
}

/// Extract consecutive Capitalized-token runs. Tokens may include a handful
/// of connector characters common in tech names (`+`, `&`, `.`). We match
/// on unicode categories so tokens containing homoglyphs (Cyrillic `о` in
/// "Gооgle", etc.) still register as proper nouns — which is desirable
/// because they will NOT lowercase-equal any profile token and will be
/// rejected by the intersection check below.
fn proper_noun_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::unwrap_used)]
    RE.get_or_init(|| {
        // `\p{Lu}` = any uppercase letter (ASCII or not).
        // `[\p{L}\p{N}+&.]` = letters (any script), digits, or connectors.
        Regex::new(r"\p{Lu}[\p{L}\p{N}+&.]*(?:\s+\p{Lu}[\p{L}\p{N}+&.]*)*").unwrap()
    })
}

#[allow(clippy::struct_field_names)]
struct ProfileTokenSets {
    employer_tokens: HashSet<String>,
    project_tokens: HashSet<String>,
    skill_tokens: HashSet<String>,
    year_tokens: HashSet<String>,
    number_tokens: HashSet<String>,
}

fn split_words_lowercase(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_lowercase)
}

fn build_token_sets(profile: &Profile) -> ProfileTokenSets {
    let mut employer_tokens = HashSet::new();
    for exp in &profile.experience {
        for w in split_words_lowercase(&exp.company) {
            employer_tokens.insert(w);
        }
    }
    for ed in &profile.education {
        for w in split_words_lowercase(&ed.institution) {
            employer_tokens.insert(w);
        }
    }

    let mut project_tokens = HashSet::new();
    for p in &profile.projects {
        for w in split_words_lowercase(&p.name) {
            project_tokens.insert(w);
        }
    }

    let mut skill_tokens = HashSet::new();
    for s in profile
        .skills
        .languages
        .iter()
        .chain(profile.skills.frameworks.iter())
        .chain(profile.skills.tools.iter())
    {
        for w in split_words_lowercase(s) {
            skill_tokens.insert(w);
        }
        // Skills may be short (e.g. "Go", "C") — keep the raw lowercase
        // form too so two-letter langs still match.
        skill_tokens.insert(s.to_lowercase());
    }

    // Scan the whole profile text for year/number tokens.
    let flat = flat_profile_text(profile);
    let year_tokens = year_regex()
        .find_iter(&flat)
        .map(|m| m.as_str().to_string())
        .collect();
    let number_tokens = number_regex()
        .find_iter(&flat)
        .map(|m| m.as_str().to_string())
        .collect();

    ProfileTokenSets {
        employer_tokens,
        project_tokens,
        skill_tokens,
        year_tokens,
        number_tokens,
    }
}

fn flat_profile_text(profile: &Profile) -> String {
    let mut out = String::new();
    out.push_str(&profile.summary);
    out.push(' ');
    for exp in &profile.experience {
        out.push_str(&exp.title);
        out.push(' ');
        out.push_str(&exp.company);
        out.push(' ');
        out.push_str(&exp.location);
        out.push(' ');
        out.push_str(&exp.start);
        out.push(' ');
        out.push_str(&exp.end);
        out.push(' ');
        for b in &exp.bullets {
            out.push_str(b);
            out.push(' ');
        }
    }
    for ed in &profile.education {
        out.push_str(&ed.degree);
        out.push(' ');
        out.push_str(&ed.institution);
        out.push(' ');
        out.push_str(&ed.start);
        out.push(' ');
        out.push_str(&ed.end);
        out.push(' ');
    }
    for p in &profile.projects {
        out.push_str(&p.name);
        out.push(' ');
        out.push_str(&p.url);
        out.push(' ');
        for b in &p.bullets {
            out.push_str(b);
            out.push(' ');
        }
    }
    out
}

/// Reject a reword that introduces employers, numbers, years, or proper
/// nouns absent from the profile. The original bullet is allowed to widen
/// the accept-set for numbers (but not proper nouns — rewords shouldn't
/// recycle some unrelated employer name just because it was in the
/// original bullet; in practice, any original-bullet employer is already
/// in the profile's employer set).
pub fn forbid_invented_entities(
    new_text: &str,
    original_bullet: &str,
    profile: &Profile,
    path: &str,
) -> Result<()> {
    let sets = build_token_sets(profile);

    // --- Numbers (including years) ---
    let original_numbers: HashSet<String> = number_regex()
        .find_iter(original_bullet)
        .map(|m| m.as_str().to_string())
        .collect();
    for m in number_regex().find_iter(new_text) {
        let tok = m.as_str().to_string();
        if original_numbers.contains(&tok) {
            continue;
        }
        if sets.number_tokens.contains(&tok) {
            continue;
        }
        if sets.year_tokens.contains(&tok) {
            continue;
        }
        // Year reject: if the token matches YYYY pattern, flag as year;
        // otherwise generic number. Both surface the token for debugging.
        let reason = if year_regex().is_match(&tok) {
            "invented year"
        } else {
            "invented number"
        };
        return Err(TailorError::InventedContent {
            path: path.to_string(),
            offending_token: tok,
            original_bullet: original_bullet.to_string(),
            reason,
        });
    }

    // --- Proper nouns ---
    for m in proper_noun_regex().find_iter(new_text) {
        let span = m.as_str();
        // Skip spans that are entirely at sentence start (single capitalized
        // leading word followed by space-then-lowercase is common prose).
        // We still require the span's tokens to intersect a known set if
        // ANY token within the span is ≥ 3 chars and not a stopword.
        let tokens: Vec<String> = span
            .split_whitespace()
            .map(str::to_lowercase)
            .filter(|t| t.len() >= 3)
            .collect();
        if tokens.is_empty() {
            continue;
        }
        let mut intersects = false;
        for t in &tokens {
            // Strip trailing punctuation for set comparison.
            let stripped: String = t
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '+' || *c == '&')
                .collect();
            if sets.employer_tokens.contains(&stripped)
                || sets.project_tokens.contains(&stripped)
                || sets.skill_tokens.contains(&stripped)
                || COMMON_ENGLISH_CAPS.contains(&stripped.as_str())
            {
                intersects = true;
                break;
            }
        }
        if !intersects {
            return Err(TailorError::InventedContent {
                path: path.to_string(),
                offending_token: span.to_string(),
                original_bullet: original_bullet.to_string(),
                reason: "invented proper noun",
            });
        }
    }

    Ok(())
}

/// Words that are commonly capitalized in English prose but are not proper
/// nouns — we allow them through to avoid false positives on sentence
/// starters. Intentionally short; everything else has to match the profile.
const COMMON_ENGLISH_CAPS: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "into",
    "from",
    "over",
    "when",
    "where",
    "which",
    "while",
    "built",
    "led",
    "shipped",
    "drove",
    "designed",
    "implemented",
    "tokenizer",
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::schema::{Education, Experience, Personal, Profile, Project, Skills};

    fn fixture() -> Profile {
        Profile {
            personal: Personal {
                name: "Ada".into(),
                ..Default::default()
            },
            summary: "Summary here".into(),
            skills: Skills {
                languages: vec!["Rust".into(), "Python".into()],
                frameworks: vec!["Kubernetes".into(), "Tokio".into()],
                tools: vec![],
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
            }],
            projects: vec![Project {
                name: "OpenLLM".into(),
                url: String::new(),
                bullets: vec!["tokenizer in Rust".into()],
            }],
        }
    }

    #[test]
    fn accepts_employer_from_profile() {
        let p = fixture();
        forbid_invented_entities("Shipped at Acme Robotics.", "", &p, "x").unwrap();
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
}
