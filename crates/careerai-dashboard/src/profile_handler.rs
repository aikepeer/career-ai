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
    let mut keywords = cfg
        .map(|c| c.sources.keywords.clone())
        .unwrap_or_else(|| {
            vec![
                "AI/ML".into(),
                "Embedded Systems".into(),
                "Robotics".into(),
                "Rust".into(),
            ]
        });
    keywords.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    keywords
        .into_iter()
        .map(|k| crate::view::KeywordStatus {
            name: k,
            enabled: true,
        })
        .collect()
}

pub fn build_sources_list(
    db_rows: Vec<(String, i64, Option<chrono::DateTime<chrono::Utc>>)>,
) -> Vec<crate::view::ConfigSourceItem> {
    let known_sources = [
        ("greenhouse", "ATS Direct Feed", "https://boards.greenhouse.io"),
        ("lever", "ATS Direct Feed", "https://jobs.lever.co"),
        ("ashby", "ATS Direct Feed", "https://jobs.ashbyhq.com"),
        ("workday", "ATS Direct Feed", "https://myworkdayjobs.com"),
        ("smartrecruiters", "ATS Direct Feed", "https://careers.smartrecruiters.com"),
        ("linkedin", "Web Scraper / CDP", "https://www.linkedin.com/jobs"),
        ("indeed", "Job Board", "https://www.indeed.com"),
        ("naukri", "India Job Portal", "https://www.naukri.com"),
        ("remotive", "Remote Jobs API", "https://remotive.com"),
        ("wellfound", "Startup Tech Jobs", "https://wellfound.com/jobs"),
        ("weworkremotely", "Remote Community", "https://weworkremotely.com"),
        ("ycombinator", "YC Startups", "https://www.workatastartup.com"),
        ("upwork", "Freelance Platform", "https://www.upwork.com"),
        ("freelancer", "Freelance Platform", "https://www.freelancer.com"),
        ("toptal", "Elite Freelance", "https://www.toptal.com"),
        ("remoteok", "Remote Tech Board", "https://remoteok.com"),
        ("otta", "Curated Tech Jobs", "https://otta.com"),
    ];

    let mut db_map = std::collections::HashMap::new();
    for (name, count, last_sync) in db_rows {
        if !name.is_empty() {
            db_map.insert(name.to_lowercase(), (count, last_sync));
        }
    }

    let mut sources = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (s_name, s_kind, s_url) in known_sources {
        let name_lower = s_name.to_lowercase();
        seen.insert(name_lower.clone());
        let (count_i, last_sync) = db_map.get(&name_lower).copied().unwrap_or((0, None));
        let sync_label = last_sync.map_or_else(|| "Ready".into(), |ts| {
            let delta = chrono::Utc::now().signed_duration_since(ts);
            if delta.num_hours() < 1 {
                "just now".into()
            } else if delta.num_hours() < 24 {
                format!("{}h ago", delta.num_hours())
            } else {
                format!("{}d ago", delta.num_days())
            }
        });
        let status = if count_i > 0 { "active" } else { "ready" }.to_string();

        sources.push(crate::view::ConfigSourceItem {
            name: s_name.to_string(),
            kind: s_kind.to_string(),
            listing_count: u64::try_from(count_i.max(0)).unwrap_or(0),
            last_sync,
            last_sync_label: sync_label,
            status,
            url: Some(s_url.to_string()),
        });
    }

    for (name, (count_i, last_sync)) in db_map {
        if !seen.contains(&name) {
            sources.push(crate::view::ConfigSourceItem {
                name: name.clone(),
                kind: "Custom Portal".into(),
                listing_count: u64::try_from(count_i.max(0)).unwrap_or(0),
                last_sync,
                last_sync_label: "active".into(),
                status: "active".into(),
                url: None,
            });
        }
    }

    sources
}

pub async fn api_pipeline_discover() -> impl IntoResponse {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("careerai"));
    match tokio::process::Command::new(&exe).arg("discover").output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let out_text = format!("{stdout}\n{stderr}").trim().to_string();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": if out.status.success() { "success" } else { "failed" },
                    "message": out_text,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "error": e.to_string() })),
        ),
    }
}

pub async fn api_pipeline_match() -> impl IntoResponse {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("careerai"));
    match tokio::process::Command::new(&exe).arg("match").output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let out_text = format!("{stdout}\n{stderr}").trim().to_string();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": if out.status.success() { "success" } else { "failed" },
                    "message": out_text,
                })),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "status": "error", "error": e.to_string() })),
        ),
    }
}
