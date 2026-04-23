//! LinkedIn "Download your data" ZIP parser.
//!
//! The export ships a folder of CSVs. We read only the files relevant to a
//! resume seed: `Profile.csv`, `Positions.csv`, `Education.csv`, `Skills.csv`,
//! and optionally `Projects.csv`. Missing optional files are tolerated;
//! `Profile.csv` is required.

use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use serde::Deserialize;

use crate::error::{ProfileError, Result};
use crate::schema::{Education, Experience, Links, Personal, Profile, Project, Skills};

/// Load a [`Profile`] from a LinkedIn export ZIP on disk.
pub fn parse_export_from_path(path: &Path) -> Result<Profile> {
    let file = File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    parse_archive(&mut archive)
}

/// Load a [`Profile`] from LinkedIn export bytes (tests).
pub fn parse_export_from_bytes(bytes: &[u8]) -> Result<Profile> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    parse_archive(&mut archive)
}

fn parse_archive<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Profile> {
    let profile_row = read_profile_csv(archive)?;
    let positions = read_positions(archive).unwrap_or_default();
    let education = read_education(archive).unwrap_or_default();
    let skills = read_skills(archive).unwrap_or_default();
    let projects = read_projects(archive).unwrap_or_default();

    Ok(Profile {
        personal: Personal {
            name: join_name(&profile_row.first_name, &profile_row.last_name),
            email: profile_row.email.unwrap_or_default(),
            phone: String::new(),
            location: profile_row.location.unwrap_or_default(),
            links: Links {
                linkedin: String::new(),
                github: String::new(),
                portfolio: profile_row.websites.unwrap_or_default(),
            },
        },
        summary: profile_row.summary.unwrap_or_default(),
        skills: Skills {
            languages: skills,
            frameworks: Vec::new(),
            tools: Vec::new(),
        },
        experience: positions,
        education,
        projects,
    })
}

fn read_csv<R: Read + Seek, T: for<'de> Deserialize<'de>>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<T>> {
    let entry = archive
        .by_name(name)
        .map_err(|_| ProfileError::LinkedInMissingFile(name.to_string()))?;
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(entry);
    let mut out = Vec::new();
    for row in rdr.deserialize() {
        out.push(row?);
    }
    Ok(out)
}

