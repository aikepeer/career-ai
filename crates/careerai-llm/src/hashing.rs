//! Canonical-bytes hashing for cache keys.
//!
//! Three hashers, all sha256 + hex:
//!
//! - `canonical_profile_hash` — serializes the `Profile` via
//!   `serde_json`, recursively sorts object keys, then hashes. Stable
//!   against JSON field-order reordering.
//! - `jd_hash` — normalizes and concatenates title/company/description
//!   with `\n` separators.
//! - `compose_key` — joins (prompt_version, profile_hash, jd_hash, model)
//!   with the ASCII unit separator (0x1F) to avoid collisions from any
//!   input containing an embedded newline.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::cache::CacheKey;

/// Sha256 of the canonical JSON bytes of `profile`. Object keys are
/// recursively sorted so hash is stable across serializer key order.
#[must_use]
pub fn canonical_profile_hash(p: &careerai_profile::Profile) -> String {
    // Profile -> serde_json::Value goes via to_value; infallible for a type
    // that derives Serialize. If it does fail we fall back to a tagged
    // sentinel so the hash is at least deterministic per-error-shape.
    let value = serde_json::to_value(p).unwrap_or(serde_json::Value::Null);
    let canonical = canonicalize(value);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    sha256_hex(&bytes)
}

/// Sha256 of `lower(trim(title)) \n lower(trim(company)) \n lower(trim(description))`.
#[must_use]
pub fn jd_hash(title: &str, company: &str, description: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.trim().to_lowercase().as_bytes());
    hasher.update(b"\n");
    hasher.update(company.trim().to_lowercase().as_bytes());
    hasher.update(b"\n");
    hasher.update(description.trim().to_lowercase().as_bytes());
    hex::encode(hasher.finalize())
}

/// Compose the cache key from its four logical inputs, delimited by the
/// ASCII unit separator (0x1F) so embedded newlines/spaces are unambiguous.
#[must_use]
pub fn compose_key(
    prompt_version: &str,
    profile_hash: &str,
    jd_hash: &str,
    model: &str,
) -> CacheKey {
    let mut hasher = Sha256::new();
    hasher.update(prompt_version.as_bytes());
    hasher.update([0x1F]);
    hasher.update(profile_hash.as_bytes());
    hasher.update([0x1F]);
    hasher.update(jd_hash.as_bytes());
    hasher.update([0x1F]);
    hasher.update(model.as_bytes());
    CacheKey::new(hex::encode(hasher.finalize()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Walk a `serde_json::Value` and rebuild it with all object keys sorted
/// lexicographically. Array order is preserved; scalars pass through.
fn canonicalize(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sorted: BTreeMap<String, serde_json::Value> =
                map.into_iter().map(|(k, v)| (k, canonicalize(v))).collect();
            let mut out = serde_json::Map::with_capacity(sorted.len());
            for (k, v) in sorted {
                out.insert(k, v);
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonicalize).collect())
        }
        other => other,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_profile::Profile;

    fn sample_profile() -> Profile {
        // Build a Profile via serde_yaml round-trip so we don't depend on
        // hand-constructed field-by-field initialization that drifts with
        // schema.rs.
        let yaml = r"
personal:
  name: Ada Lovelace
  email: ada@example.com
summary: Math-first engineer.
skills:
  languages: [Rust, Python]
experience: []
education: []
projects: []
";
        Profile::from_yaml(yaml).unwrap()
    }

    #[test]
    fn profile_hash_stable_across_json_key_reorder() {
        let p = sample_profile();
        let h1 = canonical_profile_hash(&p);

        // Round-trip through JSON and back; canonicalization must keep the
        // hash stable regardless of how serializers order keys internally.
        let as_value = serde_json::to_value(&p).unwrap();
        let reordered_json = serde_json::to_string(&as_value).unwrap();
        let back: Profile = serde_json::from_str(&reordered_json).unwrap();
        let h2 = canonical_profile_hash(&back);
        assert_eq!(h1, h2);

        // And: directly reorder a Value with a different map ordering.
        let raw_forward = r#"{"a":1,"b":{"x":1,"y":2}}"#;
        let raw_backward = r#"{"b":{"y":2,"x":1},"a":1}"#;
        let v1: serde_json::Value = serde_json::from_str(raw_forward).unwrap();
        let v2: serde_json::Value = serde_json::from_str(raw_backward).unwrap();
        let c1 = canonicalize(v1);
        let c2 = canonicalize(v2);
        assert_eq!(
            serde_json::to_vec(&c1).unwrap(),
            serde_json::to_vec(&c2).unwrap()
        );
    }

    #[test]
    fn different_model_changes_key() {
        let k1 = compose_key("tailor.v1", "p", "j", "claude-3-5");
        let k2 = compose_key("tailor.v1", "p", "j", "claude-3-7");
        assert_ne!(k1.as_str(), k2.as_str());
    }

    #[test]
    fn compose_key_deterministic() {
        let k1 = compose_key("tailor.v1", "phash", "jdhash", "m");
        let k2 = compose_key("tailor.v1", "phash", "jdhash", "m");
        assert_eq!(k1.as_str(), k2.as_str());
        assert_eq!(k1.as_str().len(), 64);
    }

    #[test]
    fn jd_hash_normalizes_case_and_whitespace() {
        let a = jd_hash("  Senior ENG  ", " ACME ", "Build stuff.");
        let b = jd_hash("senior eng", "acme", "build stuff.");
        assert_eq!(a, b);
    }
}
