//! Billing webhook contract: signature verification, dedup, reconciliation.
//!
//! Webhooks are signature-verified, persisted before processing, deduplicated
//! by `(provider, event_id)`, and applied by event version/time rules.
//! Out-of-order events trigger reconciliation.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashSet;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum BillingError {
    #[error("webhook signature verification failed")]
    InvalidSignature,
    #[error("duplicate webhook event: ({provider}, {event_id})")]
    Duplicate { provider: String, event_id: String },
    #[error("event out of order: expected version {expected}, got {actual}")]
    OutOfOrder { expected: u32, actual: u32 },
    #[error("unknown event type: {0}")]
    UnknownEventType(String),
}

/// Status of a billing event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingEventStatus {
    /// Received but not yet processed.
    Pending,
    /// Successfully processed.
    Processed,
    /// Out of order — awaiting reconciliation.
    AwaitingReconciliation,
    /// Failed processing.
    Failed,
}

/// A billing event from a webhook.
#[derive(Debug, Clone)]
pub struct BillingEvent {
    pub provider: String,
    pub event_id: String,
    pub event_type: String,
    pub event_version: u32,
    pub customer_id: String,
    pub subscription_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub status: BillingEventStatus,
    pub raw_payload: String,
}

/// Webhook deduplication tracker.
#[derive(Debug, Default)]
pub struct WebhookDedup {
    seen: HashSet<String>,
}

impl WebhookDedup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check and record a webhook event.
    /// Returns `Ok(())` if new, `Err(Duplicate)` if already seen.
    pub fn check(&mut self, provider: &str, event_id: &str) -> Result<(), BillingError> {
        let key = format!("{provider}:{event_id}");
        if self.seen.contains(&key) {
            return Err(BillingError::Duplicate {
                provider: provider.to_string(),
                event_id: event_id.to_string(),
            });
        }
        self.seen.insert(key);
        Ok(())
    }

    /// Number of unique events tracked.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether no events have been tracked.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

/// Verify a webhook signature.
/// Uses HMAC-SHA256 with the provider's webhook secret.
pub fn verify_signature(
    payload: &str,
    signature: &str,
    secret: &[u8],
) -> Result<(), BillingError> {
    let mut mac = HmacSha256::new_from_slice(secret).unwrap_or_else(|_| unreachable!("HMAC accepts any key length"));
    mac.update(payload.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());
    if expected != signature {
        return Err(BillingError::InvalidSignature);
    }
    Ok(())
}

/// Process a billing event with ordering check.
/// Events must be processed in version order; out-of-order events
/// trigger reconciliation.
pub fn process_event(
    event: &mut BillingEvent,
    last_processed_version: u32,
) -> Result<(), BillingError> {
    if event.event_version < last_processed_version {
        event.status = BillingEventStatus::AwaitingReconciliation;
        return Err(BillingError::OutOfOrder {
            expected: last_processed_version,
            actual: event.event_version,
        });
    }
    event.status = BillingEventStatus::Processed;
    Ok(())
}

/// Billing event types from payment providers.
pub mod event_types {
    pub const SUBSCRIPTION_CREATED: &str = "subscription.created";
    pub const SUBSCRIPTION_UPDATED: &str = "subscription.updated";
    pub const SUBSCRIPTION_DELETED: &str = "subscription.deleted";
    pub const PAYMENT_SUCCEEDED: &str = "payment.succeeded";
    pub const PAYMENT_FAILED: &str = "payment.failed";
    pub const INVOICE_PAID: &str = "invoice.paid";
    pub const INVOICE_PAYMENT_FAILED: &str = "invoice.payment_failed";
}

/// Plan change rules:
/// - Upgrade takes effect after verified payment
/// - Downgrade at period end
/// - Failed payment retains read/export/delete access but blocks new metered work
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanChangeEffect {
    /// Upgrade: effective immediately after payment.
    Immediate,
    /// Downgrade: effective at period end.
    AtPeriodEnd,
    /// Payment failed: read-only access with metered work blocked.
    ReadOnly,
}