fn try_read_csv<R: Read + Seek, T: for<'de> Deserialize<'de>>(
    archive: &mut zip::ZipArchive<R>,
    candidates: &[&str],
) -> Result<Vec<T>> {
    for name in candidates {
        match read_csv::<_, T>(archive, name) {
            Ok(rows) => return Ok(rows),
            Err(ProfileError::LinkedInMissingFile(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Err(ProfileError::LinkedInMissingFile(candidates.join(", ")))
}

#[derive(Debug, Deserialize, Default)]
struct ProfileCsvRow {
    #[serde(rename = "First Name", default)]
    first_name: String,
    #[serde(rename = "Last Name", default)]
    last_name: String,
    #[serde(rename = "Email Address", default)]
    email: Option<String>,
    #[serde(rename = "Geo Location", default)]
    location: Option<String>,
    #[serde(rename = "Summary", default)]
    summary: Option<String>,
    #[serde(rename = "Websites", default)]
    websites: Option<String>,
}

fn read_profile_csv<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<ProfileCsvRow> {
    let rows: Vec<ProfileCsvRow> = try_read_csv(archive, &["Profile.csv"])?;
    rows.into_iter()
        .next()
        .ok_or_else(|| ProfileError::LinkedInMissingFile("Profile.csv (empty)".to_string()))
}

#[derive(Debug, Deserialize)]
struct PositionCsvRow {
    #[serde(rename = "Company Name", default)]
    company_name: String,
    #[serde(rename = "Title", default)]
    title: String,
    #[serde(rename = "Description", default)]
    description: String,
    #[serde(rename = "Location", default)]
    location: String,
    #[serde(rename = "Started On", default)]
    started_on: String,
    #[serde(rename = "Finished On", default)]
    finished_on: String,
}

fn read_positions<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Vec<Experience>> {
    let rows: Vec<PositionCsvRow> = try_read_csv(archive, &["Positions.csv"])?;
    Ok(rows
        .into_iter()
        .map(|r| Experience {
            title: r.title,
            company: r.company_name,
            location: r.location,
            start: normalize_linkedin_date(&r.started_on),
            end: if r.finished_on.trim().is_empty() {
                "present".to_string()
            } else {
                normalize_linkedin_date(&r.finished_on)
            },
            bullets: split_description(&r.description),
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct EducationCsvRow {
    #[serde(rename = "School Name", default)]
    school_name: String,
    #[serde(rename = "Degree Name", default)]
    degree_name: String,
    #[serde(rename = "Start Date", default)]
    start_date: String,
    #[serde(rename = "End Date", default)]
    end_date: String,
}

fn read_education<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Vec<Education>> {
    let rows: Vec<EducationCsvRow> = try_read_csv(archive, &["Education.csv"])?;
    Ok(rows
        .into_iter()
        .map(|r| Education {
            degree: r.degree_name,
            institution: r.school_name,
            start: normalize_linkedin_date(&r.start_date),
            end: normalize_linkedin_date(&r.end_date),
        })
        .collect())
}

#[derive(Debug, Deserialize)]
struct SkillCsvRow {
    #[serde(rename = "Name")]
    name: String,
}

fn read_skills<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Vec<String>> {
    let rows: Vec<SkillCsvRow> = try_read_csv(archive, &["Skills.csv"])?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

#[derive(Debug, Deserialize)]
struct ProjectCsvRow {
    #[serde(rename = "Title", default)]
    title: String,
    #[serde(rename = "Description", default)]
    description: String,
    #[serde(rename = "Url", default)]
    url: String,
}

fn read_projects<R: Read + Seek>(archive: &mut zip::ZipArchive<R>) -> Result<Vec<Project>> {
    let rows: Vec<ProjectCsvRow> = try_read_csv(archive, &["Projects.csv"])?;
    Ok(rows
        .into_iter()
        .map(|r| Project {
            name: r.title,
            url: r.url,
            bullets: split_description(&r.description),
        })
        .collect())
}

fn join_name(first: &str, last: &str) -> String {
    let full = format!("{} {}", first.trim(), last.trim());
    full.trim().to_string()
}

fn split_description(text: &str) -> Vec<String> {
    text.split(['\n', '\r'])
        .map(|s| {
            s.trim_start_matches(['-', '•', '·', '*'])
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// LinkedIn exports dates as `"Feb 2022"`, `"2020"`, or empty. Normalize to
/// `YYYY-MM` / `YYYY` / `""` so downstream code has a predictable format.
fn normalize_linkedin_date(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    match parts.as_slice() {
        [year] if year.len() == 4 => (*year).to_string(),
        [month, year] if year.len() == 4 => {
            if let Some(mm) = month_to_number(month) {
                format!("{year}-{mm}")
            } else {
                (*year).to_string()
            }
        }
        _ => trimmed.to_string(),
    }
}

fn month_to_number(month: &str) -> Option<&'static str> {
    let lower = month.to_lowercase();
    match lower.get(..3)? {
        "jan" => Some("01"),
        "feb" => Some("02"),
        "mar" => Some("03"),
        "apr" => Some("04"),
        "may" => Some("05"),
        "jun" => Some("06"),
        "jul" => Some("07"),
        "aug" => Some("08"),
        "sep" => Some("09"),
        "oct" => Some("10"),
        "nov" => Some("11"),
        "dec" => Some("12"),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn build_export(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, contents) in files {
                zw.start_file(*name, opts).unwrap();
                zw.write_all(contents.as_bytes()).unwrap();
            }
            zw.finish().unwrap();
        }
        buf
    }

    #[test]
    fn normalizes_linkedin_dates() {
        assert_eq!(normalize_linkedin_date("Feb 2022"), "2022-02");
        assert_eq!(normalize_linkedin_date("2020"), "2020");
        assert_eq!(normalize_linkedin_date(""), "");
        assert_eq!(normalize_linkedin_date("December 2019"), "2019-12");
    }

    #[test]
    fn parses_minimal_export() {
        let profile_csv = "First Name,Last Name,Email Address,Summary,Geo Location\nAlice,Kumar,a@example.com,Builder of systems,Delhi NCR\n";
        let positions_csv = "Company Name,Title,Description,Location,Started On,Finished On\nAcme,Senior Engineer,Led migration\\nCut latency,Remote,Jan 2022,\n";
        let skills_csv = "Name\nRust\nPython\n";

        let bytes = build_export(&[
            ("Profile.csv", profile_csv),
            ("Positions.csv", positions_csv),
            ("Skills.csv", skills_csv),
        ]);

        let p = parse_export_from_bytes(&bytes).unwrap();
        assert_eq!(p.personal.name, "Alice Kumar");
        assert_eq!(p.personal.email, "a@example.com");
        assert_eq!(p.experience.len(), 1);
        assert_eq!(p.experience[0].start, "2022-01");
        assert_eq!(p.experience[0].end, "present");
        assert_eq!(p.skills.languages, vec!["Rust", "Python"]);
    }

    #[test]
    fn missing_profile_csv_is_an_error() {
        let bytes = build_export(&[("Skills.csv", "Name\nRust\n")]);
        assert!(matches!(
            parse_export_from_bytes(&bytes),
            Err(ProfileError::LinkedInMissingFile(_))
        ));
    }
}
