//! Per-source circuit breaker.
//!
//! Wraps any [`Source`] so that repeated discovery failures don't cascade.
//! After `threshold` consecutive failures the breaker trips to [`State::Open`]
//! and short-circuits all subsequent calls for `cooldown`. After the cooldown
//! elapses it transitions to [`State::HalfOpen`] and allows a single probe;
//! success closes the breaker, failure re-opens it.
//!
//! Ported from job_agentic's circuit-breaker pattern. The breaker is
//! time-aware via an injectable [`Clock`] so tests can advance time without
//! sleeping.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::base::{RawListing, Source, SourceError};

/// States follow the classic circuit-breaker state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Normal operation — calls pass through.
    Closed,
    /// Tripped — calls are rejected for `cooldown`.
    Open,
    /// Cooldown elapsed — one probe call is allowed.
    HalfOpen,
}

/// Abstraction over the wall clock so tests can fast-forward.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

/// Real wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug)]
struct Inner {
    state: State,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

impl Inner {
    const fn new() -> Self {
        Self {
            state: State::Closed,
            consecutive_failures: 0,
            opened_at: None,
        }
    }
}

/// Configuration for the circuit breaker.
#[derive(Debug, Clone, Copy)]
pub struct BreakerConfig {
    /// Consecutive failures before the breaker trips to `Open`.
    pub threshold: u32,
    /// How long to reject calls before probing again.
    pub cooldown: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            threshold: 5,
            cooldown: Duration::from_secs(300),
        }
    }
}

/// Error returned when the breaker is open and the call is rejected.
#[derive(Debug, thiserror::Error)]
#[error(
    "circuit breaker open for source {source_name} — {consecutive_failures} consecutive failures"
)]
pub struct BreakerOpen {
    pub source_name: String,
    pub consecutive_failures: u32,
}

/// A decorator around any [`Source`] that trips after repeated failures.
///
/// Thread-safe via an internal `Mutex`.
#[derive(Debug)]
pub struct CircuitBreakerSource<S, C = SystemClock> {
    inner: S,
    config: BreakerConfig,
    clock: C,
    state: Mutex<Inner>,
}

impl<S: Source> CircuitBreakerSource<S, SystemClock> {
    /// Wrap `inner` with the given config and the real wall clock.
    #[must_use]
    pub fn new(inner: S, config: BreakerConfig) -> Self {
        Self::with_clock(inner, config, SystemClock)
    }
}

#[allow(clippy::unwrap_used)]
impl<S: Source, C: Clock> CircuitBreakerSource<S, C> {
    /// Wrap `inner` with a custom clock (for tests).
    #[must_use]
    pub fn with_clock(inner: S, config: BreakerConfig, clock: C) -> Self {
        Self {
            inner,
            config,
            clock,
            state: Mutex::new(Inner::new()),
        }
    }

    /// Current breaker state (for observability / dashboard).
    pub fn state(&self) -> State {
        self.resolve_state();
        self.state.lock().unwrap().state
    }

    /// Consecutive failure count (resets on any success).
    pub fn consecutive_failures(&self) -> u32 {
        self.state.lock().unwrap().consecutive_failures
    }

    /// Force-reset the breaker to `Closed` (manual override / kill-switch).
    pub fn reset(&self) {
        let mut s = self.state.lock().unwrap();
        s.state = State::Closed;
        s.consecutive_failures = 0;
        s.opened_at = None;
    }

    /// Returns `Ok(())` if the call should proceed, `Err` if the breaker
    /// is open. Mutates state to `HalfOpen` when the cooldown has elapsed.
    fn allow(&self) -> Result<(), BreakerOpen> {
        let mut s = self.state.lock().unwrap();
        match s.state {
            State::Closed => Ok(()),
            State::Open => {
                let elapsed = self.clock.now().duration_since(s.opened_at.unwrap());
                if elapsed >= self.config.cooldown {
                    s.state = State::HalfOpen;
                    Ok(())
                } else {
                    Err(BreakerOpen {
                        source_name: self.inner.name().to_string(),
                        consecutive_failures: s.consecutive_failures,
                    })
                }
            }
            State::HalfOpen => {
                // Allow the probe through.
                Ok(())
            }
        }
    }

    fn on_success(&self) {
        let mut s = self.state.lock().unwrap();
        s.state = State::Closed;
        s.consecutive_failures = 0;
        s.opened_at = None;
    }

    fn on_failure(&self) {
        let mut s = self.state.lock().unwrap();
        s.consecutive_failures += 1;
        if s.state == State::HalfOpen {
            // Probe failed — re-open immediately.
            s.state = State::Open;
            s.opened_at = Some(self.clock.now());
        } else if s.consecutive_failures >= self.config.threshold {
            s.state = State::Open;
            s.opened_at = Some(self.clock.now());
        }
    }

    /// Re-evaluate whether the cooldown has elapsed and transition
    /// `Open` → `HalfOpen` if so. Called before reporting state so the
    /// dashboard sees the live state without needing a call to trigger it.
    fn resolve_state(&self) {
        let mut s = self.state.lock().unwrap();
        if s.state == State::Open {
            let elapsed = self.clock.now().duration_since(s.opened_at.unwrap());
            if elapsed >= self.config.cooldown {
                s.state = State::HalfOpen;
            }
        }
    }
}

