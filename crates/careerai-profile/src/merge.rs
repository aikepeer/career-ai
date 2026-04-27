//! Merge multiple parsed [`Profile`]s into one canonical profile.
//!
//! Merge order: earlier profiles are the base, later profiles **fill gaps and
//! contribute additional list items** — they do NOT overwrite non-empty scalar
//! fields on the base. This makes the merge commutative where it matters
//! (list concatenation + dedupe) and deterministic where it doesn't (first
//! source wins for scalars). Summary is the one exception: longer wins.
//!
//! De-duplication:
//! - Experience: by `(company_lc, title_lc, start)` — first seen wins; later
//!   duplicates donate missing bullets/location/end.
//! - Education: by `(institution_lc, degree_lc)`.
//! - Projects: by `name_lc`.
//! - Skills: case-preserving, order-preserving, dedup by lowercase.

use crate::schema::{Education, Experience, Profile, Project, Skills};

#[must_use]
pub fn merge_all(profiles: Vec<Profile>) -> Profile {
    profiles.into_iter().fold(Profile::default(), merge_pair)
}

#[must_use]
pub fn merge_pair(base: Profile, other: Profile) -> Profile {
    Profile {
        personal: merge_personal(base.personal, other.personal),
        summary: prefer_longer(base.summary, other.summary),
        skills: merge_skills(base.skills, other.skills),
        experience: merge_experience(base.experience, other.experience),
        education: merge_education(base.education, other.education),
        projects: merge_projects(base.projects, other.projects),
    }
}

fn merge_personal(
    base: crate::schema::Personal,
    other: crate::schema::Personal,
) -> crate::schema::Personal {
    crate::schema::Personal {
        name: prefer_nonempty(base.name, other.name),
        email: prefer_nonempty(base.email, other.email),
        phone: prefer_nonempty(base.phone, other.phone),
        location: prefer_nonempty(base.location, other.location),
        links: crate::schema::Links {
            github: prefer_nonempty(base.links.github, other.links.github),
            linkedin: prefer_nonempty(base.links.linkedin, other.links.linkedin),
            portfolio: prefer_nonempty(base.links.portfolio, other.links.portfolio),
        },
    }
}

fn merge_skills(base: Skills, other: Skills) -> Skills {
    Skills {
        languages: dedup_keep_order_ci(merge_vecs(base.languages, other.languages)),
        frameworks: dedup_keep_order_ci(merge_vecs(base.frameworks, other.frameworks)),
        tools: dedup_keep_order_ci(merge_vecs(base.tools, other.tools)),
    }
}

fn merge_experience(base: Vec<Experience>, other: Vec<Experience>) -> Vec<Experience> {
    let mut out = base;
    for new in other {
        // Entries with unknown start dates cannot be reliably deduplicated;
        // always treat them as new to avoid collapsing distinct roles.
        if new.start.is_empty() {
            out.push(new);
            continue;
        }
        // Two-stage match. First try the strict (company, title, start)
        // key — that's the historical contract used by snapshot tests.
        // Fall back to (company, start) so that a structured upstream
        // (LinkedIn) entry collapses with a heuristic-parsed one whose
        // title got garbled into a date-string ("Jan 2025"). The LinkedIn
        // entry, having been folded in first, owns the `title` field.
        let strict_key = exp_key(&new);
        let loose_key = exp_loose_key(&new);
        let mut idx = out
            .iter()
            .position(|e| !e.start.is_empty() && exp_key(e) == strict_key);
        if idx.is_none() {
            idx = out
                .iter()
                .position(|e| !e.start.is_empty() && exp_loose_key(e) == loose_key);
        }
        if let Some(i) = idx {
            let existing = &mut out[i];
            existing.bullets = dedup_keep_order(merge_vecs(existing.bullets.clone(), new.bullets));
            existing.location = prefer_nonempty(existing.location.clone(), new.location);
            existing.end = prefer_nonempty(existing.end.clone(), new.end);
            // Title preference: keep base unless it looks like a date-
            // garbage title and `new` has a real one.
            if looks_like_date_garbage(&existing.title) && !looks_like_date_garbage(&new.title) {
                existing.title = new.title;
            }
        } else {
            out.push(new);
        }
    }
    out
}

fn merge_education(base: Vec<Education>, other: Vec<Education>) -> Vec<Education> {
    let mut out = base;
    for new in other {
        let key = edu_key(&new);
        if !out.iter().any(|e| edu_key(e) == key) {
            out.push(new);
        }
    }
    out
}

