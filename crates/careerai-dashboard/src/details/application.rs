#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use sqlx::SqlitePool;

use careerai_db::queries as db_queries;
use careerai_db::DbError;

use crate::error::Result;
use crate::view::{ApplicationDetail, ArtifactItem};

use super::fetch_events_for_listing;

pub async fn fetch_application_detail(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<ApplicationDetail>> {
    // R12: distinguish NotFound (absence → Ok(None)) from real DB errors
    // (propagate as Err). The previous version used `.ok()` and
    // `.unwrap_or_default()` which swallowed all errors, making a DB
    // failure indistinguishable from "not found".
    let (listing, app) = match db_queries::find_by_id(pool, id).await {
        Ok(l) => {
            let a = db_queries::find_latest_application_for_listing(pool, &l.id).await?;
            (l, a)
        }
        Err(DbError::NotFound(_)) => {
            // Not found as a listing — try as an application ID.
            match db_queries::find_application_by_id(pool, id).await {
                Ok(a) => match db_queries::find_by_id(pool, &a.listing_id).await {
                    Ok(l) => (l, Some(a)),
                    Err(DbError::NotFound(_)) => return Ok(None),
                    Err(e) => return Err(e.into()),
                },
                Err(DbError::NotFound(_)) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        }
        Err(e) => return Err(e.into()),
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

    // R12: propagate payload DB errors. NotFound on the payload is expected
    // (an application may not have a payload yet), so map only NotFound to None.
    let payload = if let Some(a) = &app {
        match db_queries::find_payload_by_application_id(pool, &a.id).await {
            Ok(p) => Some(p),
            Err(DbError::NotFound(_)) => None,
            Err(e) => return Err(e.into()),
        }
    } else {
        None
    };

    let db_artifacts = if let Some(a) = &app {
        db_queries::list_artifacts(pool, &a.id).await?
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

    let timeline = fetch_events_for_listing(pool, &listing.id).await?;

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
        timeline,
    }))
}
