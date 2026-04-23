//! Raw resume text → structured [`Profile`] via heuristic section splitting.
//!
//! Resumes rarely parse cleanly. This module extracts the boring,
//! high-confidence bits (contact info, section blocks) and leaves the rest
//! as free text the user is expected to polish. Not a silver bullet — the
//! generated YAML is a seed, not a final artifact.

use std::sync::OnceLock;

use regex::Regex;

use crate::dates;
use crate::schema::{Education, Experience, Links, Personal, Profile, Project, Skills};

const EMAIL_RE: &str = r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}";
const PHONE_RE: &str = r"\+?[0-9][0-9\s\-()]{7,}[0-9]";
const GITHUB_RE: &str = r"https?://(?:www\.)?github\.com/[A-Za-z0-9_.-]+";
const LINKEDIN_RE: &str = r"https?://(?:www\.)?linkedin\.com/in/[A-Za-z0-9_.-]+/?";

fn compiled(re: &str) -> &'static Regex {
    // Regexes live for the process lifetime; leak rather than clone on every call.
    // The unwrap is safe: all regexes are compile-time literals tested at init.
    #[allow(clippy::unwrap_used)]
    let boxed: Box<Regex> = Box::new(Regex::new(re).unwrap());
    Box::leak(boxed)
}

fn email_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(EMAIL_RE))
}
fn phone_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(PHONE_RE))
}
fn github_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(GITHUB_RE))
}
fn linkedin_re() -> &'static Regex {
    static R: OnceLock<&'static Regex> = OnceLock::new();
    R.get_or_init(|| compiled(LINKEDIN_RE))
}

/// Parse raw text (from a PDF or DOCX extract) into a best-effort [`Profile`].
///
/// The personal block is filled by regex; experience/education/skills come
/// from whatever lines sit under the corresponding section header. Bullets
/// are kept verbatim so the user can tidy them up in `profile.yaml`.
pub fn parse(text: &str) -> Profile {
    let sections = split_sections(text);

    Profile {
        personal: parse_personal(&sections.preamble),
        summary: sections.summary.trim().to_string(),
        skills: parse_skills(&sections.skills),
        experience: parse_experience(&sections.experience),
        education: parse_education(&sections.education),
        projects: parse_projects(&sections.projects),
    }
}

#[derive(Debug, Default)]
struct Sections {
    preamble: String,
    summary: String,
    skills: String,
    experience: String,
    education: String,
    projects: String,
}

fn classify_header(line: &str) -> Option<&'static str> {
    let lower = line.trim().to_lowercase();
    if lower.is_empty() || lower.len() > 40 {
        return None;
    }
    // Only treat short, header-shaped lines as section markers.
    let looks_like_header = lower
        .chars()
        .all(|c| c.is_alphabetic() || c.is_whitespace() || c == '&' || c == '/');
    if !looks_like_header {
        return None;
    }
    match lower.as_str() {
        "summary" | "profile" | "about" | "about me" | "objective" => Some("summary"),
        "skills" | "technical skills" | "core skills" | "core competencies" => Some("skills"),
        "experience"
        | "work experience"
        | "employment"
        | "professional experience"
        | "work history" => Some("experience"),
        "education" | "academics" | "academic background" => Some("education"),
        "projects" | "selected projects" | "personal projects" | "side projects" => {
            Some("projects")
        }
        _ => None,
    }
}

fn split_sections(text: &str) -> Sections {
    let mut out = Sections::default();
    let mut current = "preamble";
    for line in text.lines() {
        if let Some(section) = classify_header(line) {
            current = section;
            continue;
        }
        let target = match current {
            "summary" => &mut out.summary,
            "skills" => &mut out.skills,
            "experience" => &mut out.experience,
            "education" => &mut out.education,
            "projects" => &mut out.projects,
            _ => &mut out.preamble,
        };
        target.push_str(line);
        target.push('\n');
    }
    out
}

fn parse_personal(preamble: &str) -> Personal {
    let email = email_re()
        .find(preamble)
        .map_or(String::new(), |m| m.as_str().to_string());
    let phone = phone_re()
        .find(preamble)
        .map_or(String::new(), |m| m.as_str().trim().to_string());
    let github = github_re()
        .find(preamble)
        .map_or(String::new(), |m| m.as_str().to_string());
    let linkedin = linkedin_re()
        .find(preamble)
        .map_or(String::new(), |m| m.as_str().to_string());

    // Name heuristic: first non-empty line that is neither email/phone/URL.
    let name = preamble
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.contains('@') && !l.starts_with("http") && !has_digit(l))
        .unwrap_or("")
        .to_string();

    Personal {
        name,
        email,
        phone,
        location: String::new(),
        links: Links {
            github,
            linkedin,
            portfolio: String::new(),
        },
    }
}

fn has_digit(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
}

fn parse_skills(text: &str) -> Skills {
    let mut languages = Vec::new();
    for line in text.lines() {
        for token in line.split([',', '|', ';', '·', '•']) {
            let t = token.trim();
            if !t.is_empty() && t.len() < 40 {
                languages.push(t.to_string());
            }
        }
    }
    Skills {
        languages,
        frameworks: Vec::new(),
        tools: Vec::new(),
    }
}

