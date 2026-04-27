//! Tool input/output schemas.
//!
//! Each tool exposed by the MCP server has its argument struct and result
//! struct here. The structs derive `schemars::JsonSchema` so `rmcp` can
//! advertise the JSON Schema to the LLM, plus `serde::{Deserialize,
//! Serialize}` for round-trip JSON-RPC framing.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------- careerai_discover ----------

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct DiscoverArgs {
    /// Restrict discovery to these source names (e.g. `["greenhouse",
    /// "lever"]`). Empty/omitted means run every source enabled in
    /// config.
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
    /// Maximum rows to return. Defaults to 50; capped at 500.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Optional minimum score threshold (0..1). Listings below this are
    /// filtered out client-side after the DB query.
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

/// Compact projection of a `careerai_db::Listing` for the
/// `careerai://shortlist/{date}` MCP resource. The DB row carries a
/// free-form `description` (HTML/text JD body) and `raw_json` (full ATS
/// payload) that bloat the resource for LLM consumption — a single
/// shortlist response can blow past the model's context window. This
/// struct keeps only the fields a tailoring/triage LLM actually needs:
/// id, headline (`title`/`company`/`location`), provenance (`source`,
/// `url`), and the match `score`.
///
/// Mirrors the shape of `ShortlistEntry` (which the `careerai_shortlist`
/// tool returns) plus `location`. They are kept as separate types
/// because tool results and resource bodies have separate JSON Schemas
/// in MCP and may evolve independently.
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
    /// Listing id (string UUID v7) of a shortlisted listing.
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
    /// Defaults to `true`. Real submission only happens when the caller
    /// explicitly passes `dry_run: false` AND `confirm:
    /// "I_UNDERSTAND_TOS_RISK"`. Per-source `submit_enabled` gates in
    /// config still apply.
    #[serde(default = "default_dry_run")]
    pub dry_run: bool,
    /// Required when `dry_run` is `false`. Must equal the literal
    /// string `"I_UNDERSTAND_TOS_RISK"`.
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
    /// One of `Submitted` | `DryRun` | `Drafted` | `Skipped`.
    pub outcome: String,
    /// Filled when `outcome == "DryRun"` — short payload summary that
    /// would have been sent. Always empty for `Submitted` (real
    /// submission paths return `remote_id` in `note`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub would_submit: Option<String>,
    /// Submitter notes: dry-run summary, `Drafted` reason, `Skipped`
    /// reason, or remote id on success.
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
    /// Time window. Accepts `<n>h` / `<n>d` / `<n>w` (e.g. `"1d"`,
    /// `"24h"`, `"2w"`) or a bare integer interpreted as hours.
    pub since: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DigestResult {
    /// Pre-formatted markdown digest, ready for the LLM to render.
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
    /// RFC 3339 UTC timestamp from the file's `mtime`. Populated whenever
    /// the file's metadata is readable — including when the YAML body
    /// fails to parse or validate (the on-disk timestamp is useful
    /// diagnostic info regardless of content health). `None` only when
    /// the file is absent or its metadata is unreadable
    /// (e.g. permission denied on the parent directory).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    /// One human-readable line per validation issue. Empty when valid.
    pub issues: Vec<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Round-trip every input arg shape through `serde_json` to assert
    /// our `#[serde]` and `#[derive(Deserialize)]` choices stay
    /// permissive (defaults, `Option<…>` for missing keys, etc.).
    #[test]
    fn discover_args_default_when_empty() {
        let args: DiscoverArgs = serde_json::from_str("{}").unwrap();
        assert!(args.sources.is_empty());
    }

    #[test]
    fn discover_args_accepts_sources_array() {
        let args: DiscoverArgs =
            serde_json::from_str(r#"{"sources":["greenhouse","lever"]}"#).unwrap();
        assert_eq!(args.sources, vec!["greenhouse", "lever"]);
    }

    #[test]
    fn shortlist_args_default_limit_and_min_score() {
        let args: ShortlistArgs = serde_json::from_str("{}").unwrap();
        assert!(args.limit.is_none());
        assert!(args.min_score.is_none());
    }

    #[test]
    fn shortlist_args_accepts_partial() {
        let args: ShortlistArgs = serde_json::from_str(r#"{"limit":10}"#).unwrap();
        assert_eq!(args.limit, Some(10));
        assert!(args.min_score.is_none());
    }

    #[test]
    fn apply_args_dry_run_defaults_true() {
        let args: ApplyArgs = serde_json::from_str(r#"{"application_id":"abc"}"#).unwrap();
        assert_eq!(args.application_id, "abc");
        assert!(args.dry_run, "dry_run must default to true (safety)");
        assert!(args.confirm.is_none());
    }

    #[test]
    fn apply_args_carries_confirm_token() {
        let args: ApplyArgs = serde_json::from_str(
            r#"{"application_id":"abc","dry_run":false,"confirm":"I_UNDERSTAND_TOS_RISK"}"#,
        )
        .unwrap();
        assert!(!args.dry_run);
        assert_eq!(args.confirm.as_deref(), Some("I_UNDERSTAND_TOS_RISK"));
    }

    #[test]
    fn digest_args_required() {
        let err = serde_json::from_str::<DigestArgs>("{}");
        assert!(err.is_err(), "since must be required");
        let ok: DigestArgs = serde_json::from_str(r#"{"since":"1d"}"#).unwrap();
        assert_eq!(ok.since, "1d");
    }

    #[test]
    fn tailor_and_render_and_inspect_args_require_id() {
        assert!(serde_json::from_str::<TailorArgs>("{}").is_err());
        assert!(serde_json::from_str::<RenderArgs>("{}").is_err());
        assert!(serde_json::from_str::<InspectArgs>("{}").is_err());
    }

    /// Result types serialize as expected (camel/snake case stays
    /// consistent with the field names).
    #[test]
    fn discover_result_serializes_snake_case() {
        let r = DiscoverResult {
            fetched: 12,
            new_rows: 3,
            duplicates: 9,
            errors: 0,
        };
        let v: serde_json::Value = serde_json::to_value(&r).unwrap();
        assert_eq!(v["new_rows"], 3);
        assert_eq!(v["fetched"], 12);
    }

    #[test]
    fn apply_result_omits_none_fields() {
        let r = ApplyResult {
            application_id: "id".into(),
            source: "greenhouse".into(),
            outcome: "Skipped".into(),
            would_submit: None,
            note: Some("source disabled".into()),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(
            !s.contains("would_submit"),
            "would_submit should be skipped: {s}"
        );
        assert!(s.contains("note"));
    }

    /// `CompactListing` is the projection used by the
    /// `careerai://shortlist/{date}` resource. The whole reason the
    /// type exists is to keep the bulky `description` (free-form HTML/text
    /// JD) and `raw_json` (full ATS payload) fields off the wire — those
    /// can be tens of kilobytes per row and blow the LLM context. This
    /// test pins that contract: serializing a `CompactListing` must not
    /// produce keys `description` or `raw_json`, and `None` optional
    /// fields are skipped.
    #[test]
    fn compact_listing_excludes_bulky_fields() {
        let c = CompactListing {
            listing_id: "L-1".into(),
            title: "Staff ML Engineer".into(),
            company: "ACME".into(),
            url: "https://example.test/j/1".into(),
            source: "greenhouse".into(),
            score: Some(0.87),
            location: None,
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(
            !s.contains("description"),
            "description must not appear in compact resource body: {s}"
        );
        assert!(
            !s.contains("raw_json"),
            "raw_json must not appear in compact resource body: {s}"
        );
        // None optional fields are skipped.
        assert!(
            !s.contains("location"),
            "None location must be skipped: {s}"
        );
        // Required fields are present.
        for key in ["listing_id", "title", "company", "url", "source", "score"] {
            assert!(s.contains(key), "expected key {key} in {s}");
        }
    }
}
