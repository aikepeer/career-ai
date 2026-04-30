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
    summary_proper_nouns: HashSet<String>,
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
                || original_proper_nouns.contains(&stripped)
                || COMMON_ENGLISH_CAPS.contains(&stripped.as_str());
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
    // --- connectors / pronouns / determiners ---
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
    "their",
    "these",
    "those",
    "such",
    "each",
    "every",
    "some",
    "any",
    "both",
    "either",
    "neither",
    "many",
    "most",
    "several",
    "few",
    // --- temporal / sequence adverbs (common at sentence start) ---
    "currently",
    "previously",
    "recently",
    "today",
    "yesterday",
    "now",
    "then",
    "later",
    "earlier",
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "initially",
    "finally",
    "lastly",
    "eventually",
    "subsequently",
    "simultaneously",
    "since",
    "until",
    "before",
    // --- frequency / qualifier adverbs ---
    "successfully",
    "additionally",
    "furthermore",
    "moreover",
    "however",
    "therefore",
    "thus",
    "hence",
    "specifically",
    "particularly",
    "generally",
    "typically",
    "usually",
    "frequently",
    "occasionally",
    "always",
    "often",
    "sometimes",
    "rarely",
    "never",
    "consistently",
    "deeply",
    "directly",
    "primarily",
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
    "stabilized",
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

    /// Same-bullet rewords that re-use proper nouns from the original
    /// bullet body (e.g. `Linux`, `Wayland`, `X11`, `ROS`) must still
    /// clear the guardrail. The per-bullet `original_proper_nouns`
    /// allowlist covers this — a legitimate "Migrated the Wayland
    /// compositor" reword pulled from a "...X11 to Wayland..."
    /// original is accepted.
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

        // Same-bullet reword: original mentions "Wayland", reword
        // reuses it. Must be accepted via `original_proper_nouns`.
        forbid_invented_entities(
            "Migrated the Wayland compositor for the new platform.",
            "Ported display servers from X11 to Wayland on embedded boards.",
            &p,
            "x",
        )
        .unwrap();
    }

    /// Cross-bullet token leak is rejected. A proper noun that appears
    /// only in a *different* experience entry's bullet (not in this
    /// bullet's original, not in summary, not in skills/employers)
    /// must NOT be reusable by the tailoring LLM. This is the
    /// regression Codex flagged after v0.1.1-mcp: the previous
    /// implementation harvested proper nouns from the entire flat
    /// profile, which let an Anthropic-role tailoring leak in
    /// "Honeywell" from an unrelated 2018 role.
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

        // Different-bullet reword: original is unrelated text, reword
        // tries to reuse "Wayland" (which only appears in another
        // bullet). Must be rejected.
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

    /// Proper nouns in the profile *summary* are still reusable
    /// anywhere — the summary is high-level vocabulary the candidate
    /// self-positions with, so cross-bullet sharing is appropriate.
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
        };

        // Reword pulls "Wayland" from the summary even though the
        // bullet original doesn't mention it. Accepted via
        // `summary_proper_nouns`.
        forbid_invented_entities(
            "Stabilized Wayland compositor performance.",
            "Built display servers.",
            &p,
            "x",
        )
        .unwrap();
    }

    /// Regression for Codex P1 on PR #53: punctuated employer names
    /// like `AT&T` must clear the guardrail. `employer_tokens` is
    /// built via `split_words_lowercase`, which drops fragments
    /// shorter than 3 chars — so `AT&T` splits into `at` (dropped)
    /// and `t` (dropped) and ends up as nothing in `employer_tokens`.
    /// We compensate by also running the proper-noun regex (which
    /// keeps `&`/`+`/`#`/`.` as connectors) over the structured
    /// identifier fields (company, project name, institution), not
    /// just the summary.
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
        };

        // Reword reuses "AT&T" — present in the company field but not
        // in the summary, not in skills, and not in this bullet's
        // original. Must be accepted via the proper-noun harvest of
        // structured identifier fields.
        forbid_invented_entities(
            "Migrated AT&T billing systems to a new platform.",
            "Worked on telecom infrastructure.",
            &p,
            "x",
        )
        .unwrap();
    }

    /// Regression: common temporal / qualifier adverbs at sentence
    /// start (`Currently`, `Recently`, `Successfully`, `However`, ...)
    /// are not proper nouns. Resume bullets and summaries open with
    /// these all the time. Without coverage, validate() rejects the
    /// whole tailor cycle as "invented proper noun".
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

    /// SAFETY REGRESSION test: the proper-noun check must not let an
    /// invented noun ride along inside a span where ONE token happens
    /// to be whitelisted. Earlier the loop short-circuited on the first
    /// match (e.g. `Architected` in COMMON_ENGLISH_CAPS) and returned
    /// success for the whole span, letting `Google` and `Ads` slip
    /// through unchecked. Each token must clear the intersection set
    /// independently.
    #[test]
    fn rejects_invented_token_riding_along_in_whitelisted_span() {
        let p = fixture();
        // "Architected" is whitelisted as a sentence-start verb; "Google"
        // is NOT in the profile (employer is "Acme Robotics"). The full
        // span "Architected Google Ads" must therefore reject because
        // "Google" is invented.
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
