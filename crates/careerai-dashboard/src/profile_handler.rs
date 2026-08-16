//! Candidate Profile & Keyword Manager Handlers for Career-AI Dashboard.

use std::path::PathBuf;
use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

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

pub async fn api_profile_save(
    Json(payload): Json<SaveProfileRequest>,
) -> impl IntoResponse {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("cwd error: {e}") })),
            )
                .into_response();
        }
    };

    let profile_dir = cwd.join("profile");
    let profile_path = profile_dir.join("profile.yaml");

    let mut profile = if profile_path.exists() {
        if let Ok(raw) = std::fs::read_to_string(&profile_path) {
            serde_yaml::from_str::<careerai_profile::schema::Profile>(&raw).unwrap_or_default()
        } else {
            careerai_profile::schema::Profile::default()
        }
    } else {
        careerai_profile::schema::Profile::default()
    };

    profile.personal.name = payload.name;
    profile.personal.email = payload.email;
    profile.personal.phone = payload.phone;
    profile.personal.location = payload.location;
    if !payload.target_roles.is_empty() {
        profile.summary = format!("Target Roles: {}", payload.target_roles.join(", "));
    }
    profile.skills.languages = payload.languages;
    profile.skills.frameworks = payload.frameworks;
    profile.skills.tools = payload.tools;

    let _ = std::fs::create_dir_all(&profile_dir);
    match serde_yaml::to_string(&profile) {
        Ok(yaml_str) => match std::fs::write(&profile_path, &yaml_str) {
            Ok(_) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "success",
                    "message": format!("Profile saved to {}", profile_path.display()),
                })),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("Failed to write profile: {e}") })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("Failed to serialize profile: {e}") })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct KeywordToggleRequest {
    pub keyword: String,
    pub action: String, // "add", "remove", "toggle"
}

pub async fn api_config_keywords(
    Json(payload): Json<KeywordToggleRequest>,
) -> impl IntoResponse {
    let kw = payload.keyword.trim();
    if kw.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Keyword cannot be empty" })),
        )
            .into_response();
    }

    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("cwd error: {e}") })),
            )
                .into_response();
        }
    };

    let config_path = cwd.join("config").join("local.yaml");
    let mut content = if config_path.exists() {
        std::fs::read_to_string(&config_path).unwrap_or_default()
    } else {
        "version: \"1.0\"\nsources:\n  keywords:\n".to_string()
    };

    let entry = format!("    - \"{kw}\"");
    let mut modified = false;

    match payload.action.as_str() {
        "add" | "toggle" if !content.contains(&entry) => {
            if let Some(pos) = content.find("keywords:") {
                let insert_at = pos + "keywords:\n".len();
                content.insert_str(insert_at, &format!("{entry}\n"));
                modified = true;
            }
        }
        "remove" | "toggle" if content.contains(&entry) => {
            content = content.replace(&format!("{entry}\n"), "");
            modified = true;
        }
        _ => {}
    }

    if modified {
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&config_path, &content) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("write failed: {e}") })),
            )
                .into_response();
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "keyword": kw,
            "modified": modified,
        })),
    )
        .into_response()
}

pub fn get_profile_file_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("profile")
        .join("profile.yaml")
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

pub fn load_keywords_from_config(
    cfg: Option<&careerai_core::config::CoreConfig>,
) -> Vec<crate::view::KeywordStatus> {
    let keywords = cfg
        .map(|c| c.sources.keywords.clone())
        .unwrap_or_else(|| {
            vec![
                "Embedded Systems".into(),
                "Rust".into(),
                "AI/ML".into(),
                "Robotics".into(),
            ]
        });
    keywords
        .into_iter()
        .map(|k| crate::view::KeywordStatus {
            name: k,
            enabled: true,
        })
        .collect()
}
