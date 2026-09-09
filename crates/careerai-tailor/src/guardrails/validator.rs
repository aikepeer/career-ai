//! Entity-level reword guardrail: reject invented employers, numbers,
//! years, or proper nouns that don't appear in the profile (or,
//! narrowly, in the original bullet).

use std::collections::HashSet;

use careerai_profile::schema::Profile;

use super::common_caps::COMMON_ENGLISH_CAPS;
use super::tokens::{
    build_token_sets, number_regex, proper_noun_regex, strip_for_match, year_regex,
    ProfileTokenSets,
};
use crate::error::{Result, TailorError};

/// Backwards-compatible wrapper: builds token sets on each call. Prefer
/// `forbid_invented_entities_with` when validating many rewords against
/// the same profile.
pub fn forbid_invented_entities(
    new_text: &str,
    original_bullet: &str,
    profile: &Profile,
    path: &str,
) -> Result<()> {
    let sets = build_token_sets(profile);
    forbid_invented_entities_with(new_text, original_bullet, &sets, path)
}

/// Reject a reword that introduces employers, numbers, years, or proper
/// nouns absent from the profile. Token sets are pre-built by the caller
/// via `build_token_sets` — the validator in `diff.rs` builds them once
/// per `validate()` call and passes them through here.
pub(crate) fn forbid_invented_entities_with(
    new_text: &str,
    original_bullet: &str,
    sets: &ProfileTokenSets,
    path: &str,
) -> Result<()> {
    // --- Numbers (including years) ---
    let mut original_numbers: HashSet<String> = HashSet::new();
    for m in number_regex().find_iter(original_bullet) {
        let raw = m.as_str().to_string();
        if let Some(bare) = raw.strip_suffix('%') {
            original_numbers.insert(bare.to_string());
        }
        original_numbers.insert(raw);
    }
    for m in number_regex().find_iter(new_text) {
        if is_part_of_identifier(new_text, m.start(), m.end()) {
            continue;
        }
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
    // Pre-extract proper-noun tokens from the original bullet so a
    // reword may legitimately reuse codenames / system names that
    // aren't in the profile's employer / project / skill token sets.
    // The doc-comment on this function explicitly promises this
    // carve-out ("...or, narrowly, in the original bullet"). Earlier
    // implementations honored it for numbers but silently dropped it
    // for proper nouns, rejecting every reword that echoed a system
    // / project codename from the source bullet.
    let mut original_proper_nouns: HashSet<String> = HashSet::new();
    for m in proper_noun_regex().find_iter(original_bullet) {
        for raw in m.as_str().split_whitespace() {
            let lower = raw.to_lowercase();
            if lower.len() < 3 {
                continue;
            }
            original_proper_nouns.insert(strip_for_match(&lower));
        }
    }

    // Extend the carve-out to ANY word of the original bullet, not just
    // its already-Capitalized runs. A reword may legitimately capitalize
    // an existing word at sentence start ("curated" → "Curated"), and
    // the Capitalized-run scan would miss the lowercase form entirely.
    // Still strictly per-bullet: a token that lives only in another
    // bullet or is invented outright keeps failing below.
    let mut original_words: HashSet<String> = HashSet::new();
    for w in original_bullet.split(|c: char| !c.is_alphanumeric()) {
        let lower = w.to_lowercase();
        if lower.len() >= 3 {
            original_words.insert(strip_for_match(&lower));
        }
    }

    // SAFETY: every substantive token in a span must clear the
    // intersection set. The earlier loop short-circuited on the first
    // match, which let an invented noun ride along inside a span where
    // any neighboring token happened to be whitelisted (e.g. the
    // sentence-start verb "Architected" let "Google Ads" slip through
    // even though neither is in the profile). Validate each token
    // independently and surface the first invented token by name.
    for m in proper_noun_regex().find_iter(new_text) {
        let span = m.as_str();
        for raw in span.split_whitespace() {
            let lower = raw.to_lowercase();
            if lower.len() < 3 {
                // Punctuation / acronyms < 3 chars don't carry signal.
                continue;
            }
            let stripped = strip_for_match(&lower);
            let cleared = sets.employer_tokens.contains(&stripped)
                || sets.project_tokens.contains(&stripped)
                || sets.skill_tokens.contains(&stripped)
                || sets.skill_tokens.contains(&lower)
                || sets.summary_proper_nouns.contains(&stripped)
                || sets.jd_proper_nouns.contains(&stripped)
                || original_proper_nouns.contains(&stripped)
                || original_words.contains(&stripped)
                || COMMON_ENGLISH_CAPS.contains(&stripped.as_str())
                || matches_root_in_caps(&stripped)
                || matches_root_in_set(&stripped, &original_words)
                || matches_root_in_set(&stripped, &sets.skill_tokens)
                || matches_root_in_set(&stripped, &sets.jd_proper_nouns)
                || (lower.contains('-')
                    && lower.split('-').all(|sub| {
                        let s = strip_for_match(sub);
                        s.len() < 3
                            || COMMON_ENGLISH_CAPS.contains(&s.as_str())
                            || matches_root_in_caps(&s)
                            || sets.skill_tokens.contains(&s)
                            || sets.jd_proper_nouns.contains(&s)
                            || sets.summary_proper_nouns.contains(&s)
                            || original_words.contains(&s)
                            || matches_root_in_set(&s, &original_words)
                    }));
            if !cleared {
                return Err(TailorError::InventedContent {
                    path: path.to_string(),
                    offending_token: raw.to_string(),
                    original_bullet: original_bullet.to_string(),
                    reason: "invented proper noun",
                });
            }
        }
    }

    Ok(())
}

fn is_part_of_identifier(text: &str, start: usize, end: usize) -> bool {
    let bytes = text.as_bytes();
    if start > 0 {
        let prev = bytes[start - 1];
        if prev == b'.' || prev == b'-' || prev == b'_' || prev.is_ascii_alphabetic() {
            if (prev == b'.' || prev == b'-') && start > 1 && bytes[start - 2].is_ascii_alphabetic()
            {
                return true;
            }
            if prev == b'_' || prev.is_ascii_alphabetic() {
                return true;
            }
        }
    }
    if end < bytes.len() {
        let next = bytes[end];
        if next == b'x' || next == b'X' || next == b'_' || next.is_ascii_alphabetic() {
            return true;
        }
    }
    false
}

fn word_root(w: &str) -> &str {
    let s = w;
    if let Some(r) = s
        .strip_suffix("izations")
        .or_else(|| s.strip_suffix("ization"))
    {
        return r;
    }
    if let Some(r) = s.strip_suffix("ations").or_else(|| s.strip_suffix("ation")) {
        return r;
    }
    if let Some(r) = s.strip_suffix("ments").or_else(|| s.strip_suffix("ment")) {
        return r;
    }
    if let Some(r) = s.strip_suffix("ability").or_else(|| s.strip_suffix("able")) {
        return r;
    }
    if let Some(r) = s.strip_suffix("ities").or_else(|| s.strip_suffix("ity")) {
        return r;
    }
    if let Some(r) = s.strip_suffix("ings").or_else(|| s.strip_suffix("ing")) {
        return r;
    }
    if let Some(r) = s.strip_suffix("ive") {
        return r;
    }
    if let Some(r) = s.strip_suffix("ed") {
        return r;
    }
    if let Some(r) = s.strip_suffix("es").or_else(|| s.strip_suffix('s')) {
        return r;
    }
    s
}

fn matches_root_in_set(token: &str, set: &HashSet<String>) -> bool {
    let r1 = word_root(token);
    if r1.len() < 4 {
        return false;
    }
    set.iter().any(|target| {
        let r2 = word_root(target);
        r2.len() >= 4 && (r1 == r2 || r1.starts_with(r2) || r2.starts_with(r1))
    })
}

fn matches_root_in_caps(token: &str) -> bool {
    let r1 = word_root(token);
    if r1.len() < 4 {
        return false;
    }
    COMMON_ENGLISH_CAPS.iter().any(|target| {
        let r2 = word_root(target);
        r2.len() >= 4 && (r1 == r2 || r1.starts_with(r2) || r2.starts_with(r1))
    })
}
