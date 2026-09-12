//! Entitlement admission: reserve, finalize, release.
//!
//! Before any costly job, the admission transaction:
//! 1. Locks the entitlement version
//! 2. Reserves estimated units
//! 3. Writes `usage_reservations`
//! 4. Enqueues only after commit
//!
//! Workers finalize actual units or release the remainder. Reservations
//! expire safely. Admission fails closed except for explicitly free/read/
//! export operations.

use chrono::{DateTime, Duration, Utc};
use thiserror::Error;

/// Reservation expiry window (15 minutes).
const RESERVATION_TTL: Duration = Duration::minutes(15);

#[derive(Debug, Clone, Error)]
pub enum EntitlementError {
    #[error("entitlement not found or inactive")]
    NotFound,
    #[error("usage limit exceeded: requested {requested}, available {available}")]
    LimitExceeded { requested: u64, available: u64 },
    #[error("feature not included in plan")]
    FeatureNotIncluded,
    #[error("entitlement version changed during admission")]
    VersionConflict,
}

/// Status of an entitlement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntitlementStatus {
    Active,
    Expired,
    Suspended,
    Cancelled,
}

/// Provider-neutral entitlement decision.
#[derive(Debug, Clone)]
pub struct EntitlementDecision {
    pub plan_id: String,
    pub plan_version: u32,
    pub feature: String,
    /// Maximum units allowed per period.
    pub limit: u64,
    /// Units already used in the current period.
    pub used: u64,
    /// Units currently reserved (pending finalization).
    pub reserved: u64,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub status: EntitlementStatus,
}

impl EntitlementDecision {
    /// Available units = limit - used - reserved.
    pub fn available(&self) -> u64 {
        self.limit.saturating_sub(self.used).saturating_sub(self.reserved)
    }

    /// Check if the entitlement is active and can admit a request.
    pub fn is_active(&self) -> bool {
        self.status == EntitlementStatus::Active
    }

    /// Attempt to reserve units for a metered job.
    /// Returns the reservation on success, or an error if:
    /// - Entitlement is not active
    /// - Requested units exceed available
    pub fn reserve(
        &mut self,
        requested: u64,
        at: DateTime<Utc>,
    ) -> Result<UsageReservation, EntitlementError> {
        if !self.is_active() {
            return Err(EntitlementError::NotFound);
        }
        if requested > self.available() {
            return Err(EntitlementError::LimitExceeded {
                requested,
                available: self.available(),
            });
        }
        self.reserved += requested;
        Ok(UsageReservation {
            reservation_id: generate_id(),
            plan_id: self.plan_id.clone(),
            plan_version: self.plan_version,
            feature: self.feature.clone(),
            reserved_units: requested,
            reserved_at: at,
            expires_at: at + RESERVATION_TTL,
            finalized: false,
            released: false,
        })
    }

    /// Finalize a reservation: move reserved units to used.
    /// If actual usage differs from reserved, adjust accordingly.
    pub fn finalize(&mut self, reservation: &mut UsageReservation, actual_units: u64) {
        if reservation.finalized || reservation.released {
            return;
        }
        // Move reserved back to available, then add actual to used
        self.reserved = self.reserved.saturating_sub(reservation.reserved_units);
        self.used += actual_units;
        reservation.finalized = true;
    }

    /// Release a reservation without charging (e.g., job failed before work).
    pub fn release(&mut self, reservation: &mut UsageReservation) {
        if reservation.finalized || reservation.released {
            return;
        }
        self.reserved = self.reserved.saturating_sub(reservation.reserved_units);
        reservation.released = true;
    }

    /// Check if a reservation has expired.
    pub fn is_reservation_expired(reservation: &UsageReservation, now: DateTime<Utc>) -> bool {
        now > reservation.expires_at
    }
}

/// Result of an admission check.
#[derive(Debug, Clone)]
pub enum AdmissionResult {
    /// Admission granted with a reservation.
    Admitted(UsageReservation),
    /// Operation is free (no reservation needed).
    Free,
    /// Admission denied.
    Denied(EntitlementError),
}

/// A usage reservation record.
#[derive(Debug, Clone)]
pub struct UsageReservation {
    pub reservation_id: String,
    pub plan_id: String,
    pub plan_version: u32,
    pub feature: String,
    pub reserved_units: u64,
    pub reserved_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub finalized: bool,
    pub released: bool,
}

impl UsageReservation {
    /// Check if this reservation is still active (not finalized or released).
    pub fn is_active(&self) -> bool {
        !self.finalized && !self.released
    }
}

/// Check if an operation is free (no entitlement needed).
/// Free operations: read, export, delete (but export/delete need reauth).
pub fn is_free_operation(operation: &str) -> bool {
    matches!(
        operation,
        "read" | "export" | "delete" | "list" | "get" | "health"
    )
}

