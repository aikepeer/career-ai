//! Criterion benchmark for `JaccardScorer::score`.
//!
//! TODO.md hot path: "JaccardScorer::score against a 500-token
//! profile / 300-token JD (the realistic shape)". This bench
//! generates representative inputs at three scales and measures
//! per-call latency so future scorer swaps (e.g. BGE cosine) can
//! be compared honestly.
//!
//! Run: `cargo bench -p careerai-match`

use careerai_match::{JaccardScorer, Scorer};
use careerai_sources::RawListing;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

/// Build a deterministic token bag with `n` *distinct* tokens. Each
/// token is `<vocab-stem>-<counter>` so distinct count == n at every
/// scale.
///
/// Without the counter suffix, a 2000-token bag would only have ~70
/// distinct entries (modulo VOCAB len). `JaccardScorer::score`
/// deduplicates via `HashSet`, so its intersection/union work would
/// be bounded by ~70 — understating real-input latency by
/// roughly an order of magnitude and producing a misleading
/// baseline for any future scorer comparison (Codex P1 on PR #66).
fn token_bag(n: usize, seed: u64) -> String {
    const VOCAB: &[&str] = &[
        "rust",
        "tokio",
        "async",
        "trait",
        "lifetime",
        "borrow",
        "move",
        "ownership",
        "macro",
        "cargo",
        "clippy",
        "fmt",
        "test",
        "bench",
        "tracing",
        "serde",
        "axum",
        "sqlx",
        "sqlite",
        "postgres",
        "python",
        "go",
        "java",
        "kotlin",
        "swift",
        "typescript",
        "react",
        "vue",
        "svelte",
        "next",
        "nuxt",
        "node",
        "deno",
        "kubernetes",
        "docker",
        "helm",
        "terraform",
        "pulumi",
        "aws",
        "gcp",
        "azure",
        "linux",
        "wayland",
        "embedded",
        "robotics",
        "ros",
        "gpu",
        "cuda",
        "ml",
        "llm",
        "rag",
        "transformer",
        "engineer",
        "developer",
        "architect",
        "designer",
        "manager",
        "led",
        "owned",
        "shipped",
        "scaled",
        "automated",
        "designed",
        "implemented",
        "rewrote",
        "migrated",
        "optimized",
        "reduced",
        "improved",
        "delivered",
        "spearheaded",
        "mentored",
        "interviewed",
    ];
    let mut s = String::with_capacity(n * 16);
    #[allow(clippy::cast_possible_truncation)]
    let mut idx = seed as usize;
    for i in 0..n {
        // `<vocab-stem>-<counter>` shape: the JaccardScorer tokenizer
        // splits on whitespace, so the hyphen does NOT split. The
        // counter `i` guarantees distinctness up to `n`; the hashed
        // `idx` decides which vocab stem to suffix so tokens still
        // look plausible (e.g. `robotics-345`) rather than a flat
        // `token42 token43` sequence.
        s.push_str(VOCAB[idx % VOCAB.len()]);
        s.push('-');
        s.push_str(&i.to_string());
        s.push(' ');
        idx = idx.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    }
    s
}

fn make_listing(jd_tokens: usize, seed: u64) -> RawListing {
    RawListing {
        source: "bench".into(),
        external_id: "1".into(),
        title: "Senior ML Engineer".into(),
        company: "Acme Robotics".into(),
        location: Some("Remote".into()),
        url: "https://example.com/1".into(),
        description: token_bag(jd_tokens, seed),
        raw_json: None,
    }
}

fn bench_realistic_shape(c: &mut Criterion) {
    let scorer = JaccardScorer;
    // 500-token profile / 300-token JD — the shape TODO.md calls out.
    let profile = token_bag(500, 0x00C0_FFEE);
    let listing = make_listing(300, 0xDEAD_BEEF);
    c.bench_function("jaccard/profile=500/jd=300", |b| {
        b.iter(|| {
            let s = scorer.score(black_box(&profile), black_box(&listing));
            black_box(s);
        });
    });
}

fn bench_small(c: &mut Criterion) {
    let scorer = JaccardScorer;
    let profile = token_bag(50, 1);
    let listing = make_listing(50, 2);
    c.bench_function("jaccard/profile=50/jd=50", |b| {
        b.iter(|| {
            let s = scorer.score(black_box(&profile), black_box(&listing));
            black_box(s);
        });
    });
}

fn bench_large(c: &mut Criterion) {
    let scorer = JaccardScorer;
    // 2k-token profile (long career history) / 1.5k-token JD (every
    // requirement enumerated — common for senior roles).
    let profile = token_bag(2_000, 11);
    let listing = make_listing(1_500, 22);
    c.bench_function("jaccard/profile=2000/jd=1500", |b| {
        b.iter(|| {
            let s = scorer.score(black_box(&profile), black_box(&listing));
            black_box(s);
        });
    });
}

criterion_group!(benches, bench_small, bench_realistic_shape, bench_large);
criterion_main!(benches);