fn merge_projects(base: Vec<Project>, other: Vec<Project>) -> Vec<Project> {
    let mut out = base;
    for new in other {
        let key = new.name.to_lowercase();
        if let Some(existing) = out.iter_mut().find(|p| p.name.to_lowercase() == key) {
            existing.url = prefer_nonempty(existing.url.clone(), new.url);
            existing.bullets = dedup_keep_order(merge_vecs(existing.bullets.clone(), new.bullets));
        } else {
            out.push(new);
        }
    }
    out
}

fn exp_key(e: &Experience) -> (String, String, String) {
    (
        e.company.to_lowercase(),
        e.title.to_lowercase(),
        e.start.clone(),
    )
}

fn exp_loose_key(e: &Experience) -> (String, String) {
    (e.company.to_lowercase(), e.start.clone())
}

fn looks_like_date_garbage(title: &str) -> bool {
    let t = title.trim();
    if t.is_empty() {
        return true;
    }
    date_like_re().is_match(t)
}

fn date_like_re() -> &'static regex::Regex {
    static DATE_LIKE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    DATE_LIKE.get_or_init(|| {
        // Patterns the heuristic has been observed to emit:
        //   "Jan 2025", "January 2025", "2022", "2022 - Present",
        //   "Jan 2022 - Dec 2024", "2022-01", "2022-01 - 2024-12".
        // Whole-string match (DOTALL doesn't matter — single line).
        #[allow(clippy::expect_used)]
        regex::Regex::new(
            r"(?i)^\s*(?:(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\s+)?\d{4}(?:[-/]\d{1,2})?(?:\s*[-–]\s*(?:present|(?:(?:jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\s+)?\d{4}(?:[-/]\d{1,2})?))?\s*$",
        )
        .expect("date_like regex compiles")
    })
}

fn edu_key(e: &Education) -> (String, String) {
    (e.institution.to_lowercase(), e.degree.to_lowercase())
}

fn prefer_nonempty(a: String, b: String) -> String {
    if a.trim().is_empty() {
        b
    } else {
        a
    }
}

fn prefer_longer(a: String, b: String) -> String {
    if b.len() > a.len() {
        b
    } else {
        a
    }
}

fn merge_vecs<T>(mut a: Vec<T>, b: Vec<T>) -> Vec<T> {
    a.extend(b);
    a
}