/// Determine the effect of a plan change event.
pub fn plan_change_effect(
    new_plan_rank: u32,
    current_plan_rank: u32,
    payment_succeeded: bool,
) -> PlanChangeEffect {
    if !payment_succeeded {
        return PlanChangeEffect::ReadOnly;
    }
    if new_plan_rank > current_plan_rank {
        PlanChangeEffect::Immediate
    } else {
        PlanChangeEffect::AtPeriodEnd
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn dedup_accepts_new_event() {
        let mut dedup = WebhookDedup::new();
        dedup.check("stripe", "evt_1").unwrap();
    }

    #[test]
    fn dedup_rejects_duplicate() {
        let mut dedup = WebhookDedup::new();
        dedup.check("stripe", "evt_1").unwrap();
        let err = dedup.check("stripe", "evt_1").unwrap_err();
        assert!(matches!(err, BillingError::Duplicate { .. }));
    }

    #[test]
    fn dedup_allows_same_id_different_provider() {
        let mut dedup = WebhookDedup::new();
        dedup.check("stripe", "evt_1").unwrap();
        dedup.check("paddle", "evt_1").unwrap();
    }

    #[test]
    fn signature_verification_accepts_correct() {
        let payload = r#"{"event":"test"}"#;
        let secret = b"whsec_test";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload.as_bytes());
        let sig = hex::encode(mac.finalize().into_bytes());
        verify_signature(payload, &sig, secret).unwrap();
    }

    #[test]
    fn signature_verification_rejects_wrong() {
        let payload = r#"{"event":"test"}"#;
        let secret = b"whsec_test";
        let wrong_sig = "deadbeef".repeat(16);
        assert!(verify_signature(payload, &wrong_sig, secret).is_err());
    }

    #[test]
    fn out_of_order_event_triggers_reconciliation() {
        let mut event = BillingEvent {
            provider: "stripe".into(),
            event_id: "evt_1".into(),
            event_type: event_types::PAYMENT_SUCCEEDED.into(),
            event_version: 1,
            customer_id: "c1".into(),
            subscription_id: "s1".into(),
            amount_cents: 1500,
            currency: "usd".into(),
            occurred_at: Utc::now(),
            received_at: Utc::now(),
            status: BillingEventStatus::Pending,
            raw_payload: "{}".into(),
        };
        // Last processed was version 5, this is version 1
        let err = process_event(&mut event, 5).unwrap_err();
        assert!(matches!(err, BillingError::OutOfOrder { .. }));
        assert_eq!(event.status, BillingEventStatus::AwaitingReconciliation);
    }

    #[test]
    fn in_order_event_processes() {
        let mut event = BillingEvent {
            provider: "stripe".into(),
            event_id: "evt_1".into(),
            event_type: event_types::PAYMENT_SUCCEEDED.into(),
            event_version: 3,
            customer_id: "c1".into(),
            subscription_id: "s1".into(),
            amount_cents: 1500,
            currency: "usd".into(),
            occurred_at: Utc::now(),
            received_at: Utc::now(),
            status: BillingEventStatus::Pending,
            raw_payload: "{}".into(),
        };
        process_event(&mut event, 2).unwrap();
        assert_eq!(event.status, BillingEventStatus::Processed);
    }

    #[test]
    fn plan_upgrade_is_immediate() {
        assert_eq!(
            plan_change_effect(2, 1, true),
            PlanChangeEffect::Immediate
        );
    }

    #[test]
    fn plan_downgrade_at_period_end() {
        assert_eq!(
            plan_change_effect(1, 2, true),
            PlanChangeEffect::AtPeriodEnd
        );
    }

    #[test]
    fn payment_failed_is_readonly() {
        assert_eq!(
            plan_change_effect(2, 1, false),
            PlanChangeEffect::ReadOnly
        );
    }
}
