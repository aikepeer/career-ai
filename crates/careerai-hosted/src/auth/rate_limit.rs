//! Abuse rate limiting for login, recovery, and import endpoints.
//!
//! Simple in-memory token-bucket rate limiter per (IP, endpoint) key.
//! Prevents brute-force attacks on magic links, TOTP, and recovery codes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Rate limit decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitDecision {
    Allow,
    Deny,
}

/// Configuration for a rate-limited endpoint.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum requests in the burst window.
    pub max_requests: u32,
    /// Burst window duration.
    pub window: Duration,
}

impl RateLimitConfig {
    /// Login endpoint: 5 requests per 5 minutes per IP.
    pub fn login() -> Self {
        Self {
            max_requests: 5,
            window: Duration::from_secs(300),
        }
    }

    /// Recovery endpoint: 3 requests per 15 minutes per IP.
    pub fn recovery() -> Self {
        Self {
            max_requests: 3,
            window: Duration::from_secs(900),
        }
    }

    /// Import endpoint: 5 requests per 10 minutes per IP.
    pub fn import() -> Self {
        Self {
            max_requests: 5,
            window: Duration::from_secs(600),
        }
    }
}

/// In-memory rate limiter tracking request counts per key.
#[derive(Debug)]
pub struct RateLimiter {
    /// (ip, endpoint) -> (count, window_start)
    buckets: HashMap<String, (u32, Instant)>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }

    /// Check and consume a rate limit token for the given key.
    /// Returns `Deny` if the limit has been exceeded.
    pub fn check(&mut self, key: &str, config: &RateLimitConfig) -> RateLimitDecision {
        let now = Instant::now();
        let entry = self.buckets.entry(key.to_string()).or_insert((0, now));

        // Reset window if expired
        if now.duration_since(entry.1) > config.window {
            *entry = (0, now);
        }

        if entry.0 >= config.max_requests {
            return RateLimitDecision::Deny;
        }

        entry.0 += 1;
        RateLimitDecision::Allow
    }

    /// Remove expired entries to prevent unbounded memory growth.
    pub fn evict_expired(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.buckets
            .retain(|_, (_, start)| now.duration_since(*start) < max_age);
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn allows_requests_within_limit() {
        let mut limiter = RateLimiter::new();
        let config = RateLimitConfig {
            max_requests: 3,
            window: Duration::from_secs(60),
        };
        for _ in 0..3 {
            assert_eq!(
                limiter.check("ip1:login", &config),
                RateLimitDecision::Allow
            );
        }
    }

    #[test]
    fn denies_after_limit_exceeded() {
        let mut limiter = RateLimiter::new();
        let config = RateLimitConfig {
            max_requests: 2,
            window: Duration::from_secs(60),
        };
        assert_eq!(
            limiter.check("ip1:login", &config),
            RateLimitDecision::Allow
        );
        assert_eq!(
            limiter.check("ip1:login", &config),
            RateLimitDecision::Allow
        );
        assert_eq!(limiter.check("ip1:login", &config), RateLimitDecision::Deny);
    }

    #[test]
    fn different_keys_are_independent() {
        let mut limiter = RateLimiter::new();
        let config = RateLimitConfig {
            max_requests: 1,
            window: Duration::from_secs(60),
        };
        assert_eq!(
            limiter.check("ip1:login", &config),
            RateLimitDecision::Allow
        );
        assert_eq!(
            limiter.check("ip2:login", &config),
            RateLimitDecision::Allow
        );
    }

    #[test]
    fn window_resets_after_expiry() {
        let mut limiter = RateLimiter::new();
        let config = RateLimitConfig {
            max_requests: 1,
            window: Duration::from_millis(50),
        };
        assert_eq!(
            limiter.check("ip1:login", &config),
            RateLimitDecision::Allow
        );
        assert_eq!(limiter.check("ip1:login", &config), RateLimitDecision::Deny);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(
            limiter.check("ip1:login", &config),
            RateLimitDecision::Allow
        );
    }

    #[test]
    fn login_config_allows_5_per_5min() {
        let config = RateLimitConfig::login();
        assert_eq!(config.max_requests, 5);
        assert_eq!(config.window, Duration::from_secs(300));
    }

    #[test]
    fn recovery_config_allows_3_per_15min() {
        let config = RateLimitConfig::recovery();
        assert_eq!(config.max_requests, 3);
        assert_eq!(config.window, Duration::from_secs(900));
    }
}
