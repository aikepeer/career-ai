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
    pub id: String,
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
    pub drafted: u64,
    pub total_tailored: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EventLogItem {
    pub id: i64,
    pub listing_id: String,
    pub from_state: Option<String>,
    pub to_state: String,
    pub note: Option<String>,
    pub summary: String,
    pub raw_payload: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub relative_time: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConfigSourceItem {
    pub name: String,
    pub kind: String,
    pub listing_count: u64,
    pub last_sync: Option<DateTime<Utc>>,
    pub last_sync_label: String,
    pub status: String,
    pub url: Option<String>,
    /// Whether live submission is enabled for this source
    /// (`submit.per_source.<source>.enabled`).
    pub submit_enabled: bool,
    /// ATS HTTP rate cap. `0` means "use the default" (shown as "—").
    pub max_per_day: u32,
    /// ATS HTTP minimum interval between submissions (seconds).
    pub min_seconds_between: u32,
    /// ATS HTTP quiet-hours window, if any.
    pub quiet_hours_utc: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KeywordStatus {
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CareerStoryItem {
    pub kind: String,
    pub era: String,
    pub title: String,
    pub organization: String,
    pub proof: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProfileView {
    pub name: String,
    pub email: String,
    pub phone: String,
    pub location: String,
    pub github: String,
    pub linkedin: String,
    pub portfolio: String,
    pub summary: String,
    pub target_roles: Vec<String>,
    pub languages: Vec<String>,
    pub platforms: Vec<String>,
    pub frameworks: Vec<String>,
    pub devops: Vec<String>,
    pub tools: Vec<String>,
    pub debugging: Vec<String>,
    pub protocols: Vec<String>,
    pub skill_count: usize,
    pub experience_count: usize,
    pub education_count: usize,
    pub career_story: Vec<CareerStoryItem>,
    pub raw_yaml: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConfigView {
    pub score_threshold: f32,
    pub must_include_skills: Vec<String>,
    pub keywords: Vec<KeywordStatus>,
    pub sources: Vec<ConfigSourceItem>,
    pub llm_provider: String,
    pub llm_model: String,
    pub llm_status: String,
    pub rate_limit_per_min: u32,
    pub prompt_version: String,
    pub profile: Option<ProfileView>,
    pub llm_backend: String,
    pub llm_strategy: String,
    pub llm_api_base: Option<String>,
    pub llm_timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ActionItem {
    pub id: String,
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub state: String,
    pub score: Option<f32>,
    pub action_type: String,
    pub action_label: String,
    pub action_url: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ArtifactItem {
    pub kind: String,
    pub path: String,
    pub bytes: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ApplicationDetail {
    pub id: String,
    pub listing_id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub url: String,
    pub description: String,
    pub state: String,
    pub score: Option<f32>,
    pub source: String,
    pub external_id: String,
    pub profile_hash: String,
    pub prompt_version: String,
    pub llm_model: String,
    pub resume_view_json: Option<String>,
    pub cover_letter_text: Option<String>,
    pub artifacts: Vec<ArtifactItem>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Audit trail for this listing, oldest → newest, for the modal's
    /// pipeline timeline.
    pub timeline: Vec<EventLogItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DiscoveredExplorerItem {
    pub id: String,
    pub title: String,
    pub company: String,
    pub location: Option<String>,
    pub source: String,
    pub state: String,
    pub score: Option<f32>,
    pub is_remote: bool,
    pub url: String,
    pub application_id: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub legitimacy: String,
    pub eligibility: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexView {
    pub kpi: KpiStrip,
    pub columns: Vec<FunnelColumn>,
    pub state_counts: StateCounts,
    pub next_steps: Vec<NextStep>,
    pub daemon_health: crate::daemon_health::DaemonHealth,
    pub llm_health: crate::llm_health::LlmHealth,
    pub recent_events: Vec<EventLogItem>,
    pub config: ConfigView,
    pub action_items: Vec<ActionItem>,
    pub discovered_explorer: Vec<DiscoveredExplorerItem>,
    /// Total listing count (cheap COUNT) for the badge. The actual rows
    /// load lazily via `/api/v1/explorer` after the page renders.
    pub explorer_total: u64,
    /// True when the explorer rows were NOT included in the initial SSR
    /// render (lazy-load mode). The template shows a loading placeholder.
    pub explorer_lazy: bool,
    /// LLM cost + token savings summary (from costs.jsonl).
    pub llm_cost: LlmCostSummary,
    /// Content library stats (cover letters + bullets indexed by domain).
    pub content_library: ContentLibraryView,
}

/// Aggregate LLM cost + token-savings summary, read from the JSONL
/// cost log. Populated server-side so the KPI strip renders immediately.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LlmCostSummary {
    pub total_cost_usd: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub call_count: u64,
    pub model: String,
    /// Estimated savings from cache hits + similarity/content-library reuse
    /// (calls that did NOT go to the provider). Each avoided call is
    /// valued at the average cost per live call.
    pub estimated_savings_usd: f64,
}

/// Content library stats for the dashboard: how many cover letters and
/// bullets are indexed, broken down by domain + role.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct ContentLibraryView {
    pub bullet_count: u64,
    pub cover_letter_count: u64,
    pub domains: Vec<ContentDomainEntry>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ContentDomainEntry {
    pub domain: String,
    pub role: String,
    pub bullets: u64,
    pub cover_letters: u64,
}

// ---- Feature view structs ----

/// Source attribution: per-source ranking by response rate.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SourceAttributionView {
    pub source: String,
    pub discovered: i64,
    pub submitted: i64,
    pub responded: i64,
    pub response_rate: f64,
}

/// Skill gap heatmap entry.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SkillGapViewEntry {
    pub skill: String,
    pub frequency: usize,
    pub percentage: f32,
    pub category: String,
}

/// Application quality score display.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct QualityScoreView {
    pub overall: f32,
    pub jd_relevance: f32,
    pub skill_coverage: f32,
    pub cover_letter_depth: f32,
    pub bullet_density: f32,
    pub recommendations: Vec<String>,
}

/// Follow-up summary for the dashboard.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FollowUpView {
    pub id: i64,
    pub application_id: String,
    pub listing_id: String,
    pub company: String,
    pub title: String,
    pub status: String,
    pub scheduled_at: String,
    /// R18: full body, not a truncated preview. The client decides how
    /// much to display; the full draft is needed for editing.
    pub body: String,
    /// R18: cadence step (1 = first reminder, 2 = second, etc.).
    #[serde(default)]
    pub cadence_step: i64,
}

/// Referral opportunity.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReferralView {
    pub listing_id: String,
    pub company: String,
    pub title: String,
    pub url: String,
    pub connection_source: String,
}
