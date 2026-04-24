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