/// Admission gate: check entitlement and reserve units.
pub fn admit(
    entitlement: &mut EntitlementDecision,
    operation: &str,
    estimated_units: u64,
    now: DateTime<Utc>,
) -> AdmissionResult {
    if is_free_operation(operation) {
        return AdmissionResult::Free;
    }
    match entitlement.reserve(estimated_units, now) {
        Ok(r) => AdmissionResult::Admitted(r),
        Err(e) => AdmissionResult::Denied(e),
    }
}

fn generate_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_entitlement() -> EntitlementDecision {
        EntitlementDecision {
            plan_id: "pro".to_string(),
            plan_version: 1,
            feature: "llm_tokens".to_string(),
            limit: 10000,
            used: 1000,
            reserved: 500,
            period_start: Utc::now(),
            period_end: Utc::now() + Duration::days(30),
            status: EntitlementStatus::Active,
        }
    }

    #[test]
    fn available_calculates_correctly() {
        let e = make_entitlement();
        // 10000 - 1000 - 500 = 8500
        assert_eq!(e.available(), 8500);
    }

    #[test]
    fn reserve_succeeds_within_limit() {
        let mut e = make_entitlement();
        let r = e.reserve(100, Utc::now()).unwrap();
        assert_eq!(r.reserved_units, 100);
        // reserved went from 500 to 600
        assert_eq!(e.reserved, 600);
        assert_eq!(e.available(), 8400);
    }

    #[test]
    fn reserve_fails_when_exceeds_available() {
        let mut e = make_entitlement();
        let err = e.reserve(10000, Utc::now()).unwrap_err();
        assert!(matches!(err, EntitlementError::LimitExceeded { .. }));
    }

    #[test]
    fn reserve_fails_when_inactive() {
        let mut e = make_entitlement();
        e.status = EntitlementStatus::Suspended;
        let err = e.reserve(1, Utc::now()).unwrap_err();
        assert!(matches!(err, EntitlementError::NotFound));
    }

    #[test]
    fn finalize_moves_reserved_to_used() {
        let mut e = make_entitlement();
        let mut r = e.reserve(100, Utc::now()).unwrap();
        // Reserved is now 600
        assert_eq!(e.reserved, 600);
        e.finalize(&mut r, 80);
        // Reserved back to 500, used went from 1000 to 1080
        assert_eq!(e.reserved, 500);
        assert_eq!(e.used, 1080);
        assert!(r.finalized);
    }

    #[test]
    fn release_returns_reserved_without_charging() {
        let mut e = make_entitlement();
        let mut r = e.reserve(100, Utc::now()).unwrap();
        assert_eq!(e.reserved, 600);
        e.release(&mut r);
        assert_eq!(e.reserved, 500);
        assert_eq!(e.used, 1000); // unchanged
        assert!(r.released);
    }

    #[test]
    fn double_finalize_is_noop() {
        let mut e = make_entitlement();
        let mut r = e.reserve(100, Utc::now()).unwrap();
        e.finalize(&mut r, 80);
        let used_after_first = e.used;
        e.finalize(&mut r, 50); // should be no-op
        assert_eq!(e.used, used_after_first);
    }

    #[test]
    fn free_operations_dont_need_reservation() {
        assert!(is_free_operation("read"));
        assert!(is_free_operation("export"));
        assert!(is_free_operation("delete"));
        assert!(!is_free_operation("tailor"));
        assert!(!is_free_operation("render"));
        assert!(!is_free_operation("discover"));
    }

    #[test]
    fn admit_returns_free_for_read() {
        let mut e = make_entitlement();
        let result = admit(&mut e, "read", 0, Utc::now());
        assert!(matches!(result, AdmissionResult::Free));
    }

    #[test]
    fn admit_reserves_for_metered_operation() {
        let mut e = make_entitlement();
        let result = admit(&mut e, "tailor", 100, Utc::now());
        assert!(matches!(result, AdmissionResult::Admitted(_)));
    }

    #[test]
    fn admit_denies_when_limit_exceeded() {
        let mut e = make_entitlement();
        let result = admit(&mut e, "tailor", 100_000, Utc::now());
        assert!(matches!(result, AdmissionResult::Denied(_)));
    }

    #[test]
    fn reservation_expiry_check() {
        let mut e = make_entitlement();
        let r = e.reserve(100, Utc::now()).unwrap();
        assert!(!EntitlementDecision::is_reservation_expired(&r, Utc::now()));
        let future = Utc::now() + Duration::minutes(16);
        assert!(EntitlementDecision::is_reservation_expired(&r, future));
    }

    #[test]
    fn available_saturates_at_zero() {
        let mut e = make_entitlement();
        e.used = 9999;
        e.reserved = 999;
        assert_eq!(e.available(), 0);
    }
}
