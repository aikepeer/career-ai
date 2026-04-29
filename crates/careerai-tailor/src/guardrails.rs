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
/// of connector characters common in tech names (`+`, `&`, `#`, `.`). We
/// match on unicode categories so tokens containing homoglyphs (Cyrillic
/// `о` in "Gооgle", etc.) still register as proper nouns — which is
/// desirable because they will NOT lowercase-equal any profile token and
/// will be rejected by the intersection check below.
fn proper_noun_regex() -> &'static Regex {
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
    employer_tokens: HashSet<String>,
    project_tokens: HashSet<String>,
    skill_tokens: HashSet<String>,
    /// Proper-noun tokens scraped from the full flattened profile text
    /// (summary + experience bullets + project bullets + ...). This
    /// captures system/library/protocol names that legitimately appear
    /// in the user's prose but are not modeled as structured employers,
    /// project names, or skills — `Linux`, `Wayland`, `X11`, `Docker`,
    /// `ROS`, etc. Without this, every reword that reused such a noun
    /// was rejected as "invented proper noun" even when the term was
    /// clearly the user's own.
    profile_proper_nouns: HashSet<String>,
    year_tokens: HashSet<String>,
    number_tokens: HashSet<String>,
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

    // Profile-wide proper-noun harvest. Same regex as the new-text
    // scan; we strip-and-lowercase to match the intersection format.
    let mut profile_proper_nouns: HashSet<String> = HashSet::new();
    for m in proper_noun_regex().find_iter(&flat) {
        for raw in m.as_str().split_whitespace() {
            let lower = raw.to_lowercase();
            if lower.len() < 3 {
                continue;
            }
            profile_proper_nouns.insert(strip_for_match(&lower));
        }
    }

    ProfileTokenSets {
        employer_tokens,
        project_tokens,
        skill_tokens,
        profile_proper_nouns,
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

/// Strip a token down to comparable form. Keeps alphanumerics + `+`, `&`,
/// `#` so tech identifiers like "C++", "AT&T", "C#" survive intact.
fn strip_for_match(t: &str) -> String {
    t.chars()
        .filter(|c| c.is_alphanumeric() || *c == '+' || *c == '&' || *c == '#')
        .collect()
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

    for m in proper_noun_regex().find_iter(new_text) {
        let span = m.as_str();
        let mut intersects = false;
        let mut any_substantive = false;
        for raw in span.split_whitespace() {
            let lower = raw.to_lowercase();
            if lower.len() < 3 {
                continue;
            }
            any_substantive = true;
            let stripped = strip_for_match(&lower);
            if sets.employer_tokens.contains(&stripped)
                || sets.project_tokens.contains(&stripped)
                || sets.skill_tokens.contains(&stripped)
                || sets.skill_tokens.contains(&lower)
                || sets.profile_proper_nouns.contains(&stripped)
                || original_proper_nouns.contains(&stripped)
                || COMMON_ENGLISH_CAPS.contains(&stripped.as_str())
            {
                intersects = true;
                break;
            }
        }
        if !any_substantive {
            continue;
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

/// Words commonly capitalized at sentence start in English prose. Kept
/// curated rather than open-ended; anything outside this list (and the
/// profile's employer / project / skill token sets, and the original
/// bullet's proper-noun set) is treated as a candidate invented proper
/// noun. The list bundles two categories:
///
/// 1. Sentence connectors / pronouns (`The`, `And`, `That`, ...). Almost
///    every English bullet that doesn't lead with a verb leads with one
///    of these.
/// 2. Common resume-action verbs (`Built`, `Architected`, `Engineered`,
///    `Optimized`, `Migrated`, ...). The LLM legitimately picks from
///    the standard resume-verb thesaurus, and these get sentence-start
///    capitalization automatically. Without them the guardrail
///    false-rejects rewords whose lead verb wasn't in the original
///    seed list of seven.
const COMMON_ENGLISH_CAPS: &[&str] = &[
    // --- connectors / pronouns ---
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
    "after",
    "during",
    "using",
    "through",
    "also",
    // --- resume-action verbs ---
    "built",
    "led",
    "shipped",
    "drove",
    "designed",
    "implemented",
    "owned",
    "delivered",
    "architected",
    "engineered",
    "developed",
    "deployed",
    "optimized",
    "migrated",
    "refactored",
    "scaled",
    "automated",
    "integrated",
    "established",
    "achieved",
    "reduced",
    "increased",
    "coordinated",
    "mentored",
    "spearheaded",
    "defined",
    "authored",
    "modernized",
    "hardened",
    "instrumented",
    "profiled",
    "tuned",
    "accelerated",
    "parallelized",
    "decoupled",
    "simplified",
    "managed",
    "executed",
    "transformed",
    "validated",
    "prototyped",
    "championed",
    "partnered",
    "collaborated",
    "contributed",
    "supported",
    "enabled",
    "drafted",
    "presented",
    "analyzed",
    "investigated",
    "diagnosed",
    "resolved",
    "rewrote",
    "rolled",
    "rolled-out",
    "rolled-back",
    "promoted",
    "expanded",
    "consolidated",
    "documented",
    "tested",
    "benchmarked",
    "monitored",
    "enforced",
    "secured",
    "audited",
    "ported",
    "packaged",
    "released",
    "configured",
    "ramped",
    "kicked",
    "saved",
    "trimmed",
    "fixed",
    "patched",
    "added",
    "removed",
    "replaced",
    "introduced",
    "improved",
    "raised",
    "doubled",
    "tripled",
    "halved",
    "tracked",
    "received",
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
                languages: vec!["Rust".into(), "Python".into(), "C#".into()],
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
    fn accepts_csharp_skill_with_hash_kept_intact() {
        let p = fixture();
        // "C#" must survive the strip_for_match normalization so it
        // intersects with `profile.skills.languages = ["C#"]`.
        forbid_invented_entities("built systems in C# and Rust", "", &p, "x").unwrap();
    }

    #[test]
    fn accepts_bare_number_when_profile_has_percent_form() {
        let p = fixture();
        // Profile has "35% throughput win" — the number_regex captures
        // the bare form "35". A reword mentioning "35 engineers" must be
        // accepted because "35" is in the number token set.
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

    /// Regression: a reword that reuses a proper noun from the **original
    /// bullet** must be accepted, even if that proper noun isn't in the
    /// profile's employer / project / skill token sets. The function's
    /// docstring explicitly promises this carve-out: "may not invent ...
    /// proper nouns that don't appear in the profile (or, narrowly, in
    /// the original bullet)". Before the fix only numbers honored that
    /// carve-out; proper nouns silently violated it. Real-world impact:
    /// every reword bullet that referenced a project / system codename
    /// (e.g. `Symbot6`, `OpenAMP`) failed validation despite the LLM
    /// faithfully echoing the term from the source bullet.
    #[test]
    fn accepts_proper_noun_from_original_bullet() {
        let p = fixture();
        // Symbot6 is not an employer/project/skill in the profile, but
        // it IS in the original bullet, so this reword is legitimate.
        forbid_invented_entities(
            "Owned the Symbot6 platform from boot to telemetry.",
            "Architected the Symbot6 platform on Linux.",
            &p,
            "x",
        )
        .unwrap();
    }

    /// Regression: common resume-action verbs at sentence start
    /// (`Architected`, `Engineered`, `Optimized`, `Migrated`, ...) are
    /// not proper nouns — they're capitalized only because the bullet
    /// starts there. A profile guardrail that flags them as "invented
    /// proper noun" silently rejects every reword whose lead verb the
    /// LLM picked from the resume-action thesaurus rather than the
    /// short pre-existing whitelist (`Built`, `Led`, `Shipped`, etc.).
    #[test]
    fn accepts_common_resume_action_verbs_at_sentence_start() {
        let p = fixture();
        // Each of these starts a typical resume bullet. None reference
        // a profile employer/project/skill, none appear in the trivial
        // original bullet — so they MUST clear the guardrail purely on
        // the sentence-start carve-out.
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

    /// Regression: proper nouns appearing in profile bullet bodies
    /// (e.g. `Linux`, `Wayland`, `X11`, `ROS`) must clear the guardrail
    /// even though they're not employers, project names, or skills.
    /// `build_token_sets` previously only pulled from the structured
    /// fields (companies / project.name / skills), so a perfectly
    /// legitimate reword that referenced `Linux` from a bullet was
    /// rejected as "invented proper noun".
    #[test]
    fn accepts_proper_noun_from_profile_bullet_body() {
        // Build a profile whose bullet mentions "Wayland" but where
        // Wayland is NOT an employer, project name, or skill. The
        // reword must be allowed to reuse "Wayland" because it was
        // already in the user's profile prose.
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
            },
            experience: vec![Experience {
                title: "Engineer".into(),
                company: "Acme".into(),
                location: String::new(),
                start: "2020".into(),
                end: "2023".into(),
                bullets: vec![
                    "Ported display servers from X11 to Wayland on embedded boards.".into(),
                ],
            }],
            education: vec![],
            projects: vec![],
        };

        // Reword reuses "Wayland" — present in the bullet body, NOT a
        // company/project/skill. Must be accepted.
        forbid_invented_entities(
            "Migrated the Wayland compositor for the new platform.",
            "Ported display servers from X11 to Wayland on embedded boards.",
            &p,
            "x",
        )
        .unwrap();

        // And a different bullet's reword (with empty original) should
        // ALSO accept "Wayland" because it appears in the profile prose,
        // not just this specific bullet.
        forbid_invented_entities(
            "Stabilized Wayland compositor performance.",
            "different bullet original here",
            &p,
            "x",
        )
        .unwrap();
    }

    /// Pinning the negative case: a proper noun that is in NEITHER the
    /// profile NOR the original bullet must still be rejected — the
    /// original-bullet whitelist must not over-broaden the carve-out.
    #[test]
    fn still_rejects_proper_noun_absent_from_both_profile_and_original() {
        let p = fixture();
        let err = forbid_invented_entities(
            "Owned the Symbot7 platform end-to-end.", // 7, not 6
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
}
