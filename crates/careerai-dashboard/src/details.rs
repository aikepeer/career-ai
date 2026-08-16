#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use careerai_db::queries as db_queries;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::error::Result;
use crate::view::{
    ActionItem, ApplicationDetail, ArtifactItem, ConfigSourceItem, ConfigView, EventLogItem,
};

fn relative_time_label(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(ts);
    if delta.num_seconds() < 60 {
        "just now".to_string()
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else {
        format!("{}d ago", delta.num_days())
    }
}

pub async fn fetch_recent_events(pool: &SqlitePool, limit: u32) -> Result<Vec<EventLogItem>> {
    let events = db_queries::list_recent_events(pool, limit, 0).await?;
    let mut items = Vec::with_capacity(events.len());
    for ev in events {
        let relative = relative_time_label(ev.created_at);
        let severity = if ev.to_state == "failed" {
            "error".to_string()
        } else if ev.to_state == "skipped" || ev.from_state.as_deref() == Some("failed") {
            "warn".to_string()
        } else {
            "info".to_string()
        };
        items.push(EventLogItem {
            id: ev.id,
            listing_id: ev.listing_id,
            from_state: ev.from_state,
            to_state: ev.to_state,
            note: ev.note,
            timestamp: ev.created_at,
            relative_time: relative,
            severity,
        });
    }
    Ok(items)
}

pub async fn fetch_discovered_explorer(
    pool: &SqlitePool,
    limit: u32,
) -> Result<Vec<crate::view::DiscoveredExplorerItem>> {
    let rows = sqlx::query(
        "SELECT id, title, company, location, source, state, score, is_remote, url, created_at \
         FROM listings WHERE state IN ('discovered', 'filtered_out') \
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(careerai_db::error::DbError::from)?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id").map_err(careerai_db::error::DbError::from)?;
        let title: String = row.try_get("title").map_err(careerai_db::error::DbError::from)?;
        let company: String = row.try_get("company").map_err(careerai_db::error::DbError::from)?;
        let location: Option<String> = row.try_get("location").ok();
        let source: String = row.try_get("source").unwrap_or_else(|_| "unknown".to_string());
        let state: String = row.try_get("state").map_err(careerai_db::error::DbError::from)?;
        let score_f64: Option<f64> = row.try_get("score").ok();
        let score = score_f64.map(|s| s as f32);
        let is_remote_i64: Option<i64> = row.try_get("is_remote").ok();
        let is_remote = is_remote_i64.is_some_and(|r| r > 0);
        let url: String = row.try_get("url").unwrap_or_default();
        let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();

        out.push(crate::view::DiscoveredExplorerItem {
            id,
            title,
            company,
            location,
            source,
            state,
            score,
            is_remote,
            url,
            created_at,
        });
    }
    Ok(out)
}

