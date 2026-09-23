use careerai_db::models::Listing;
use careerai_match::cluster_jds;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn fixtures() -> Vec<Listing> {
    (0..100)
        .map(|index| Listing {
            id: index.to_string(),
            source: "benchmark".into(),
            external_id: index.to_string(),
            title: if index % 2 == 0 {
                "Rust backend engineer".into()
            } else {
                "Embedded firmware engineer".into()
            },
            company: "Acme".into(),
            location: None,
            url: format!("https://example.test/{index}"),
            description: if index % 2 == 0 {
                "Rust Tokio distributed services and observability".into()
            } else {
                "C++ Linux BSP firmware and device drivers".into()
            },
            raw_json: None,
            state: "shortlisted".into(),
            score: Some(index as f64 / 100.0),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        })
        .collect()
}

fn cluster_benchmark(c: &mut Criterion) {
    let listings = fixtures();
    c.bench_function("cluster/100-listings", |b| {
        b.iter(|| black_box(cluster_jds(black_box(&listings), black_box(0.85))))
    });
}

criterion_group!(benches, cluster_benchmark);
criterion_main!(benches);
