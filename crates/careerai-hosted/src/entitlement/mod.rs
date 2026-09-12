//! Entitlements, usage reservation, and billing (PR 4+5).
//!
//! Provider-neutral entitlement admission: reserve estimated units → enqueue
//! after commit → finalize actual units or release remainder. Admission fails
//! closed except for explicitly free/read/export operations.

pub mod admission;
pub mod billing;

pub use admission::{EntitlementDecision, AdmissionResult, UsageReservation, EntitlementStatus};
pub use billing::{BillingEvent, BillingEventStatus, WebhookDedup};
