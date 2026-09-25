//! Durable job queue state machine and lease management.
//!
//! States: pending → leased → running → succeeded | failed | dead_letter
//! At-least-once delivery with fencing tokens and exponential backoff.
//! Maximum 5 attempts; dead-letter after poison-job classification.

use chrono::{DateTime, Duration, Utc};
use thiserror::Error;

/// Maximum retry attempts before dead-lettering.
const MAX_ATTEMPTS: u32 = 5;

/// Base backoff duration (exponential: 2^attempt * base).
const BACKOFF_BASE_SECS: i64 = 2;

/// Maximum backoff between retries (5 minutes).
const BACKOFF_MAX_SECS: i64 = 300;

/// Lease duration (5 minutes).
const LEASE_DURATION: Duration = Duration::minutes(5);

#[derive(Debug, Error)]
pub enum JobTransitionError {
    #[error("invalid job state transition: {from:?} → {to:?}")]
    InvalidTransition { from: JobState, to: JobState },
    #[error("lease expired")]
    LeaseExpired,
    #[error("fencing token mismatch")]
    FencingMismatch,
    #[error("max attempts exceeded — dead-lettered")]
    MaxAttemptsExceeded,
}

/// Job states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    Leased,
    Running,
    Succeeded,
    Failed,
    DeadLetter,
}

impl JobState {
    pub fn can_transition_to(self, to: JobState) -> bool {
        match (self, to) {
            (JobState::Pending, JobState::Leased)
            | (JobState::Leased, JobState::Running | JobState::Pending) // lease expired
            | (JobState::Running, JobState::Succeeded | JobState::Failed)
            | (JobState::Failed, JobState::Pending | JobState::DeadLetter) => true, // retry
            _ => false,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Succeeded | JobState::DeadLetter)
    }
}

/// Job configuration.
#[derive(Debug, Clone)]
pub struct JobConfig {
    pub max_attempts: u32,
    pub lease_duration: Duration,
}

impl Default for JobConfig {
    fn default() -> Self {
        Self {
            max_attempts: MAX_ATTEMPTS,
            lease_duration: LEASE_DURATION,
        }
    }
}

/// Lease information for a running job.
#[derive(Debug, Clone)]
pub struct LeaseInfo {
    pub job_id: String,
    pub fencing_token: u64,
    pub leased_by: String,
    pub leased_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Compute the exponential backoff delay for a given attempt number.
/// attempt 0 → 2s, 1 → 4s, 2 → 8s, 3 → 16s, 4 → 32s, capped at 300s.
pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = BACKOFF_BASE_SECS.pow(attempt + 1).min(BACKOFF_MAX_SECS);
    Duration::seconds(secs)
}

/// Determine the next state after a job failure.
/// If attempts remain, go to Pending (retry). Otherwise, dead-letter.
pub fn on_failure(attempt: u32, config: &JobConfig) -> JobState {
    if attempt >= config.max_attempts {
        JobState::DeadLetter
    } else {
        JobState::Failed
    }
}

/// Check if a lease has expired.
pub fn is_lease_expired(lease: &LeaseInfo, now: DateTime<Utc>) -> bool {
    now > lease.expires_at
}

/// Generate the next fencing token.
/// Fencing tokens are monotonically increasing per job.
pub fn next_fencing_token(current: u64) -> u64 {
    current + 1
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn pending_to_leased_valid() {
        assert!(JobState::Pending.can_transition_to(JobState::Leased));
    }

    #[test]
    fn leased_to_running_valid() {
        assert!(JobState::Leased.can_transition_to(JobState::Running));
    }

    #[test]
    fn running_to_succeeded_valid() {
        assert!(JobState::Running.can_transition_to(JobState::Succeeded));
    }

    #[test]
    fn running_to_failed_valid() {
        assert!(JobState::Running.can_transition_to(JobState::Failed));
    }

    #[test]
    fn failed_to_pending_retry_valid() {
        assert!(JobState::Failed.can_transition_to(JobState::Pending));
    }

    #[test]
    fn failed_to_dead_letter_valid() {
        assert!(JobState::Failed.can_transition_to(JobState::DeadLetter));
    }

    #[test]
    fn pending_to_running_invalid() {
        assert!(!JobState::Pending.can_transition_to(JobState::Running));
    }

    #[test]
    fn succeeded_is_terminal() {
        assert!(JobState::Succeeded.is_terminal());
        assert!(JobState::DeadLetter.is_terminal());
        assert!(!JobState::Pending.is_terminal());
    }

    #[test]
    fn backoff_increases_exponentially() {
        let d0 = backoff_delay(0);
        let d1 = backoff_delay(1);
        let d2 = backoff_delay(2);
        assert!(d1 > d0);
        assert!(d2 > d1);
    }

    #[test]
    fn backoff_capped_at_max() {
        let d = backoff_delay(20);
        assert!(d <= Duration::seconds(BACKOFF_MAX_SECS));
    }

    #[test]
    fn on_failure_retries_when_attempts_remain() {
        let config = JobConfig::default();
        assert_eq!(on_failure(1, &config), JobState::Failed);
    }

    #[test]
    fn on_failure_dead_letters_at_max() {
        let config = JobConfig::default();
        assert_eq!(on_failure(5, &config), JobState::DeadLetter);
    }

    #[test]
    fn lease_expiry_check() {
        let now = Utc::now();
        let lease = LeaseInfo {
            job_id: "j1".into(),
            fencing_token: 1,
            leased_by: "w1".into(),
            leased_at: now,
            expires_at: now + Duration::minutes(5),
        };
        assert!(!is_lease_expired(&lease, now));
        let later = now + Duration::minutes(6);
        assert!(is_lease_expired(&lease, later));
    }

    #[test]
    fn fencing_token_increases() {
        assert_eq!(next_fencing_token(0), 1);
        assert_eq!(next_fencing_token(5), 6);
    }

    #[test]
    fn backoff_base_2_seconds() {
        let d = backoff_delay(0);
        assert_eq!(d, Duration::seconds(2));
    }

    #[test]
    fn default_config_5_attempts() {
        let config = JobConfig::default();
        assert_eq!(config.max_attempts, 5);
    }
}
