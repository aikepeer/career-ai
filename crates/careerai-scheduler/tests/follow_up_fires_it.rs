//! Integration test — the follow-up cron job actually fires (not just registers).
//!
//! Sets `follow_up_cadence` to every-second, clears all other cadences so
//! the follow-up job is the ONLY registered job, starts the scheduler, and
//! asserts the shared tick counter increments. This proves the async closure
//! inside `build_follow_up_job` actually executes — the closure calls
//! `check_and_create_follow_ups` on a real pool, so if the counter increments,
//! the function was invoked.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use careerai_core::config::CoreConfig;
use careerai_scheduler::Scheduler;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn follow_up_cron_job_actually_fires() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = CoreConfig::load(tmp.path()).expect("load embedded defaults");

    // Clear all per-source cadences and submit cadence so the follow-up
    // job is the ONLY registered job. Any tick counter increment must
    // come from the follow-up job.
    cfg.scheduler.cadence.clear();
    cfg.scheduler.submit_cadence = None;
    cfg.scheduler.follow_up_cadence = Some("*/1 * * * * *".into());

    let counter = Arc::new(AtomicUsize::new(0));
    let mut sched = Scheduler::from_config_with_counter(tmp.path(), &cfg, Arc::clone(&counter))
        .await
        .expect("scheduler builds with follow_up_cadence");

    sched.start().await.expect("scheduler starts");

    // Wait up to 4s for the follow-up job to fire at least once.
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    while std::time::Instant::now() < deadline {
        if counter.load(Ordering::Relaxed) >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let observed = counter.load(Ordering::Relaxed);
    sched.shutdown().await.expect("clean shutdown");

    assert!(
        observed >= 1,
        "follow-up cron job never fired in 4s (counter={observed}). \
         The job was registered but its async closure did not execute.",
    );
}