fn parse_experience(text: &str) -> Vec<Experience> {
    // Each experience block is separated by a blank line. Within a block:
    //   line 1: "<title> — <company>" or "<title> at <company>" or "<title>, <company>"
    //   line 2: dates like "Jan 2022 - Present"
    //   remaining lines: bullets (leading - or • stripped)
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let (title, company) = split_title_company(lines[0]);
        let (start, end) = lines.get(1).map(|s| split_dates(s)).unwrap_or_default();
        let bullets: Vec<String> = lines
            .iter()
            .skip(2)
            .map(|l| strip_bullet(l).to_string())
            .collect();
        if title.is_empty() && company.is_empty() && bullets.is_empty() {
            continue;
        }
        out.push(Experience {
            title,
            company,
            start,
            end,
            bullets,
            ..Default::default()
        });
    }
    out
}

fn parse_education(text: &str) -> Vec<Education> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let (degree, institution) = split_title_company(lines[0]);
        let (start, end) = lines.get(1).map(|s| split_dates(s)).unwrap_or_default();
        out.push(Education {
            degree,
            institution,
            start,
            end,
        });
    }
    out
}

fn parse_projects(text: &str) -> Vec<Project> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let lines: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            continue;
        }
        let (name, url) = split_name_url(lines[0]);
        let bullets: Vec<String> = lines
            .iter()
            .skip(1)
            .map(|l| strip_bullet(l).to_string())
            .collect();
        out.push(Project { name, url, bullets });
    }
    out
}

fn split_title_company(line: &str) -> (String, String) {
    for sep in ["—", "–", " - ", " at ", ", ", " @ ", "|"] {
        if let Some(idx) = line.find(sep) {
            let (a, b) = line.split_at(idx);
            return (a.trim().to_string(), b[sep.len()..].trim().to_string());
        }
    }
    (line.trim().to_string(), String::new())
}

fn split_name_url(line: &str) -> (String, String) {
    if let Some(start) = line.find("http") {
        return (
            line[..start]
                .trim_end_matches([' ', '—', '-', '|'])
                .trim()
                .to_string(),
            line[start..].trim().to_string(),
        );
    }
    (line.trim().to_string(), String::new())
}

fn split_dates(line: &str) -> (String, String) {
    for sep in [" - ", " – ", " — ", " to "] {
        if let Some(idx) = line.find(sep) {
            let (a, b) = line.split_at(idx);
            return (dates::normalize(a), dates::normalize_end(&b[sep.len()..]));
        }
    }
    (dates::normalize(line), String::new())
}

fn strip_bullet(line: &str) -> &str {
    let trimmed = line.trim_start();
    for marker in ["-", "•", "·", "*", "–", "—"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return rest.trim_start();
        }
    }
    trimmed
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Alice Kumar
alice.k@example.com
+91-9876543210
https://github.com/alicek
https://www.linkedin.com/in/alicek

SUMMARY
Backend engineer with 6 years of Rust/Python.

SKILLS
Rust, Python, Tokio, SQLx, Linux

EXPERIENCE
Senior Engineer — Acme Robotics
Jan 2022 - Present
- Led migration to async Rust stack.
- Cut p99 latency 40%.

Engineer — BetaCorp
Jun 2019 - Dec 2021
- Built data ingest pipeline.

EDUCATION
B.Tech Computer Science — IIT Delhi
2015 - 2019

PROJECTS
ros2-rust-bridge https://github.com/alicek/ros2-rust-bridge
- ROS2 control plane with async Rust.
";

    #[test]
    fn classifies_section_headers() {
        assert_eq!(classify_header("EXPERIENCE"), Some("experience"));
        assert_eq!(classify_header("  Work Experience  "), Some("experience"));
        assert_eq!(classify_header("Projects"), Some("projects"));
        assert_eq!(classify_header("Not a header"), None);
        assert_eq!(classify_header("alice@example.com"), None);
    }

    #[test]
    fn personal_block_extracts_contact_info() {
        let p = parse(SAMPLE);
        assert_eq!(p.personal.name, "Alice Kumar");
        assert_eq!(p.personal.email, "alice.k@example.com");
        assert_eq!(p.personal.links.github, "https://github.com/alicek");
        assert!(p.personal.phone.starts_with("+91"));
    }

    #[test]
    fn experience_blocks_split_on_blank_lines() {
        let p = parse(SAMPLE);
        assert_eq!(p.experience.len(), 2);
        assert_eq!(p.experience[0].title, "Senior Engineer");
        assert_eq!(p.experience[0].company, "Acme Robotics");
        assert_eq!(p.experience[0].end, "present");
        assert_eq!(p.experience[0].bullets.len(), 2);
        assert!(p.experience[0].bullets[0].starts_with("Led migration"));
    }

    #[test]
    fn education_block_extracted() {
        let p = parse(SAMPLE);
        assert_eq!(p.education.len(), 1);
        assert_eq!(p.education[0].degree, "B.Tech Computer Science");
        assert_eq!(p.education[0].institution, "IIT Delhi");
    }

    #[test]
    fn projects_block_extracted_with_url() {
        let p = parse(SAMPLE);
        assert_eq!(p.projects.len(), 1);
        assert_eq!(p.projects[0].name, "ros2-rust-bridge");
        assert!(p.projects[0].url.starts_with("https://github.com"));
    }

    #[test]
    fn summary_line_trimmed() {
        let p = parse(SAMPLE);
        assert!(p.summary.contains("Backend engineer"));
    }
}
