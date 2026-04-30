//! JSON response parser: unpacks MCP tool-call output into [`RawListing`]s.
//! Tolerates schema drift between community MCP servers via permissive
//! field-aliasing.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::base::RawListing;

/// Parse a JSON-text response body into `RawListing`s.
///
/// Community MCP servers return one of:
/// - a top-level array (`[ {...}, {...} ]`)
/// - an object wrapping the array under `results` / `jobs` / `data`.
///
/// Inside each entry we try multiple field aliases per logical column
/// and keep the first non-empty hit. Unknown fields are preserved as
/// the `raw_json` payload for debugging.
pub(crate) fn parse_listings(body: &str, source_name: &str) -> Result<Vec<RawListing>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| format!("invalid JSON: {e}"))?;
    let arr = extract_array(&value).ok_or_else(|| {
        format!(
            "expected JSON array or {{ results | jobs | data: [...] }}; got {}",
            value.as_object().map_or_else(
                || "scalar".into(),
                |o| format!("object with keys {:?}", o.keys().collect::<Vec<_>>()),
            )
        )
    })?;
    Ok(arr
        .iter()
        .filter_map(|item| map_listing(item, source_name))
        .collect())
}

/// Walk the common envelopes (`results`, `jobs`, `data`) to find the
/// listings array.
fn extract_array(value: &Value) -> Option<&Vec<Value>> {
    if let Some(arr) = value.as_array() {
        return Some(arr);
    }
    let obj = value.as_object()?;
    for key in ["results", "jobs", "data", "listings", "items"] {
        if let Some(Value::Array(a)) = obj.get(key) {
            return Some(a);
        }
    }
    None
}

fn map_listing(item: &Value, source_name: &str) -> Option<RawListing> {
    let title = pick_string(item, &["title", "job_title", "position"]).unwrap_or_default();
    let company = pick_string(item, &["company", "company_name", "employer"]).unwrap_or_default();
    let url = pick_string(item, &["url", "apply_url", "link"]).unwrap_or_default();
    if title.is_empty() && company.is_empty() && url.is_empty() {
        // Nothing useful — skip the row rather than emit a ghost.
        return None;
    }
    let location = pick_string(item, &["location", "place", "city"]);
    // Strip HTML to plain text — community MCPs (e.g. mcp-linkedin)
    // return marketing HTML in `description`, and the matcher
    // embeddings work much better on clean text. Mirrors the same
    // pre-processing done by every other adapter (greenhouse, lever,
    // remoteok, remotive, naukri).
    let description = pick_string(item, &["description", "summary", "snippet"])
        .map(|s| crate::util::html_to_text(&s))
        .unwrap_or_default();
    let external_id = pick_string(item, &["id", "job_id", "external_id"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| derive_external_id(source_name, &url, &title, &company));
    Some(RawListing {
        source: source_name.to_string(),
        external_id,
        title,
        company,
        location,
        url,
        description,
        raw_json: Some(item.to_string()),
    })
}

/// Try each candidate key. If the value is a string, return it; if it
/// is a non-string scalar, stringify it. Empty strings count as
/// "missing" so the next alias gets a chance.
fn pick_string(item: &Value, keys: &[&str]) -> Option<String> {
    let obj = item.as_object()?;
    for key in keys {
        match obj.get(*key) {
            Some(Value::String(s)) if !s.is_empty() => return Some(s.clone()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            Some(Value::Bool(b)) => return Some(b.to_string()),
            _ => {}
        }
    }
    None
}

/// SHA256 over `(source_name, url, title, company)` truncated to 16
/// hex chars. Used when the server doesn't expose a stable ID; the
/// downstream upsert key `(source, external_id)` then dedupes runs as
/// long as the server returns the same `(url, title, company)`.
pub(crate) fn derive_external_id(
    source_name: &str,
    url: &str,
    title: &str,
    company: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(url.as_bytes());
    hasher.update(b"\0");
    hasher.update(title.as_bytes());
    hasher.update(b"\0");
    hasher.update(company.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

// `parse_posted_at` was previously used to populate a discarded
// `_posted_at_unused` local. `RawListing` has no `posted_at` field; the
// value was thrown away. Removed in the PR-19 review pass — if the
// schema later grows a `posted_at`, reintroduce the parser then.
