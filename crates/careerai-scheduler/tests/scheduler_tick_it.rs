//! Integration test: the cron tick actually fires.
//!
//! Stub strategy — DB-seed (or rather: empty-DB, no seed at all). We don't
//! wire a stubbed `Source` because that would mean either standing up
//! `wiremock` for every adapter or reaching past the pipeline boundary into
//! `careerai-sources`. Instead we exercise only the **submit cron path**:
//! set `submit_cadence` to fire every second, point at a fresh tempdir DB,
//! and let `apply_all` walk an empty `applications` table. That call
//! returns `Ok(vec![])` cleanly — proving the cron tick fired and the
//! pipeline entry point ran without panicking, which is exactly the
//! "tick handler ran" signal we need.
//!
//! Per-source `cadence` is cleared so no real network discover/match
//! kicks off — keeps the test fully offline.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use careerai_core::config::CoreConfig;
use careerai_scheduler::Scheduler;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn submit_cron_tick_fires_within_three_seconds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = CoreConfig::load(tmp.path()).expect("load embedded defaults");

    // No per-source ticks — those would hit real networks. We only want
    // the submit cron exercised here.
    cfg.scheduler.cadence.clear();
    // Every-second submit cron. Six-field cron: sec min hour dom mon dow.
    cfg.scheduler.submit_cadence = Some("*/1 * * * * *".to_string());

    let counter = Arc::new(AtomicUsize::new(0));
    let mut sched = Scheduler::from_config_with_counter(tmp.path(), &cfg, Arc::clone(&counter))
        .await
        .expect("scheduler builds");

    sched.start().await.expect("scheduler starts");

    // Poll for up to 3 seconds for the first tick to land. tokio-cron-
    // scheduler aligns to wall-clock seconds, so 1–2 ticks is the typical
    // outcome inside a 3-second window.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if counter.load(Ordering::Relaxed) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let observed = counter.load(Ordering::Relaxed);
    assert!(
        observed >= 1,
        "expected at least one submit tick within 3s, got {observed}",
    );

    // Drain cleanly via the public shutdown API. Same drain budget as
    // `run_until_shutdown` uses on signal.
    sched.shutdown().await.expect("clean shutdown");
}
