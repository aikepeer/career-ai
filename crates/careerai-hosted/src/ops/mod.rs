//! Hosted beta operations (PR 12).
//!
//! - Metrics collection with Prometheus exposition
//! - `careerai-export/v1` portable archive format
//! - Artifact object store with capability-based access
//! - Billing provider adapter (Mock + Stripe skeleton)

pub mod billing_adapter;
pub mod export_format;
pub mod metrics;
pub mod object_store;

pub use billing_adapter::{
    BillingAdapter, CheckoutSession, CheckoutSessionRequest, MockBillingAdapter,
    StripeBillingAdapter, SubscriptionStatus,
};
pub use export_format::{ExportBuilder, ExportFormatError, ExportManifest, RecordCounts};
pub use metrics::{BetaExitSnapshot, MetricEvent, MetricsCollector};
pub use object_store::{ArtifactStore, Capability, ObjectStoreError, ObjectVersion};
