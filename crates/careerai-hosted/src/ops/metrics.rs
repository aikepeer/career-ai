//! Beta exit metrics tracking with Prometheus exposition format.
//!
//! Metrics follow the event envelope from the design doc:
//! `{schema_version, event_name, occurred_at, tenant_metric_key,
//!  actor_type, consent_scope, source, object_type, object_id_hash,
//!  properties, sample_rate}`.
//!
//! Beta exit criteria metrics:
//! - onboarding_completed
//! - match_reviewed
//! - program_completed
//! - artifact_previewed
//! - outcome_recorded
//! - action_approved / action_denied
//! - usage_reserved / usage_finalized
//! - subscription_changed
//! - export_completed

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Metric event names defined in the design doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricEvent {
    OnboardingCompleted,
    MatchReviewed,
    ProgramCompleted,
    ArtifactPreviewed,
    OutcomeRecorded,
    ActionApproved,
    ActionDenied,
    UsageReserved,
    UsageFinalized,
    SubscriptionChanged,
    ExportCompleted,
}

impl MetricEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::OnboardingCompleted => "onboarding_completed",
            Self::MatchReviewed => "match_reviewed",
            Self::ProgramCompleted => "program_completed",
            Self::ArtifactPreviewed => "artifact_previewed",
            Self::OutcomeRecorded => "outcome_recorded",
            Self::ActionApproved => "action_approved",
            Self::ActionDenied => "action_denied",
            Self::UsageReserved => "usage_reserved",
            Self::UsageFinalized => "usage_finalized",
            Self::SubscriptionChanged => "subscription_changed",
            Self::ExportCompleted => "export_completed",
        }
    }
}

/// A single metric event record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricRecord {
    pub event_name: String,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub tenant_metric_key: String,
    pub actor_type: String,
    pub source: String,
    pub object_type: String,
    pub object_id_hash: String,
}

/// Counters for a single metric event, keyed by tenant metric key.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricCounter {
    pub total: u64,
    pub per_tenant: HashMap<String, u64>,
}

/// Central metrics collector for beta exit criteria.
///
/// Thread-safe via `std::sync::Mutex`. In production this is
/// backed by a time-series store; in beta it is in-memory.
#[derive(Debug)]
pub struct MetricsCollector {
    inner: std::sync::Mutex<MetricsInner>,
}

