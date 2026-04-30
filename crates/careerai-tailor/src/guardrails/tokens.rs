//! Token-set builder: regex factories, [`ProfileTokenSets`], and the
//! helpers that extract employer / project / skill / year / number /
//! proper-noun tokens from a [`Profile`].

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use careerai_profile::schema::Profile;

pub(crate) fn number_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::unwrap_used)]
    RE.get_or_init(|| Regex::new(r"\b\d+(?:\.\d+)?%?\b").unwrap())
}

pub(crate) fn year_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::unwrap_used)]
    RE.get_or_init(|| Regex::new(r"\b(?:19|20)\d{2}\b").unwrap())
}

/// Extract consecutive Capitalized-token runs. Tokens may include a handful
/// of connector characters common in tech names (`+`, `&`, `#`, `.`). We
/// match on unicode categories so tokens containing homoglyphs (Cyrillic
/// `о` in "Gооgle", etc.) still register as proper nouns — which is
/// desirable because they will NOT lowercase-equal any profile token and
/// will be rejected by the intersection check below.
pub(crate) fn proper_noun_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    #[allow(clippy::unwrap_used)]
    RE.get_or_init(|| {
        // `\p{Lu}` = any uppercase letter (ASCII or not).
        // `[\p{L}\p{N}+&#.]` = letters (any script), digits, or connectors.
        Regex::new(r"\p{Lu}[\p{L}\p{N}+&#.]*(?:\s+\p{Lu}[\p{L}\p{N}+&#.]*)*").unwrap()
    })
}

/// Pre-computed lookup sets derived from a `Profile`. Built once per
/// `validate()` call and reused across every reword op — rebuilding per
/// op was O(ops × profile_size) and dominated validate time for large
/// profiles.
#[allow(clippy::struct_field_names)]
pub(crate) struct ProfileTokenSets {
    pub(crate) employer_tokens: HashSet<String>,
    pub(crate) project_tokens: HashSet<String>,
    pub(crate) skill_tokens: HashSet<String>,
    /// Proper-noun tokens scraped from the full flattened profile text
    /// (summary + experience bullets + project bullets + ...). This
    /// captures system/library/protocol names that legitimately appear
    /// in the user's prose but are not modeled as structured employers,
    /// project names, or skills — `Linux`, `Wayland`, `X11`, `Docker`,
    /// `ROS`, etc. Without this, every reword that reused such a noun
    /// was rejected as "invented proper noun" even when the term was
    /// clearly the user's own.
    pub(crate) summary_proper_nouns: HashSet<String>,
    pub(crate) year_tokens: HashSet<String>,
    pub(crate) number_tokens: HashSet<String>,
}

fn split_words_lowercase(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(str::to_lowercase)
}

pub(crate) fn build_token_sets(profile: &Profile) -> ProfileTokenSets {
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
        // Skills may be short (e.g. "Go", "C") or contain kept punctuation
        // (e.g. "C++", "C#") — keep the raw lowercase form too so two-letter
        // langs and `#`-suffixed langs still match against `strip_for_match`.
        skill_tokens.insert(s.to_lowercase());
    }

    // Scan the whole profile text for year/number tokens.
    let flat = flat_profile_text(profile);
    let year_tokens = year_regex()
        .find_iter(&flat)
        .map(|m| m.as_str().to_string())
        .collect();
    // Numbers: store both the raw token ("35%") and the bare digits form
    // ("35") so `forbid_invented_entities` accepts `"35 years"` when the
    // profile only has "35%" or vice-versa.
    let mut number_tokens: HashSet<String> = HashSet::new();
    for m in number_regex().find_iter(&flat) {
        let raw = m.as_str().to_string();
        number_tokens.insert(raw.clone());
        if let Some(bare) = raw.strip_suffix('%') {
            number_tokens.insert(bare.to_string());
        }
    }

    // Proper-noun harvest from the summary + structured identifier
    // fields (company names, project names, education institutions).
    //
    // This deliberately does NOT include experience bullet bodies or
    // project description bullets. Previously the harvest scanned the
    // FULL flattened profile, which let a tailored bullet for employer
    // A reuse a proper noun mentioned only in employer B's bullet — a
    // "cross-bullet token leak" that would write "...accelerated
    // Honeywell's deployment of Claude..." for an Anthropic-role bullet
    // because Honeywell appeared anywhere in the profile.
    //
    // We still need to scan structured identifier fields with the
    // proper-noun regex (rather than relying on `employer_tokens`
    // alone) because `split_words_lowercase` drops punctuation and
    // short fragments — `AT&T` becomes nothing in `employer_tokens`
    // since `at`/`t` are both shorter than 3 chars. The proper-noun
    // regex keeps `&`, `+`, `#`, `.` as connectors so `AT&T` survives.
    //
    // The other allowlists cover the rest:
    //  * `skill_tokens`    — every skill / language / framework / tool
    //  * `original_proper_nouns` (per-bullet, computed at check-time)
    //  * `COMMON_ENGLISH_CAPS` — the curated whitelist
    let mut allowed_text = String::new();
    allowed_text.push_str(&profile.summary);
    allowed_text.push(' ');
    for exp in &profile.experience {
        allowed_text.push_str(&exp.company);
        allowed_text.push(' ');
        allowed_text.push_str(&exp.title);
        allowed_text.push(' ');
        allowed_text.push_str(&exp.location);
        allowed_text.push(' ');
    }
    for ed in &profile.education {
        allowed_text.push_str(&ed.institution);
        allowed_text.push(' ');
        allowed_text.push_str(&ed.degree);
        allowed_text.push(' ');
    }
    for p in &profile.projects {
        allowed_text.push_str(&p.name);
        allowed_text.push(' ');
    }
    let mut summary_proper_nouns: HashSet<String> = HashSet::new();
    for m in proper_noun_regex().find_iter(&allowed_text) {
        for raw in m.as_str().split_whitespace() {
            let lower = raw.to_lowercase();
            if lower.len() < 3 {
                continue;
            }
            summary_proper_nouns.insert(strip_for_match(&lower));
        }
    }

    ProfileTokenSets {
        employer_tokens,
        project_tokens,
        skill_tokens,
        summary_proper_nouns,
        year_tokens,
        number_tokens,
    }
}

/// Flatten profile into a single space-separated string for regex scans.
/// Stays `pub(crate)` so `diff.rs` can reuse it without duplicating the
/// walker.
pub(crate) fn flat_profile_text(profile: &Profile) -> String {
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

/// Strip a token down to comparable form. Keeps alphanumerics + `+`, `&`,
/// `#` so tech identifiers like "C++", "AT&T", "C#" survive intact.
pub(crate) fn strip_for_match(t: &str) -> String {
    t.chars()
        .filter(|c| c.is_alphanumeric() || *c == '+' || *c == '&' || *c == '#')
        .collect()
}
