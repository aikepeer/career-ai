//! Persistence models. `state` and timestamps are stored as TEXT in SQLite
//! and converted at the call site to keep sqlx feature surface minimal
//! (no compile-time DATABASE_URL required).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use careerai_core::state::ListingState;

use crate::error::Result;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Listing {
    pub id: String,
    pub source: String,
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: String,
    pub description: String,
    pub raw_json: Option<String>,
    pub state: String,
    pub score: Option<f64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Listing {
    /// Parse the persisted state column into the typed enum.
    pub fn typed_state(&self) -> Result<ListingState> {
        Ok(self.state.parse()?)
    }
}

/// What every source adapter must hand to the DB layer. Pre-persistence —
/// no id, no state, no timestamps yet.
#[derive(Debug, Clone)]
pub struct NewListing {
    pub source: String,
    pub external_id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: String,
    pub description: String,
    pub raw_json: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub listing_id: String,
    pub from_state: Option<String>,
    pub to_state: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One user-approved "tailor this listing" attempt. Multiple applications
/// per listing are allowed (re-tailor after profile edits); the latest one
/// by `created_at` is considered current.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Application {
    pub id: String,
    pub listing_id: String,
    pub state: String,
    pub profile_hash: String,
    pub prompt_version: String,
    pub llm_model: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Tailored resume JSON + cover letter text, stored for inspect/diff without
/// re-calling the LLM. Separate from `Artifact` which tracks files on disk.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ApplicationPayload {
    pub application_id: String,
    pub resume_view_json: String,
    pub cover_letter_text: String,
    pub diff_json: String,
    pub created_at: DateTime<Utc>,
}

/// One rendered output file on disk for an application (e.g. resume.docx).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Artifact {
    pub id: i64,
    pub application_id: String,
    pub kind: String,
    pub path: String,
    pub bytes: i64,
    pub created_at: DateTime<Utc>,
}

/// Pre-persistence view of an application — id and timestamps are assigned
/// by the DB layer.
#[derive(Debug, Clone)]
pub struct NewApplication {
    pub listing_id: String,
    pub profile_hash: String,
    pub prompt_version: String,
    pub llm_model: String,
}

/// Pre-persistence view of a rendered artifact — id, application_id, and
/// created_at are assigned by the DB layer.
#[derive(Debug, Clone)]
pub struct NewArtifact {
    pub kind: String,
    pub path: String,
    pub bytes: i64,
}