#[derive(Debug, Default)]
struct MetricsInner {
    counters: HashMap<&'static str, MetricCounter>,
    latency_samples: HashMap<String, Vec<f64>>,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    /// Create a new empty collector.
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(MetricsInner {
                counters: HashMap::new(),
                latency_samples: HashMap::new(),
            }),
        }
    }

    /// Convenience wrapper for tests.
    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Lock the inner state, recovering from poison.
    fn lock(&self) -> std::sync::MutexGuard<'_, MetricsInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record a metric event for a tenant.
    pub fn record(&self, event: MetricEvent, tenant_key: &str) {
        let mut inner = self.lock();
        let counter = inner.counters.entry(event.as_str()).or_default();
        counter.total += 1;
        *counter
            .per_tenant
            .entry(tenant_key.to_string())
            .or_insert(0) += 1;
    }

    /// Record an API latency sample in milliseconds.
    pub fn record_latency(&self, endpoint: &str, ms: f64) {
        let mut inner = self.lock();
        inner
            .latency_samples
            .entry(endpoint.to_string())
            .or_default()
            .push(ms);
    }

    /// Get the total count for a metric event.
    pub fn count(&self, event: MetricEvent) -> u64 {
        self.lock()
            .counters
            .get(event.as_str())
            .map_or(0, |c| c.total)
    }

    /// Get per-tenant count for a metric event.
    pub fn tenant_count(&self, event: MetricEvent, tenant_key: &str) -> u64 {
        self.lock()
            .counters
            .get(event.as_str())
            .and_then(|c| c.per_tenant.get(tenant_key).copied())
            .unwrap_or(0)
    }

    /// Get all distinct tenant keys that have recorded a given event.
    pub fn tenant_keys(&self, event: MetricEvent) -> Vec<String> {
        self.lock()
            .counters
            .get(event.as_str())
            .map(|c| c.per_tenant.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Get p95 latency for an endpoint.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn p95_latency(&self, endpoint: &str) -> Option<f64> {
        let inner = self.lock();
        let samples = inner.latency_samples.get(endpoint)?;
        if samples.is_empty() {
            return None;
        }
        let mut sorted = samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
        let idx = idx.min(sorted.len());
        sorted.get(idx.saturating_sub(1)).copied()
    }

    /// Render metrics in Prometheus text exposition format (v0.0.4).
    pub fn render_prometheus(&self) -> String {
        let inner = self.lock();
        let mut out = String::new();

        for (name, counter) in &inner.counters {
            let _ = writeln!(out, "# TYPE careerai_{name} counter");
            let _ = writeln!(
                out,
                "careerai_{name}_total {total}",
                name = name,
                total = counter.total
            );
            for (tenant, count) in &counter.per_tenant {
                let _ = writeln!(out, "careerai_{name}_total{{tenant=\"{tenant}\"}} {count}");
            }
        }

        for (endpoint, samples) in &inner.latency_samples {
            if let Some(p95) = percentile(samples, 0.95) {
                let _ = writeln!(
                    out,
                    "careerai_api_latency_ms{{endpoint=\"{endpoint}\",quantile=\"0.95\"}} {p95}",
                );
            }
            if let Some(p50) = percentile(samples, 0.50) {
                let _ = writeln!(
                    out,
                    "careerai_api_latency_ms{{endpoint=\"{endpoint}\",quantile=\"0.5\"}} {p50}",
                );
            }
        }

        out
    }

    /// Beta exit criteria snapshot for the design doc's metrics.
    pub fn beta_exit_snapshot(&self) -> BetaExitSnapshot {
        let inner = self.lock();
        BetaExitSnapshot {
            onboarding_completed: inner
                .counters
                .get(MetricEvent::OnboardingCompleted.as_str())
                .map_or(0, |c| c.per_tenant.len()),
            match_reviewed_total: inner
                .counters
                .get(MetricEvent::MatchReviewed.as_str())
                .map_or(0, |c| c.total),
            program_completed: inner
                .counters
                .get(MetricEvent::ProgramCompleted.as_str())
                .map_or(0, |c| c.per_tenant.len()),
            artifact_previewed_total: inner
                .counters
                .get(MetricEvent::ArtifactPreviewed.as_str())
                .map_or(0, |c| c.total),
            outcome_recorded_total: inner
                .counters
                .get(MetricEvent::OutcomeRecorded.as_str())
                .map_or(0, |c| c.total),
            export_completed_total: inner
                .counters
                .get(MetricEvent::ExportCompleted.as_str())
                .map_or(0, |c| c.total),
            action_approved: inner
                .counters
                .get(MetricEvent::ActionApproved.as_str())
                .map_or(0, |c| c.total),
            action_denied: inner
                .counters
                .get(MetricEvent::ActionDenied.as_str())
                .map_or(0, |c| c.total),
        }
    }
}

/// Snapshot of beta exit criteria metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetaExitSnapshot {
    pub onboarding_completed: usize,
    pub match_reviewed_total: u64,
    pub program_completed: usize,
    pub artifact_previewed_total: u64,
    pub outcome_recorded_total: u64,
    pub export_completed_total: u64,
    pub action_approved: u64,
    pub action_denied: u64,
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn percentile(samples: &[f64], p: f64) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f64) * p).ceil() as usize;
    let idx = idx.min(sorted.len());
    sorted.get(idx.saturating_sub(1)).copied()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn record_increments_counter() {
        let m = MetricsCollector::new();
        m.record(MetricEvent::OnboardingCompleted, "t1");
        m.record(MetricEvent::OnboardingCompleted, "t1");
        m.record(MetricEvent::OnboardingCompleted, "t2");
        assert_eq!(m.count(MetricEvent::OnboardingCompleted), 3);
        assert_eq!(m.tenant_count(MetricEvent::OnboardingCompleted, "t1"), 2);
        assert_eq!(m.tenant_count(MetricEvent::OnboardingCompleted, "t2"), 1);
    }

    #[test]
    fn tenant_keys_lists_all_tenants() {
        let m = MetricsCollector::new();
        m.record(MetricEvent::MatchReviewed, "t1");
        m.record(MetricEvent::MatchReviewed, "t2");
        m.record(MetricEvent::MatchReviewed, "t3");
        let mut keys = m.tenant_keys(MetricEvent::MatchReviewed);
        keys.sort();
        assert_eq!(keys, vec!["t1", "t2", "t3"]);
    }

    #[test]
    fn p95_latency_returns_expected_percentile() {
        let m = MetricsCollector::new();
        for ms in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0] {
            m.record_latency("/v1/listings", ms);
        }
        let p95 = m.p95_latency("/v1/listings").unwrap();
        assert!(p95 >= 90.0, "p95 should be >= 90, got {p95}");
    }

    #[test]
    fn prometheus_includes_counters() {
        let m = MetricsCollector::new();
        m.record(MetricEvent::OnboardingCompleted, "t1");
        let text = m.render_prometheus();
        assert!(text.contains("careerai_onboarding_completed_total 1"));
        assert!(text.contains("# TYPE careerai_onboarding_completed counter"));
    }

    #[test]
    fn beta_exit_snapshot_captures_key_metrics() {
        let m = MetricsCollector::new();
        m.record(MetricEvent::OnboardingCompleted, "t1");
        m.record(MetricEvent::OnboardingCompleted, "t2");
        m.record(MetricEvent::MatchReviewed, "t1");
        m.record(MetricEvent::MatchReviewed, "t2");
        m.record(MetricEvent::MatchReviewed, "t1");
        m.record(MetricEvent::ProgramCompleted, "t1");
        m.record(MetricEvent::ExportCompleted, "t1");

        let snap = m.beta_exit_snapshot();
        assert_eq!(snap.onboarding_completed, 2);
        assert_eq!(snap.match_reviewed_total, 3);
        assert_eq!(snap.program_completed, 1);
        assert_eq!(snap.export_completed_total, 1);
    }

    #[test]
    fn empty_collector_returns_zero_counts() {
        let m = MetricsCollector::new();
        assert_eq!(m.count(MetricEvent::MatchReviewed), 0);
        assert!(m.p95_latency("/v1/listings").is_none());
        let snap = m.beta_exit_snapshot();
        assert_eq!(snap.onboarding_completed, 0);
    }
}
