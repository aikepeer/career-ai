use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------- careerai_discover ----------

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct DiscoverArgs {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DiscoverResult {
    pub fetched: usize,
    pub new_rows: usize,
    pub duplicates: usize,
    pub errors: usize,
}

// ---------- careerai_shortlist ----------

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ShortlistArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ShortlistEntry {
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub url: String,
    pub source: String,
    pub score: Option<f32>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ShortlistResult {
    pub entries: Vec<ShortlistEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CompactListing {
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub url: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

// ---------- careerai_tailor ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct TailorArgs {
    pub listing_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TailorResult {
    pub application_id: String,
    pub diff_summary: String,
}

// ---------- careerai_render ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RenderArgs {
    pub application_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RenderResult {
    pub application_id: String,
    pub docx_path: String,
    pub pdf_path: String,
    pub cover_docx_path: String,
}

// ---------- careerai_apply ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ApplyArgs {
    pub application_id: String,
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
}

fn default_dry_run() -> bool {
    true
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ApplyResult {
    pub application_id: String,
    pub source: String,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub would_submit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// ---------- careerai_inspect ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct InspectArgs {
    pub application_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InspectEvent {
    pub from_state: Option<String>,
    pub to_state: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InspectArtifact {
    pub kind: String,
    pub path: String,
    pub bytes: i64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InspectResult {
    pub application_id: String,
    pub listing_title: String,
    pub listing_company: String,
    pub listing_source: String,
    pub state: String,
    pub events: Vec<InspectEvent>,
    pub artifacts: Vec<InspectArtifact>,
}

// ---------- careerai_digest ----------

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DigestArgs {
    pub since: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DigestResult {
    pub markdown: String,
    pub since_iso: String,
    pub discovered: usize,
    pub matched: usize,
    pub shortlisted: usize,
    pub drafted: usize,
    pub submitted: usize,
    pub failed: usize,
    pub responded: usize,
    pub per_source: BTreeMap<String, usize>,
    pub last_tick: Option<String>,
}

// ---------- careerai_profile_status ----------

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ProfileStatusArgs {}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProfileStatusResult {
    pub path: String,
    pub exists: bool,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    pub issues: Vec<String>,
}
