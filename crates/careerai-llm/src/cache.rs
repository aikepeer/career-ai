//! On-disk response cache for LLM calls.
//!
//! Keyed by `CacheKey` (64-char sha256 hex computed via `hashing.rs`).
//! Entries are plain JSON files at `<root>/<hex>.json`. Writes are atomic:
//! write-to-tmp + rename. Reads set `cache_hit = true` on the returned
//! response; the stored on-disk form always has `cache_hit = false` so a
//! subsequent hit observably changes the field.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::trace;

use crate::error::{LlmError, Result};
use crate::types::LlmResponse;

/// Newtype over the 64-char hex of `sha256(compose_key-inputs)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKey(String);

impl CacheKey {
    #[must_use]
    pub fn new(hex: String) -> Self {
        Self(hex)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CacheKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Keys must be exactly 64 ASCII hex digits (the shape produced by
    /// `hashing::compose_key`). Rejecting anything else keeps `../`,
    /// absolute paths, and other traversal strings out of the
    /// `root.join(...)` in [`Cache::path_for`].
    fn validate(key: &CacheKey) -> Result<()> {
        let hex = key.as_str();
        if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(())
        } else {
            Err(LlmError::Schema(
                "invalid cache key: expected 64 hex characters".into(),
            ))
        }
    }

    fn path_for(&self, key: &CacheKey) -> PathBuf {
        self.root.join(format!("{}.json", key.as_str()))
    }

    /// Read a cached response. Returns `Ok(None)` if the file does not
    /// exist; any other I/O or parse error surfaces as `LlmError`.
    pub async fn get(&self, key: &CacheKey) -> Result<Option<LlmResponse>> {
        Self::validate(key)?;
        let path = self.path_for(key);
        let bytes = match fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                trace!(key = %key, "cache miss");
                return Ok(None);
            }
            Err(e) => return Err(LlmError::Io(e)),
        };
        let mut resp: LlmResponse = serde_json::from_slice(&bytes)?;
        resp.cache_hit = true;
        trace!(key = %key, "cache hit");
        Ok(Some(resp))
    }

    /// Atomically write a response. Writes to `<hex>.json.tmp-<pid>-<nanos>`
    /// and renames to `<hex>.json`. The persisted form has
    /// `cache_hit = false` so a subsequent `get()` flips it to true.
    pub async fn put(&self, key: &CacheKey, value: &LlmResponse) -> Result<()> {
        Self::validate(key)?;
        fs::create_dir_all(&self.root).await?;
        let mut to_persist = value.clone();
        to_persist.cache_hit = false;
        let bytes = serde_json::to_vec(&to_persist)?;

        let final_path = self.path_for(key);
        let pid = std::process::id();
        // `as_nanos()` returns u128; nanosecond resolution is enough entropy
        // across concurrent writers in the same process, paired with pid it
        // avoids cross-process races on the tmp file.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let tmp_path = self
            .root
            .join(format!("{}.json.tmp-{pid}-{nanos}", key.as_str()));

        fs::write(&tmp_path, &bytes).await?;
        match fs::rename(&tmp_path, &final_path).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // Best-effort cleanup; preserve the original error.
                let _ = fs::remove_file(&tmp_path).await;
                Err(LlmError::Io(e))
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_response() -> LlmResponse {
        LlmResponse {
            text: "hello".into(),
            prompt_tokens: 10,
            completion_tokens: 5,
            cache_hit: false,
            cached_prompt_tokens: 0,
        }
    }

    #[tokio::test]
    async fn roundtrip_put_then_get_flips_cache_hit() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let key = CacheKey::new("a".repeat(64));
        cache.put(&key, &sample_response()).await.unwrap();

        let got = cache.get(&key).await.unwrap().unwrap();
        assert_eq!(got.text, "hello");
        assert!(got.cache_hit, "get() must set cache_hit=true on hit");
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_key() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let key = CacheKey::new("b".repeat(64));
        assert!(cache.get(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejects_non_hex_traversal_key() {
        let dir = tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let key = CacheKey::new("../outside".into());
        assert!(cache.put(&key, &sample_response()).await.is_err());
    }
}
