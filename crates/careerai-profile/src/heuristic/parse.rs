use crate::schema::{Education, Experience, Links, Personal, Profile, Project, Skills};

use super::regex::{email_re, github_re, linkedin_re, phone_re};
use super::text::{split_dates, split_name_url, split_title_company, strip_bullet};

/// Parse raw text (from a PDF or DOCX extract) into a best-effort [`Profile`].
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

pub(crate) fn classify_header(line: &str) -> Option<&'static str> {
    let lower = line.trim().to_lowercase();
    if lower.is_empty() || lower.len() > 40 {
        return None;
    }
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
