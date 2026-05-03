#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::limiter::RateLimiter;
use super::permit::RatePermit;
use super::types::*;

/// Test policy: no min-interval, no jitter, no quiet hours.
fn test_policy(max_per_day: u32) -> RatePolicy {
    RatePolicy {
        max_per_day,
        min_seconds_between: 0,
        jitter_seconds: 0,
        quiet_hours_utc: None,
    }
}

#[test]
fn quiet_window_non_wrapping() {
    assert!(!in_window(8, 9, 17));
    assert!(in_window(9, 9, 17));
    assert!(in_window(12, 9, 17));
    assert!(!in_window(17, 9, 17));
}

#[test]
fn quiet_window_wrapping_midnight() {
    assert!(in_window(23, 22, 7));
    assert!(in_window(0, 22, 7));
    assert!(in_window(6, 22, 7));
    assert!(!in_window(7, 22, 7));
    assert!(!in_window(12, 22, 7));
    assert!(in_window(22, 22, 7));
}

#[tokio::test]
async fn day_cap_returns_error_when_exceeded() {
    let rl = RateLimiter::new();
    let policy = test_policy(2);
    rl.acquire("test", &policy).await.unwrap().commit();
    rl.acquire("test", &policy).await.unwrap().commit();
    let err = rl.acquire("test", &policy).await.unwrap_err();
    match err {
        RateLimitError::DayCap { source_name, cap } => {
            assert_eq!(source_name, "test");
            assert_eq!(cap, 2);
        }
        RateLimitError::QuietHours { .. } => panic!("unexpected QuietHours"),
    }
}

#[tokio::test]
async fn quiet_hours_returns_error_immediately() {
    let rl = RateLimiter::new();
    let policy = RatePolicy {
        max_per_day: 5,
        min_seconds_between: 0,
        jitter_seconds: 0,
        quiet_hours_utc: Some((0, 24)),
    };
    let err = rl.acquire("test", &policy).await.unwrap_err();
    assert!(matches!(err, RateLimitError::QuietHours { .. }));
}

#[tokio::test]
async fn dropped_permit_refunds_day_cap_slot() {
    let rl = RateLimiter::new();
    let policy = test_policy(2);
    for _ in 0..5 {
        let _permit = rl.acquire("test", &policy).await.unwrap();
    }
    rl.acquire("test", &policy).await.unwrap().commit();
    rl.acquire("test", &policy).await.unwrap().commit();
    let err = rl.acquire("test", &policy).await.unwrap_err();
    assert!(matches!(err, RateLimitError::DayCap { .. }));
}

#[tokio::test]
async fn committed_permit_consumes_day_cap_slot() {
    let rl = RateLimiter::new();
    let policy = test_policy(1);
    rl.acquire("test", &policy).await.unwrap().commit();
    let err = rl.acquire("test", &policy).await.unwrap_err();
    assert!(matches!(err, RateLimitError::DayCap { .. }));
}

#[tokio::test]
async fn refund_after_day_rollover_does_not_undercount_new_day() {
    let rl = RateLimiter::new();
    let policy = test_policy(3);

    let permit = rl.acquire("test", &policy).await.unwrap();

    let yesterday = {
        let map = rl.inner.lock().await;
        let entry = map.get("test").unwrap();
        UtcDay {
            year: entry.day.year,
            ordinal: entry.day.ordinal.saturating_sub(1).max(1),
        }
    };
    permit.commit();

    let real_today_permit = rl.acquire("test", &policy).await.unwrap();
    let count_before_drop = {
        let map = rl.inner.lock().await;
        map.get("test").unwrap().count_today
    };

    {
        let _stale = RatePermit::new(&rl, "test".to_string(), yesterday);
    }

    let count_after_drop = {
        let map = rl.inner.lock().await;
        map.get("test").unwrap().count_today
    };
    assert_eq!(
        count_before_drop, count_after_drop,
        "stale-day refund must not decrement today's counter"
    );

    real_today_permit.commit();
}

#[tokio::test]
async fn min_seconds_between_zero_skips_governor_wait() {
    let rl = RateLimiter::new();
    let policy = test_policy(10);
    let start = std::time::Instant::now();
    for _ in 0..10 {
        rl.acquire("test", &policy).await.unwrap().commit();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 200,
        "acquire burst took {}ms — governor wait was not skipped",
        elapsed.as_millis()
    );
}
