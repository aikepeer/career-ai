//! `inspect_show` — gather an application's row, state history,
//! and rendered artifacts for `careerai inspect <id>`.

use std::path::Path;

use anyhow::{Context, Result};

use careerai_db::queries;

use crate::open_pool;

/// Structured payload for `careerai inspect <application_id>`.
#[derive(Debug)]
pub struct InspectReport {
    pub application: careerai_db::Application,
    pub listing_title: String,
    pub listing_company: String,
    pub listing_source: String,
    pub events: Vec<careerai_db::Event>,
    pub artifacts: Vec<careerai_db::Artifact>,
}

/// Gather everything needed to render `careerai inspect <id>`.
pub async fn inspect_show(root: &Path, application_id: &str) -> Result<InspectReport> {
    let pool = open_pool(root).await?;

    // Same typed-error pattern as apply_one: don't bail!() into strings.
    let application = queries::find_application_by_id(&pool, application_id).await?;
    let listing = queries::find_by_id(&pool, &application.listing_id).await?;
    let events = queries::events_for(&pool, &listing.id)
        .await
        .context("events_for listing")?;
    let artifacts = queries::list_artifacts(&pool, &application.id)
        .await
        .context("list_artifacts")?;

    Ok(InspectReport {
        application,
        listing_title: listing.title,
        listing_company: listing.company,
        listing_source: listing.source,
        events,
        artifacts,
    })
}
