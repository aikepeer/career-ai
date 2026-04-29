use careerai_core::state::ListingState;
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KpiStrip {
    pub today_discovered: u64,
    pub shortlisted_active: u64,
    pub applied_lifetime: u64,
    pub response_rate_pct: Option<f32>,
    /// Pre-formatted label for the template — "—" when no applications
    /// have been submitted, otherwise the rounded percentage like "0%"
    /// or "12%". Avoids Tera's truthy-on-zero / null-vs-defined quirks.
    pub response_rate_label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ListingCard {
    pub title: String,
    pub company: String,
    pub score: Option<f32>,
    pub url: String,
    pub posted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FunnelColumn {
    pub state: ListingState,
    pub label: String,
    pub count: u64,
    pub top: Vec<ListingCard>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    Action,
    Warn,
    Info,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NextStepKind {
    ReadyToTailor { count: u64 },
    ReadyToRender { count: u64 },
    ReadyToApply { count: u64 },
    SourceStale { source: String, age_hours: u64 },
    LinkedInCookieExpiring { days_left: u64 },
    ProfileStale { age_days: u64 },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NextStep {
    pub kind: NextStepKind,
    pub label: String,
    pub urgency: Urgency,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PipelineSnapshot {
    pub kpi: KpiStrip,
    pub columns: Vec<FunnelColumn>,
    pub state_counts: StateCounts,
    pub source_lag_hours: Vec<(String, u64)>,
    pub linkedin_cookie_days_left: Option<u64>,
    pub profile_age_days: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct StateCounts {
    pub shortlisted: u64,
    pub tailored: u64,
    pub rendered: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexView {
    pub kpi: KpiStrip,
    pub columns: Vec<FunnelColumn>,
    pub next_steps: Vec<NextStep>,
    pub daemon_health: crate::daemon_health::DaemonHealth,
}