/// Case-sensitive dedup preserving first-occurrence order.
/// Used for bullets where casing differences (e.g. acronyms) are meaningful.
fn dedup_keep_order(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

/// Case-insensitive dedup preserving first-occurrence order and casing.
/// Used for skills where "Rust" and "RUST" are the same skill.
fn dedup_keep_order_ci(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let key = item.to_lowercase();
        if seen.insert(key) {
            out.push(item);
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::schema::Personal;

    fn exp(company: &str, title: &str, start: &str, bullets: &[&str]) -> Experience {
        Experience {
            title: title.into(),
            company: company.into(),
            start: start.into(),
            end: "present".into(),
            bullets: bullets.iter().map(|b| (*b).to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn merges_personal_fields_preferring_base_then_filling_empties() {
        let base = Profile {
            personal: Personal {
                name: "Alice".into(),
                email: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };
        let other = Profile {
            personal: Personal {
                name: "Ignored".into(),
                email: "a@example.com".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_pair(base, other);
        assert_eq!(merged.personal.name, "Alice");
        assert_eq!(merged.personal.email, "a@example.com");
    }

    #[test]
    fn dedups_experience_and_merges_bullets() {
        let base = Profile {
            experience: vec![exp("Acme", "Engineer", "2022-01", &["A"])],
            ..Default::default()
        };
        let other = Profile {
            experience: vec![exp("ACME", "engineer", "2022-01", &["A", "B"])],
            ..Default::default()
        };
        let merged = merge_pair(base, other);
        assert_eq!(merged.experience.len(), 1);
        assert_eq!(merged.experience[0].bullets, vec!["A", "B"]);
    }

    #[test]
    fn keeps_distinct_positions() {
        let base = Profile {
            experience: vec![exp("Acme", "Engineer", "2022-01", &[])],
            ..Default::default()
        };
        let other = Profile {
            experience: vec![exp("Beta", "Engineer", "2019-06", &[])],
            ..Default::default()
        };
        let merged = merge_pair(base, other);
        assert_eq!(merged.experience.len(), 2);
    }

    #[test]
    fn dedups_skills_case_insensitive_keeping_first_casing() {
        let base = Profile {
            skills: Skills {
                languages: vec!["Rust".into(), "python".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let other = Profile {
            skills: Skills {
                languages: vec!["RUST".into(), "Go".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_pair(base, other);
        assert_eq!(merged.skills.languages, vec!["Rust", "python", "Go"]);
    }

    #[test]
    fn summary_prefers_longer() {
        let base = Profile {
            summary: "short".into(),
            ..Default::default()
        };
        let other = Profile {
            summary: "a much longer summary".into(),
            ..Default::default()
        };
        let merged = merge_pair(base, other);
        assert_eq!(merged.summary, "a much longer summary");
    }

    #[test]
    fn bullets_with_different_casing_are_kept_distinct() {
        // Bullets are case-sensitive: "API" and "api" are different.
        let base = Profile {
            experience: vec![exp("Acme", "Engineer", "2022-01", &["Designed API"])],
            ..Default::default()
        };
        let other = Profile {
            experience: vec![exp(
                "ACME",
                "engineer",
                "2022-01",
                &["Designed api", "New work"],
            )],
            ..Default::default()
        };
        let merged = merge_pair(base, other);
        assert_eq!(merged.experience.len(), 1);
        assert_eq!(
            merged.experience[0].bullets,
            vec!["Designed API", "Designed api", "New work"]
        );
    }

    #[test]
    fn empty_start_entries_are_never_deduped() {
        // Two roles at the same company/title but unknown start dates must be
        // kept as separate entries, not collapsed into one.
        let base = Profile {
            experience: vec![exp("Acme", "Engineer", "", &["Early work"])],
            ..Default::default()
        };
        let other = Profile {
            experience: vec![exp("Acme", "Engineer", "", &["Later work"])],
            ..Default::default()
        };
        let merged = merge_pair(base, other);
        assert_eq!(merged.experience.len(), 2);
    }

    #[test]
    fn linkedin_title_wins_over_pdf_date_garbage() {
        // Mirrors the real-world failure: PDF heuristic stuffed "Jan 2025"
        // into the title field. LinkedIn (folded in as base) has the real
        // title. After the merge, the LinkedIn title must survive.
        let linkedin = Profile {
            experience: vec![exp(
                "Acme Robotics",
                "Senior Embedded Developer",
                "2022-01",
                &["Led migration"],
            )],
            ..Default::default()
        };
        let pdf_garbage = Profile {
            experience: vec![exp(
                "Acme Robotics",
                "Jan 2025",
                "2022-01",
                &["Cut latency"],
            )],
            ..Default::default()
        };
        let merged = merge_pair(linkedin, pdf_garbage);
        assert_eq!(merged.experience.len(), 1);
        assert_eq!(merged.experience[0].title, "Senior Embedded Developer");
        // Bullets from both sources still merge.
        assert!(merged.experience[0]
            .bullets
            .iter()
            .any(|b| b.contains("migration")));
        assert!(merged.experience[0]
            .bullets
            .iter()
            .any(|b| b.contains("latency")));
    }

    #[test]
    fn pdf_title_wins_when_linkedin_is_garbage() {
        // Symmetry check: if the *base* has the date-garbage title and
        // the second source has a real one, prefer the real title.
        let bad_base = Profile {
            experience: vec![exp("Acme", "Jan 2022", "2022-01", &[])],
            ..Default::default()
        };
        let good_other = Profile {
            experience: vec![exp("Acme", "Engineer", "2022-01", &[])],
            ..Default::default()
        };
        let merged = merge_pair(bad_base, good_other);
        assert_eq!(merged.experience.len(), 1);
        assert_eq!(merged.experience[0].title, "Engineer");
    }

    #[test]
    fn looks_like_date_garbage_classifies_known_shapes() {
        assert!(looks_like_date_garbage(""));
        assert!(looks_like_date_garbage("Jan 2025"));
        assert!(looks_like_date_garbage("January 2025"));
        assert!(looks_like_date_garbage("2022"));
        assert!(looks_like_date_garbage("2022-01"));
        assert!(looks_like_date_garbage("2022 - Present"));
        assert!(looks_like_date_garbage("Jan 2022 - Dec 2024"));
        assert!(!looks_like_date_garbage("Senior Embedded Developer"));
        assert!(!looks_like_date_garbage("Engineer III"));
        // Words containing month-prefix substrings must NOT match (the
        // pattern requires the month token followed by whitespace +
        // digits, so a plain word like "Marketing" stays a real title).
        assert!(!looks_like_date_garbage("Marketing Lead"));
    }
}
