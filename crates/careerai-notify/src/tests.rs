#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::dispatcher::{Notifier, Pipeline};
use crate::event::NotifyEvent;
use crate::severity::Severity;
use crate::NotifyError;

#[derive(Debug)]
struct CountingNotifier {
    name: &'static str,
    delivered: Arc<AtomicUsize>,
    delay_ms: u64,
    fail: bool,
}

#[async_trait]
impl Notifier for CountingNotifier {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn notify(&self, _event: &NotifyEvent, _severity: Severity) -> Result<(), NotifyError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        if self.fail {
            return Err(NotifyError::Channel {
                channel: self.name,
                reason: "synthetic failure".into(),
            });
        }
        self.delivered.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn ev() -> NotifyEvent {
    NotifyEvent::SourceUnreachable {
        source: "test".into(),
        reason: "boom".into(),
    }
}

#[tokio::test]
async fn fire_runs_all_channels_concurrently_and_swallows_errors() {
    let ok = Arc::new(AtomicUsize::new(0));
    let slow_ok = Arc::new(AtomicUsize::new(0));
    let pipe = Pipeline::with_channels(
        vec![
            Arc::new(CountingNotifier {
                name: "ok",
                delivered: ok.clone(),
                delay_ms: 0,
                fail: false,
            }),
            Arc::new(CountingNotifier {
                name: "broken",
                delivered: Arc::new(AtomicUsize::new(0)),
                delay_ms: 0,
                fail: true,
            }),
            Arc::new(CountingNotifier {
                name: "slow",
                delivered: slow_ok.clone(),
                delay_ms: 50,
                fail: false,
            }),
        ],
        Severity::Info,
    );
    // Should not bubble despite the failing channel.
    pipe.fire(ev(), Severity::Warning).await;
    assert_eq!(ok.load(Ordering::SeqCst), 1);
    assert_eq!(slow_ok.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fire_drops_events_below_min_severity() {
    let ok = Arc::new(AtomicUsize::new(0));
    let pipe = Pipeline::with_channels(
        vec![Arc::new(CountingNotifier {
            name: "ok",
            delivered: ok.clone(),
            delay_ms: 0,
            fail: false,
        })],
        Severity::Critical,
    );
    pipe.fire(ev(), Severity::Info).await;
    pipe.fire(ev(), Severity::Warning).await;
    assert_eq!(ok.load(Ordering::SeqCst), 0);
    pipe.fire(ev(), Severity::Critical).await;
    assert_eq!(ok.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fire_is_cheap_with_no_channels() {
    let pipe = Pipeline::with_channels(vec![], Severity::Info);
    // Just shouldn't panic / deadlock.
    pipe.fire(ev(), Severity::Critical).await;
    assert_eq!(pipe.channel_count(), 0);
}

/// A panicking channel must NOT crash the daemon tick: the spawned
/// task is isolated, the JoinError gets warn-logged, sibling
/// channels still deliver.
#[derive(Debug)]
struct PanickingNotifier;

#[async_trait]
impl Notifier for PanickingNotifier {
    fn name(&self) -> &'static str {
        "panicker"
    }
    async fn notify(&self, _event: &NotifyEvent, _severity: Severity) -> Result<(), NotifyError> {
        panic!("synthetic panic inside notifier");
    }
}

#[tokio::test]
async fn fire_swallows_channel_panics() {
    let ok = Arc::new(AtomicUsize::new(0));
    let pipe = Pipeline::with_channels(
        vec![
            Arc::new(PanickingNotifier),
            Arc::new(CountingNotifier {
                name: "ok",
                delivered: ok.clone(),
                delay_ms: 0,
                fail: false,
            }),
        ],
        Severity::Info,
    );
    // Must not propagate the panic; sibling channel still delivers.
    pipe.fire(ev(), Severity::Critical).await;
    assert_eq!(ok.load(Ordering::SeqCst), 1);
}