pub async fn fetch_config_view(pool: &SqlitePool) -> Result<ConfigView> {
    let rows = sqlx::query(
        "SELECT source, COUNT(*) as n, MAX(created_at) as last_sync FROM listings GROUP BY source",
    )
    .fetch_all(pool)
    .await
    .map_err(careerai_db::error::DbError::from)?;

    let now = Utc::now();
    let mut sources = Vec::new();
    for row in rows {
        let name: String = row
            .try_get("source")
            .map_err(careerai_db::error::DbError::from)?;
        let count_i: i64 = row
            .try_get("n")
            .map_err(careerai_db::error::DbError::from)?;
        let last_sync: Option<DateTime<Utc>> = row.try_get("last_sync").ok();
        let (sync_label, status) = match last_sync {
            Some(ts) => {
                let hours = now.signed_duration_since(ts).num_hours();
                let label = relative_time_label(ts);
                let st = if hours > 48 { "stale" } else { "active" };
                (label, st.to_string())
            }
            None => ("Never".to_string(), "inactive".to_string()),
        };
        let kind = match name.as_str() {
            "greenhouse" | "lever" | "ashby" => "ATS Direct Feed",
            "linkedin" => "Web Scraper",
            "naukri" => "Browser Extension",
            _ => "Custom Board",
        }
        .to_string();

        sources.push(ConfigSourceItem {
            name,
            kind,
            listing_count: u64::try_from(count_i.max(0)).unwrap_or(0),
            last_sync,
            last_sync_label: sync_label,
            status,
        });
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let core_cfg = careerai_core::config::CoreConfig::load(&cwd).ok();

    let llm_provider = core_cfg
        .as_ref()
        .map(|c| c.llm.backend.as_str().to_string())
        .unwrap_or_else(|| "auto".to_string());

    let llm_model = core_cfg
        .as_ref()
        .map(|c| {
            if !c.llm.tailor_model.is_empty() {
                c.llm.tailor_model.clone()
            } else if !c.llm.parse_resume_model.is_empty() {
                c.llm.parse_resume_model.clone()
            } else {
                "default".to_string()
            }
        })
        .unwrap_or_else(|| "claude-3-5-sonnet".to_string());

    let prompt_ver = core_cfg
        .as_ref()
        .map(|c| c.llm.prompt_version.clone())
        .unwrap_or_else(|| "v1.2.0".to_string());

    Ok(ConfigView {
        score_threshold: 0.70,
        must_include_skills: vec![
            "Rust".into(),
            "Python".into(),
            "System Architecture".into(),
            "AI/ML".into(),
        ],
        sources,
        llm_provider,
        llm_model,
        llm_status: "healthy".into(),
        rate_limit_per_min: 60,
        prompt_version: prompt_ver,
    })
}

pub async fn fetch_action_center(pool: &SqlitePool) -> Result<Vec<ActionItem>> {
    let rows = sqlx::query(
        "SELECT id, source, external_id, title, company, state, score, url, created_at \
         FROM listings WHERE state IN ('drafted', 'responded') ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(careerai_db::error::DbError::from)?;

    let mut items = Vec::new();
    for row in rows {
        let listing_id: String = row
            .try_get("id")
            .map_err(careerai_db::error::DbError::from)?;
        let title: String = row
            .try_get("title")
            .map_err(careerai_db::error::DbError::from)?;
        let company: String = row
            .try_get("company")
            .map_err(careerai_db::error::DbError::from)?;
        let state: String = row
            .try_get("state")
            .map_err(careerai_db::error::DbError::from)?;
        let score_f64: Option<f64> = row.try_get("score").ok();
        let score = score_f64.map(|s| s as f32);
        let url: String = row
            .try_get("url")
            .map_err(careerai_db::error::DbError::from)?;
        let created_at: Option<DateTime<Utc>> = row.try_get("created_at").ok();

        let (action_type, action_label, action_url) = if state == "drafted" {
            (
                "review".to_string(),
                "Review Resume & Letter Draft".to_string(),
                None,
            )
        } else {
            (
                "prep".to_string(),
                "Open Interview Study Sheet".to_string(),
                Some(url.clone()),
            )
        };

        items.push(ActionItem {
            id: listing_id.clone(),
            listing_id,
            title,
            company,
            state,
            score,
            action_type,
            action_label,
            action_url,
            created_at,
        });
    }
    Ok(items)
}

pub async fn fetch_application_detail(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<ApplicationDetail>> {
    let (listing, app) = match db_queries::find_by_id(pool, id).await {
        Ok(l) => {
            let a = db_queries::find_latest_application_for_listing(pool, &l.id)
                .await
                .ok()
                .flatten();
            (l, a)
        }
        Err(_) => match db_queries::find_application_by_id(pool, id).await {
            Ok(a) => match db_queries::find_by_id(pool, &a.listing_id).await {
                Ok(l) => (l, Some(a)),
                Err(_) => return Ok(None),
            },
            Err(_) => return Ok(None),
        },
    };

    let (app_id, profile_hash, prompt_version, llm_model, app_created_at, app_updated_at) =
        match &app {
            Some(a) => (
                a.id.clone(),
                a.profile_hash.clone(),
                a.prompt_version.clone(),
                a.llm_model.clone(),
                a.created_at,
                a.updated_at,
            ),
            None => (
                format!("app_{}", listing.id),
                "sha256:none".to_string(),
                "v1.0.0".to_string(),
                "none".to_string(),
                listing.created_at,
                listing.updated_at,
            ),
        };

    let payload = if let Some(ref a) = app {
        db_queries::find_payload_by_application_id(pool, &a.id)
            .await
            .ok()
    } else {
        None
    };

    let db_artifacts = if let Some(ref a) = app {
        db_queries::list_artifacts(pool, &a.id)
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let artifacts = db_artifacts
        .into_iter()
        .map(|art| ArtifactItem {
            kind: art.kind,
            path: art.path,
            bytes: art.bytes,
        })
        .collect();

    let (resume_view_json, cover_letter_text) = match payload {
        Some(p) => (Some(p.resume_view_json), Some(p.cover_letter_text)),
        None => (None, None),
    };

    Ok(Some(ApplicationDetail {
        id: app_id,
        listing_id: listing.id,
        title: listing.title,
        company: listing.company,
        location: listing.location,
        url: listing.url,
        description: listing.description,
        state: listing.state,
        score: listing.score.map(|s| s as f32),
        source: listing.source,
        external_id: listing.external_id,
        profile_hash,
        prompt_version,
        llm_model,
        resume_view_json,
        cover_letter_text,
        artifacts,
        created_at: app_created_at,
        updated_at: app_updated_at,
    }))
}
