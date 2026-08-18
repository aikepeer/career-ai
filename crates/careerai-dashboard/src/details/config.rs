//! Config-view assembly from DB + layered config.

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::error::Result;
use crate::view::ConfigView;

pub async fn fetch_config_view(pool: &SqlitePool) -> Result<ConfigView> {
    let rows = sqlx::query(
        "SELECT source, COUNT(*) as n, MAX(created_at) as last_sync FROM listings GROUP BY source",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut db_rows = Vec::new();
    for row in rows {
        let name: String = row.try_get("source").unwrap_or_default();
        let count_i: i64 = row.try_get("n").unwrap_or(0);
        let last_sync: Option<DateTime<Utc>> = row.try_get("last_sync").ok();
        db_rows.push((name, count_i, last_sync));
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let core_cfg = careerai_core::config::CoreConfig::load(&cwd).ok();
    let sources = crate::profile_handler::build_sources_list(db_rows, core_cfg.as_ref());

    let llm_provider = core_cfg.as_ref().map_or_else(
        || "auto".to_string(),
        |c| c.llm.backend.as_str().to_string(),
    );

    let llm_model = core_cfg.as_ref().map_or_else(
        || "claude-3-5-sonnet".to_string(),
        |c| {
            if !c.llm.tailor_model.is_empty() {
                c.llm.tailor_model.clone()
            } else if !c.llm.parse_resume_model.is_empty() {
                c.llm.parse_resume_model.clone()
            } else {
                "default".to_string()
            }
        },
    );

    let prompt_ver = core_cfg
        .as_ref()
        .map_or_else(|| "v1.2.0".to_string(), |c| c.llm.prompt_version.clone());

    let llm_backend = core_cfg.as_ref().map_or_else(
        || "auto".to_string(),
        |c| c.llm.backend.as_str().to_string(),
    );

    let llm_api_base = core_cfg.as_ref().and_then(|c| c.llm.api_base_url.clone());

    let llm_timeout_seconds = core_cfg.as_ref().map_or(300, |c| c.llm.timeout_seconds);

    let profile = crate::profile_handler::load_profile_view();
    let keywords = crate::profile_handler::load_keywords_from_config(core_cfg.as_ref());

    Ok(ConfigView {
        score_threshold: 0.70,
        must_include_skills: vec![
            "Rust".into(),
            "Python".into(),
            "System Architecture".into(),
            "AI/ML".into(),
        ],
        keywords,
        sources,
        llm_provider,
        llm_model,
        llm_status: "healthy".into(),
        rate_limit_per_min: 60,
        prompt_version: prompt_ver,
        profile,
        llm_backend,
        llm_api_base,
        llm_timeout_seconds,
    })
}