#[async_trait]
impl<S: Source, C: Clock> Source for CircuitBreakerSource<S, C> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
        self.allow()
            .map_err(|_e| SourceError::Parse("circuit breaker open".into()))?;
        match self.inner.discover().await {
            Ok(listings) => {
                self.on_success();
                Ok(listings)
            }
            Err(err) => {
                self.on_failure();
                Err(err)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// A controllable clock for deterministic tests. Cloneable so the
    /// test harness can keep a copy and advance time while the breaker
    /// owns its own clone — both share the same internal instant.
    #[derive(Debug, Clone)]
    struct FakeClock {
        inner: Arc<Mutex<Instant>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                inner: Arc::new(Mutex::new(Instant::now())),
            }
        }

        fn advance(&self, dur: Duration) {
            let mut s = self.inner.lock().unwrap();
            *s += dur;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.inner.lock().unwrap()
        }
    }

    /// A source that fails N times then succeeds, or always fails.
    struct FlakySource {
        name: &'static str,
        fail_count: AtomicU32,
        fail_times: u32,
    }

    impl FlakySource {
        fn always_fail(name: &'static str) -> Self {
            Self {
                name,
                fail_count: AtomicU32::new(0),
                fail_times: u32::MAX,
            }
        }

        fn fail_n_then_succeed(name: &'static str, n: u32) -> Self {
            Self {
                name,
                fail_count: AtomicU32::new(0),
                fail_times: n,
            }
        }
    }

    #[async_trait]
    impl Source for FlakySource {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn discover(&self) -> Result<Vec<RawListing>, SourceError> {
            let n = self.fail_count.fetch_add(1, Ordering::SeqCst);
            if n < self.fail_times {
                Err(SourceError::Parse(format!("fail #{n}")))
            } else {
                Ok(vec![])
            }
        }
    }

    fn config(threshold: u32, cooldown: Duration) -> BreakerConfig {
        BreakerConfig {
            threshold,
            cooldown,
        }
    }

    #[tokio::test]
    async fn passes_through_when_under_threshold() {
        let src = FlakySource::fail_n_then_succeed("test", 2);
        let breaker = CircuitBreakerSource::with_clock(
            src,
            config(5, Duration::from_secs(60)),
            FakeClock::new(),
        );
        assert_eq!(breaker.state(), State::Closed);

        // Two failures, then a success.
        let _ = breaker.discover().await;
        let _ = breaker.discover().await;
        let _ = breaker.discover().await;

        assert_eq!(breaker.state(), State::Closed);
        assert_eq!(breaker.consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn trips_after_threshold_consecutive_failures() {
        let src = FlakySource::always_fail("test");
        let breaker = CircuitBreakerSource::with_clock(
            src,
            config(3, Duration::from_secs(60)),
            FakeClock::new(),
        );

        let _ = breaker.discover().await; // fail 1
        let _ = breaker.discover().await; // fail 2
        assert_eq!(breaker.state(), State::Closed);

        let _ = breaker.discover().await; // fail 3 → trip
        assert_eq!(breaker.state(), State::Open);
        assert_eq!(breaker.consecutive_failures(), 3);
    }

    #[tokio::test]
    async fn rejects_calls_while_open() {
        let src = FlakySource::always_fail("test");
        let breaker = CircuitBreakerSource::with_clock(
            src,
            config(1, Duration::from_secs(60)),
            FakeClock::new(),
        );

        let _ = breaker.discover().await; // trip immediately
        assert_eq!(breaker.state(), State::Open);

        let result = breaker.discover().await;
        assert!(result.is_err());
        // The inner source should NOT have been called again.
        assert_eq!(breaker.consecutive_failures(), 1);
    }

    #[tokio::test]
    async fn state_reports_half_open_after_cooldown() {
        let clock = FakeClock::new();
        let src = FlakySource::always_fail("test");
        let breaker = CircuitBreakerSource::with_clock(
            src,
            config(1, Duration::from_secs(30)),
            clock.clone(),
        );

        // Trip the breaker.
        let _ = breaker.discover().await;
        assert_eq!(breaker.state(), State::Open);

        // Advance time past cooldown — state() should reflect HalfOpen.
        clock.advance(Duration::from_secs(31));
        assert_eq!(breaker.state(), State::HalfOpen);
    }

    #[tokio::test]
    async fn probe_succeeds_after_cooldown_closes_breaker() {
        let clock = FakeClock::new();
        let src = FlakySource::fail_n_then_succeed("test", 1);
        let breaker = CircuitBreakerSource::with_clock(
            src,
            config(1, Duration::from_secs(30)),
            clock.clone(),
        );

        // Trip the breaker with the one failure.
        let _ = breaker.discover().await;
        assert_eq!(breaker.state(), State::Open);

        // Advance past cooldown.
        clock.advance(Duration::from_secs(31));

        // Probe — source now succeeds.
        let result = breaker.discover().await;
        assert!(result.is_ok());
        assert_eq!(breaker.state(), State::Closed);
        assert_eq!(breaker.consecutive_failures(), 0);
    }

    #[tokio::test]
    async fn probe_failure_after_cooldown_reopens_breaker() {
        let clock = FakeClock::new();
        let src = FlakySource::always_fail("test");
        let breaker = CircuitBreakerSource::with_clock(
            src,
            config(1, Duration::from_secs(30)),
            clock.clone(),
        );

        // Trip.
        let _ = breaker.discover().await;
        assert_eq!(breaker.state(), State::Open);

        // Advance past cooldown.
        clock.advance(Duration::from_secs(31));

        // Probe — still failing.
        let _ = breaker.discover().await;
        assert_eq!(breaker.state(), State::Open);
    }

    #[tokio::test]
    async fn reset_clears_state() {
        let src = FlakySource::always_fail("test");
        let breaker = CircuitBreakerSource::with_clock(
            src,
            config(1, Duration::from_secs(60)),
            FakeClock::new(),
        );

        let _ = breaker.discover().await;
        assert_eq!(breaker.state(), State::Open);

        breaker.reset();
        assert_eq!(breaker.state(), State::Closed);
        assert_eq!(breaker.consecutive_failures(), 0);
    }
}
