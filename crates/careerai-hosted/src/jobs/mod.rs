//! Durable jobs, outbox, and worker leases (PR 4+5).
//!
//! Queue delivery is at-least-once with leases, fencing, exponential backoff
//! (maximum 5 attempts), dead-letter after poison-job classification, and
//! manual replay only after policy revalidation.

pub mod queue;

pub use queue::{JobConfig, JobState, JobTransitionError, LeaseInfo};
