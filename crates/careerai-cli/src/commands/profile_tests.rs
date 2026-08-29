//! Tests for careerai profile commands.

#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn detect_stale_skills_flags_legacy_flat_list() {
    let yaml = "personal:\n  name: Alice\nskills:\n  - Rust\n  - Python\n";
    assert!(detect_stale_skills_schema(yaml));
}

#[test]
fn detect_stale_skills_passes_current_mapping_shape() {
    let yaml = "personal:\n  name: Alice\nskills:\n  languages:\n    - Rust\n";
    assert!(!detect_stale_skills_schema(yaml));
}

#[test]
fn detect_stale_skills_handles_missing_skills_block() {
    let yaml = "personal:\n  name: Alice\nsummary: hi\n";
    assert!(!detect_stale_skills_schema(yaml));
}

#[test]
fn detect_stale_skills_ignores_blank_lines_and_comments() {
    let yaml = "personal:\n  name: Alice\nskills:\n\n  # a comment\n  languages:\n    - Rust\n";
    assert!(!detect_stale_skills_schema(yaml));
}

fn personal_dir(home: &Path) -> PathBuf {
    let dir = home.join("Documents").join("personal");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn touch(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), b"resume").unwrap();
}

#[test]
fn default_resume_sources_picks_canonical_resume_first() {
    let home = tempfile::tempdir().unwrap();
    let dir = personal_dir(home.path());
    touch(&dir, "Resume-Kamal-Pandey-Honeywell.pdf");
    touch(&dir, "Resume-Kamal-Pandey.pdf");
    touch(&dir, "electricity-bill.pdf");
    let sources = default_resume_sources(home.path()).unwrap();
    let names: Vec<String> = sources
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    assert_eq!(
        names,
        vec![
            "Resume-Kamal-Pandey.pdf",
            "Resume-Kamal-Pandey-Honeywell.pdf"
        ]
    );
}

#[test]
fn default_resume_sources_falls_back_to_other_resume_files() {
    let home = tempfile::tempdir().unwrap();
    let dir = personal_dir(home.path());
    touch(&dir, "Resume-Kamal-Pandey-Honeywell.pdf");
    touch(&dir, "DHBVN.pdf");
    let sources = default_resume_sources(home.path()).unwrap();
    let names: Vec<String> = sources
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    assert_eq!(names, vec!["Resume-Kamal-Pandey-Honeywell.pdf"]);
}

#[test]
fn default_resume_sources_accepts_docx_resumes() {
    let home = tempfile::tempdir().unwrap();
    let dir = personal_dir(home.path());
    touch(&dir, "Resume-Kamal-Pandey.docx");
    let sources = default_resume_sources(home.path()).unwrap();
    assert_eq!(sources.len(), 1);
    assert!(sources[0].ends_with("Resume-Kamal-Pandey.docx"));
}

#[test]
fn default_resume_sources_errors_when_dir_missing() {
    let home = tempfile::tempdir().unwrap();
    let err = default_resume_sources(home.path()).unwrap_err();
    assert!(format!("{err:#}").contains("Documents/personal"));
}

#[test]
fn default_resume_sources_errors_when_no_resume_found() {
    let home = tempfile::tempdir().unwrap();
    let dir = personal_dir(home.path());
    touch(&dir, "DHBVN.pdf");
    touch(&dir, "electricity-bill.pdf");
    let err = default_resume_sources(home.path()).unwrap_err();
    assert!(format!("{err:#}").contains("Documents/personal"));
}

#[test]
fn profile_yaml_path_resolves_profile_yaml_under_root() {
    let p = profile_yaml_path();
    assert!(p.ends_with("profile/profile.yaml"));
}
