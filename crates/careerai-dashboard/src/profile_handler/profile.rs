//! Profile load/save/draft/import-confirm handlers.

use axum::{http::StatusCode, response::IntoResponse, Json};
use careerai_profile::schema::Profile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::util::{backup_file, unique_tmp_path};

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct SaveProfileRequest {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub location: String,
    #[serde(default)]
    pub github: String,
    #[serde(default)]
    pub linkedin: String,
    #[serde(default)]
    pub portfolio: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub target_roles: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub frameworks: Vec<String>,
    #[serde(default)]
    pub devops: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub debugging: Vec<String>,
    #[serde(default)]
    pub protocols: Vec<String>,
    #[serde(default)]
    pub raw_yaml: Option<String>,
}

pub async fn api_profile_save(Json(payload): Json<SaveProfileRequest>) -> impl IntoResponse {
    match save_profile_edits(payload).await {
        Ok(path) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!("Profile saved to {}", path.display()),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn save_profile_edits(payload: SaveProfileRequest) -> Result<PathBuf, String> {
    if let Some(raw) = payload.raw_yaml.filter(|s| !s.trim().is_empty()) {
        let profile = Profile::from_yaml(&raw).map_err(|e| format!("invalid YAML: {e}"))?;
        return save_profile_to_disk(&profile).await;
    }
    let mut profile = load_profile_for_edit().await?;
    profile.personal.name = payload.name;
    profile.personal.email = payload.email;
    profile.personal.phone = payload.phone;
    profile.personal.location = payload.location;
    profile.personal.links.github = payload.github;
    profile.personal.links.linkedin = payload.linkedin;
    profile.personal.links.portfolio = payload.portfolio;
    if !payload.summary.trim().is_empty() {
        profile.summary = payload.summary;
    }
    if !payload.target_roles.is_empty() {
        profile.target_roles = payload.target_roles;
    }
    profile.skills.languages = payload.languages;
    profile.skills.platforms = payload.platforms;
    profile.skills.frameworks = payload.frameworks;
    profile.skills.devops = payload.devops;
    profile.skills.tools = payload.tools;
    profile.skills.debugging = payload.debugging;
    profile.skills.protocols = payload.protocols;
    save_profile_to_disk(&profile).await
}

/// Load the existing `profile.yaml` for editing. Returns `Profile::default()`
/// when the file does not exist, and fails loudly when it exists but cannot
/// be read or parsed — silently replacing an unparseable profile with a
/// default one would destroy the user's data.
async fn load_profile_for_edit() -> Result<Profile, String> {
    let path = get_profile_file_path();
    if !path.exists() {
        return Ok(Profile::default());
    }
    let raw = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_yaml::from_str::<Profile>(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Persist a full [`Profile`] to `profile/profile.yaml` atomically (write a
/// temp file and rename it), preserving existing files as backup before overwriting.
pub async fn save_profile_to_disk(profile: &Profile) -> Result<PathBuf, String> {
    save_profile_to_disk_at(profile, &get_profile_file_path()).await
}

#[cfg(unix)]
fn fix_sudo_ownership(path: &Path) {
    if let (Ok(uid_s), Ok(gid_s)) = (std::env::var("SUDO_UID"), std::env::var("SUDO_GID")) {
        if let (Ok(uid), Ok(gid)) = (uid_s.parse::<u32>(), gid_s.parse::<u32>()) {
            let _ = std::process::Command::new("chown")
                .arg(format!("{uid}:{gid}"))
                .arg(path)
                .status();
        }
    }
}

/// Write a profile YAML document atomically to an arbitrary path, creating
/// parent directory and backing up any existing target file first.
pub async fn save_profile_to_disk_at(profile: &Profile, path: &Path) -> Result<PathBuf, String> {
    let dir = path
        .parent()
        .ok_or_else(|| "profile path has no parent directory".to_string())?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("create profile dir: {e}"))?;
    #[cfg(unix)]
    fix_sudo_ownership(dir);
    if path.exists() {
        backup_file(path)?;
        #[cfg(unix)]
        if let Some(bak) = path.parent().map(|p| p.join("profile.yaml.bak")) {
            fix_sudo_ownership(&bak);
        }
    }
    let yaml = serde_yaml::to_string(profile).map_err(|e| format!("serialize profile: {e}"))?;
    let tmp = unique_tmp_path(path);
    tokio::fs::write(&tmp, &yaml)
        .await
        .map_err(|e| format!("write profile: {e}"))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("commit profile: {e}"))?;
    #[cfg(unix)]
    fix_sudo_ownership(path);
    Ok(path.to_path_buf())
}

/// Persist an imported profile to a *draft* file instead of overwriting
/// `profile/profile.yaml`. The operator reviews the extracted fields and
/// then confirms via `/api/v1/profile/import/confirm`, which is what
/// actually replaces the live profile.
pub async fn save_profile_draft(profile: &Profile) -> Result<PathBuf, String> {
    save_profile_at(profile, &get_profile_draft_path()).await
}

/// Write a profile YAML document atomically to an arbitrary path (used by
/// the draft flow and directly by tests).
async fn save_profile_at(profile: &Profile, path: &Path) -> Result<PathBuf, String> {
    let dir = path
        .parent()
        .ok_or_else(|| "profile path has no parent directory".to_string())?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("create profile dir: {e}"))?;
    #[cfg(unix)]
    fix_sudo_ownership(dir);
    let yaml = serde_yaml::to_string(profile).map_err(|e| format!("serialize profile: {e}"))?;
    let tmp = unique_tmp_path(path);
    tokio::fs::write(&tmp, &yaml)
        .await
        .map_err(|e| format!("write profile draft: {e}"))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("commit profile draft: {e}"))?;
    #[cfg(unix)]
    fix_sudo_ownership(path);
    Ok(path.to_path_buf())
}

/// Promote the pending profile draft to `profile/profile.yaml`, backing up
/// the previous profile first. Returns the live profile path.
pub fn confirm_profile_import() -> Result<PathBuf, String> {
    confirm_profile_import_at(&get_profile_draft_path(), &get_profile_file_path())
}

/// Testable core of [`confirm_profile_import`]: promote `draft` to
/// `target`, backing up any existing `target` first.
fn confirm_profile_import_at(draft: &Path, target: &Path) -> Result<PathBuf, String> {
    if !draft.exists() {
        return Err("no pending profile import to confirm".to_string());
    }
    if target.exists() {
        backup_file(target)?;
    }
    std::fs::rename(draft, target).map_err(|e| format!("apply profile import: {e}"))?;
    #[cfg(unix)]
    fix_sudo_ownership(target);
    Ok(target.to_path_buf())
}

pub fn get_profile_file_path() -> PathBuf {
    let root = careerai_core::paths::resolve_root_env();
    careerai_core::paths::profile_path(&root)
}

pub fn get_profile_draft_path() -> PathBuf {
    let root = careerai_core::paths::resolve_root_env();
    careerai_core::paths::profile_draft_path(&root)
}

pub fn load_profile_view() -> Option<crate::view::ProfileView> {
    load_profile_view_at(&get_profile_file_path())
}

pub fn load_profile_view_at(path: &Path) -> Option<crate::view::ProfileView> {
    if !path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let p = serde_yaml::from_str::<careerai_profile::schema::Profile>(&raw).ok()?;
    let target_roles: Vec<String> = if !p.target_roles.is_empty() {
        p.target_roles.clone()
    } else if p.summary.starts_with("Target Roles:") {
        p.summary["Target Roles:".len()..]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        p.experience.iter().map(|e| e.title.clone()).collect()
    };

    let skill_count = p.skills.all_skill_names().count();
    let mut career_story = Vec::with_capacity(p.education.len() + p.experience.len());
    for education in &p.education {
        let proof = education
            .achievements
            .first()
            .or_else(|| education.projects.first())
            .cloned()
            .unwrap_or_else(|| "Built the technical foundation for the journey ahead.".to_string());
        career_story.push(crate::view::CareerStoryItem {
            kind: "education".to_string(),
            era: format_era(&education.start, &education.end),
            title: education.degree.clone(),
            organization: education.institution.clone(),
            proof,
        });
    }
    for experience in &p.experience {
        career_story.push(crate::view::CareerStoryItem {
            kind: "experience".to_string(),
            era: format_era(&experience.start, &experience.end),
            title: experience.title.clone(),
            organization: experience.company.clone(),
            proof: experience.bullets.first().map_or_else(
                || "Expanded scope, impact, and technical depth.".to_string(),
                |bullet| truncate_story_proof(bullet, 156),
            ),
        });
    }
    career_story.sort_by(|a, b| a.era.cmp(&b.era));

    Some(crate::view::ProfileView {
        name: p.personal.name,
        email: p.personal.email,
        phone: p.personal.phone,
        location: p.personal.location,
        github: p.personal.links.github,
        linkedin: p.personal.links.linkedin,
        portfolio: p.personal.links.portfolio,
        summary: p.summary,
        target_roles,
        languages: p.skills.languages,
        platforms: p.skills.platforms,
        frameworks: p.skills.frameworks,
        devops: p.skills.devops,
        tools: p.skills.tools,
        debugging: p.skills.debugging,
        protocols: p.skills.protocols,
        skill_count,
        experience_count: p.experience.len(),
        education_count: p.education.len(),
        career_story,
        raw_yaml: raw,
    })
}

fn format_era(start: &str, end: &str) -> String {
    match (start.trim(), end.trim()) {
        ("", "") => "Milestone".to_string(),
        (start, "") => start.to_string(),
        ("", end) => end.to_string(),
        (start, end) => format!("{start} — {end}"),
    }
}

fn truncate_story_proof(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "profile_tests.rs"]
mod tests;
