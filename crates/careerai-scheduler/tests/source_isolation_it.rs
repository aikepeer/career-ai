//! H6: Integration test — multiple per-source cron jobs fire independently.
//!
//! Verifies the scheduler's marquee invariant: registering multiple per-source
//! cron jobs against the same `JobScheduler` actually wires both into the
//! event loop and ticks them concurrently, neither starving the other.
//!
//! Stays fully offline by leaving all source companies empty — `discover_one`
//! returns an empty `DiscoveryReport` quickly (the "no sources enabled"
//! fall-through), `match_one` walks an empty `discovered` table. The closure
//! body still completes, so the shared `tick_counter` increments per fired
//! tick. With two jobs ticking once a second over a 3-second window we
//! expect at least 4 increments.
//!
//! What this DOESN'T cover: panic isolation between sibling cron jobs. That
//! would require injecting a `panic!` into a source adapter or pipeline
//! call, which needs test hooks the codebase doesn't currently expose.
//! Tracked as M6.1 follow-up. The structural property tested here — both
//! jobs fire, neither blocks the other — is the load-bearing piece of M6's
//! "graceful degradation" design.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use careerai_core::config::CoreConfig;
use careerai_scheduler::Scheduler;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multiple_per_source_jobs_fire_independently() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = CoreConfig::load(tmp.path()).expect("load embedded defaults");

    // Wipe the embedded cadence and install two every-second per-source jobs.
    // Six-field cron: sec min hour dom mon dow.
    cfg.scheduler.cadence.clear();
    cfg.scheduler
        .cadence
        .insert("greenhouse".into(), "*/1 * * * * *".into());
    cfg.scheduler
        .cadence
        .insert("lever".into(), "*/1 * * * * *".into());
    cfg.scheduler.submit_cadence = None;

    // Disable the actual source adapters so neither tick hits the network.
    // `discover_one` will fall through to the "no sources enabled" branch
    // and return Ok(DiscoveryReport::default()).
    cfg.sources.greenhouse.companies.clear();
    cfg.sources.lever.companies.clear();
    cfg.sources.remotive.enabled = false;
    cfg.sources.remoteok.enabled = false;
    cfg.sources.naukri.enabled = false;

    let counter = Arc::new(AtomicUsize::new(0));
    let mut sched = Scheduler::from_config_with_counter(tmp.path(), &cfg, Arc::clone(&counter))
        .await
        .expect("scheduler builds");

    sched.start().await.expect("scheduler starts");

    // Wait up to 4s for the counter to reach 4 (two jobs × ~2 ticks each in
    // a 3-second wall-clock window, with slack for cron-aligned firing).
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while std::time::Instant::now() < deadline {
        if counter.load(Ordering::Relaxed) >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let observed = counter.load(Ordering::Relaxed);
    sched.shutdown().await.expect("clean shutdown");

    assert!(
        observed >= 4,
        "expected ≥4 ticks across two per-source jobs in ~3s; got {observed}. \
         If <4, one job is starving the other (or one didn't register at all).",
    );
}
