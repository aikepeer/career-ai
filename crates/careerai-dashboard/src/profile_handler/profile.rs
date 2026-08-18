//! Profile load/save/draft/import-confirm handlers.

use axum::{http::StatusCode, response::IntoResponse, Json};
use careerai_profile::schema::Profile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::util::{backup_file, unique_tmp_path};

#[derive(Debug, Deserialize, Serialize)]
pub struct SaveProfileRequest {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub location: String,
    pub target_roles: Vec<String>,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub tools: Vec<String>,
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
    let mut profile = load_profile_for_edit().await?;
    profile.personal.name = payload.name;
    profile.personal.email = payload.email;
    profile.personal.phone = payload.phone;
    profile.personal.location = payload.location;
    // Only populate summary when it's empty: an imported/hand-written
    // summary must not be clobbered on every save. (A dedicated
    // `target_roles` field would be cleaner than overloading `summary`.)
    if !payload.target_roles.is_empty() && profile.summary.is_empty() {
        profile.summary = format!("Target Roles: {}", payload.target_roles.join(", "));
    }
    profile.skills.languages = payload.languages;
    profile.skills.frameworks = payload.frameworks;
    profile.skills.tools = payload.tools;
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
/// temp file and rename it), so experience / education / projects and any
/// other fields not surfaced by the dashboard form are preserved on save.
pub async fn save_profile_to_disk(profile: &Profile) -> Result<PathBuf, String> {
    let path = get_profile_file_path();
    let dir = path
        .parent()
        .ok_or_else(|| "profile path has no parent directory".to_string())?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("create profile dir: {e}"))?;
    let yaml = serde_yaml::to_string(profile).map_err(|e| format!("serialize profile: {e}"))?;
    let tmp = unique_tmp_path(&path);
    tokio::fs::write(&tmp, &yaml)
        .await
        .map_err(|e| format!("write profile: {e}"))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("commit profile: {e}"))?;
    Ok(path)
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
    let yaml = serde_yaml::to_string(profile).map_err(|e| format!("serialize profile: {e}"))?;
    let tmp = unique_tmp_path(path);
    tokio::fs::write(&tmp, &yaml)
        .await
        .map_err(|e| format!("write profile draft: {e}"))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("commit profile draft: {e}"))?;
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
    Ok(target.to_path_buf())
}

pub fn get_profile_file_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("profile")
        .join("profile.yaml")
}

pub fn get_profile_draft_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("profile")
        .join("profile.draft.yaml")
}

pub fn load_profile_view() -> Option<crate::view::ProfileView> {
    let path = get_profile_file_path();
    if !path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(&path).ok()?;
    let p = serde_yaml::from_str::<careerai_profile::schema::Profile>(&raw).ok()?;
    let target_roles: Vec<String> = if p.summary.starts_with("Target Roles:") {
        p.summary["Target Roles:".len()..]
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        p.experience.iter().map(|e| e.title.clone()).collect()
    };
    Some(crate::view::ProfileView {
        name: p.personal.name,
        email: p.personal.email,
        phone: p.personal.phone,
        location: p.personal.location,
        target_roles,
        languages: p.skills.languages,
        frameworks: p.skills.frameworks,
        tools: p.skills.tools,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn confirm_profile_import_at_promotes_draft_and_backs_up_target() {
        let tmp = tempfile::tempdir().unwrap();
        let draft = tmp.path().join("profile.draft.yaml");
        let target = tmp.path().join("profile.yaml");
        std::fs::write(&draft, "name: Draft\n").unwrap();
        std::fs::write(&target, "name: Previous\n").unwrap();

        let applied = confirm_profile_import_at(&draft, &target).unwrap();

        assert_eq!(applied, target);
        assert!(!draft.exists(), "draft should be consumed");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "name: Draft\n");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("profile.yaml.bak")).unwrap(),
            "name: Previous\n"
        );
    }

    #[test]
    fn confirm_profile_import_at_missing_draft_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let draft = tmp.path().join("profile.draft.yaml");
        let target = tmp.path().join("profile.yaml");
        std::fs::write(&target, "name: Previous\n").unwrap();

        let err = confirm_profile_import_at(&draft, &target).unwrap_err();
        assert!(err.contains("no pending profile import"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "name: Previous\n"
        );
    }

    #[test]
    fn confirm_profile_import_at_creates_target_without_backup_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let draft = tmp.path().join("profile.draft.yaml");
        let target = tmp.path().join("profile.yaml");
        std::fs::write(&draft, "name: Draft\n").unwrap();

        confirm_profile_import_at(&draft, &target).unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "name: Draft\n");
        assert!(!tmp.path().join("profile.yaml.bak").exists());
    }

    #[tokio::test]
    async fn save_profile_at_writes_atomically_and_creates_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("profile.yaml");
        let mut profile = Profile::default();
        profile.personal.name = "Alice".to_string();

        let written = save_profile_at(&profile, &path).await.unwrap();

        assert_eq!(written, path);
        let parsed: Profile =
            serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed.personal.name, "Alice");
    }
}
