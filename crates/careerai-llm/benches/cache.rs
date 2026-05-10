//! Criterion benchmarks for the on-disk LLM response cache.
//!
//! Measures cache hit (read existing file), cache miss (file-not-found),
//! and cache put (write + atomic rename). Each is I/O bound but the
//! costs are well under 1ms on modern SSDs; mainly useful as a canary
//! to detect regressions if the storage format or hashing changes.
//!
//! Run: `cargo bench -p careerai-llm --bench cache`
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;

use careerai_llm::cache::{Cache, CacheKey};
use careerai_llm::types::LlmResponse;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tempfile::TempDir;
use tokio::runtime::Runtime;

fn rt() -> Runtime {
    Runtime::new().unwrap()
}

fn sample_response() -> LlmResponse {
    LlmResponse {
        text: "Tailored bullet: Shipped Rust pipeline reducing latency 35%.".into(),
        prompt_tokens: 120,
        completion_tokens: 45,
        cache_hit: false,
        cached_prompt_tokens: 0,
    }
}

fn bench_cache_miss(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let cache = Cache::new(dir.path());
    let key = CacheKey::new("a".repeat(64));
    let rt = rt();
    c.bench_function("cache/miss", |b| {
        b.iter(|| {
            rt.block_on(async {
                let result = cache.get(black_box(&key)).await.unwrap();
                black_box(result);
            });
        });
    });
}

fn bench_cache_hit(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let cache = Cache::new(dir.path());
    let key = CacheKey::new("b".repeat(64));
    let resp = sample_response();
    let rt = rt();
    rt.block_on(async { cache.put(&key, &resp).await.unwrap() });
    c.bench_function("cache/hit", |b| {
        b.iter(|| {
            rt.block_on(async {
                let result = cache.get(black_box(&key)).await.unwrap();
                black_box(result);
            });
        });
    });
}

fn bench_cache_put(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let cache = Cache::new(dir.path());
    let resp = sample_response();
    let counter: Cell<u64> = Cell::new(0);
    let rt = rt();
    c.bench_function("cache/put", |b| {
        b.iter(|| {
            let n = counter.get();
            counter.set(n + 1);
            let key = CacheKey::new(format!("c{n:063x}"));
            rt.block_on(async {
                cache.put(black_box(&key), black_box(&resp)).await.unwrap();
            });
        });
    });
}

criterion_group!(benches, bench_cache_miss, bench_cache_hit, bench_cache_put);
criterion_main!(benches);
