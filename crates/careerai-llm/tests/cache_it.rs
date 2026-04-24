//! Integration tests for the on-disk LLM response cache.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_llm::{Cache, CacheKey, LlmResponse};
use tempfile::tempdir;

fn fresh_response(text: &str) -> LlmResponse {
    LlmResponse {
        text: text.into(),
        prompt_tokens: 1,
        completion_tokens: 2,
        cache_hit: false,
        cached_prompt_tokens: 0,
    }
}

#[tokio::test]
async fn round_trip_flips_cache_hit_from_false_to_true() {
    let dir = tempdir().unwrap();
    let cache = Cache::new(dir.path());
    let key = CacheKey::new("c".repeat(64));

    let r = fresh_response("v1");
    assert!(!r.cache_hit);
    cache.put(&key, &r).await.unwrap();

    let got = cache.get(&key).await.unwrap().expect("expected a hit");
    assert!(got.cache_hit, "get() must flip cache_hit=true on hit");
    assert_eq!(got.text, "v1");
}

#[tokio::test]
async fn miss_returns_ok_none() {
    let dir = tempdir().unwrap();
    let cache = Cache::new(dir.path());
    let key = CacheKey::new("d".repeat(64));
    assert!(cache.get(&key).await.unwrap().is_none());
}

#[tokio::test]
async fn second_put_wins_and_leaves_no_tmp_files() {
    let dir = tempdir().unwrap();
    let cache = Cache::new(dir.path());
    let key = CacheKey::new("e".repeat(64));

    cache.put(&key, &fresh_response("first")).await.unwrap();
    cache.put(&key, &fresh_response("second")).await.unwrap();

    let got = cache.get(&key).await.unwrap().unwrap();
    assert_eq!(got.text, "second", "last put must win");

    // Directory should contain exactly one entry: <hex>.json.
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one cache file, found: {entries:?}"
    );
    assert!(
        entries.iter().all(|n| {
            let path = std::path::Path::new(n);
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
                && !n.contains(".tmp-")
        }),
        "tmp file leaked: {entries:?}"
    );
}
