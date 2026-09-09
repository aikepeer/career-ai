//! Hand-rolled SQL queries split by entity table. No `query!` macro
//! so we avoid the DATABASE_URL/.sqlx-cache compile-time dance for
//! now. Switch to compile-time-checked queries (M3+) once the schema
//! stabilizes.
//!
//! The module structure mirrors the schema:
//!
//! | Module        | Tables touched |
//! |---|---|
//! | `listings`     | `listings`, `events` (the latter only via `transition`) |
//! | `applications` | `applications`, `listings`, `events` (lockstep transitions) |
//! | `linkedin`     | `applications` ⨝ `listings` (LinkedIn drafts review flow) |
//! | `payloads`     | `application_payloads` |
//! | `artifacts`    | `artifacts` |
//! | `events`       | `events` (read-only; writes live next to their triggers) |
//!
//! `pub use submodule::*` keeps the legacy flat path
//! `careerai_db::queries::find_by_id` working unchanged for every
//! caller in the workspace.

pub mod analytics;
pub mod applications;
pub mod applications_sync;
pub mod artifacts;
pub mod command_log;
pub mod content_library;
pub mod employer_outcomes;
pub mod events;
pub mod followups;
pub mod interview_feedback;
pub mod linkedin;
pub mod listings;
pub mod market_pulse;
pub mod match_reasons;
pub mod patterns;
pub mod payloads;
pub mod quality;
pub mod salary;
pub mod saved_views;
pub mod similarity;
pub mod submission_attempts;
pub mod timing;
pub mod variants;
pub mod workspace_io;

pub use analytics::{
    query_intelligence_records, query_source_performance, ApplicationIntelligenceRecord,
    SourcePerformance,
};
pub use applications::{
    create_application, find_application_by_id, find_latest_application_for_listing,
    has_application_for_company, list_applications_by_state, set_application_state,
};
pub use applications_sync::{
    list_applications_by_state_and_source, transition_application_and_listing,
};
pub use artifacts::{all_artifact_paths, attach_artifact, list_artifacts};
pub use command_log::{list_recent_commands, log_command, CommandLogEntry};
pub use content_library::{
    fetch_bullets_for_domain, fetch_cover_letter_for_domain, library_stats, store_bullet,
    store_cover_letter, DomainEntry, LibraryStats,
};
pub use employer_outcomes::{
    delete_outcome, list_recent_outcomes, outcomes_for_application, outcomes_for_listing,
    record_outcome, EmployerOutcome, OUTCOME_TYPES,
};
pub use events::{
    count_submissions_since, events_for, latest_submission_time, list_recent_events,
    list_stale_submissions, StaleSubmission,
};
pub use followups::{
    create_follow_up, dismiss_follow_up, list_pending_follow_ups,
    list_pending_follow_ups_with_listing, mark_follow_up_handled, mark_follow_up_sent,
    snooze_follow_up, update_follow_up_body, FollowUp, FollowUpWithListing,
};
pub use interview_feedback::{
    feedback_for_listing, list_feedback, save_feedback, InterviewFeedback,
};
pub use linkedin::{claim_drafted_application, list_drafted_linkedin};
pub use listings::{
    find_by_external_id, find_by_id, insert_or_ignore, list_by_state, set_score, transition,
    transition_if,
};
pub use market_pulse::{weekly_market_summary, CompanyHiring, MarketPulse, SkillFrequency};
pub use match_reasons::{fetch_match_reasons, upsert_match_reasons, MatchReasonRow};
pub use patterns::{
    advance_rates, detect_reposts, funnel_velocity, rejection_latencies, AdvanceRate,
    FunnelVelocity, RejectionLatency, Repost,
};
pub use payloads::{find_payload_by_application_id, write_payload};
pub use quality::{fetch_quality_score, store_quality_score, QualityScoreRow};
pub use salary::{
    list_salary_ranges, salary_stats_by_role, store_salary_range, SalaryRange, SalaryStats,
};
pub use saved_views::{delete_saved_view, list_saved_views, upsert_saved_view, SavedView};
pub use similarity::{find_similar_tailored, record_similarity_index, SimilarTailoredResult};
pub use submission_attempts::{
    claim_submission_attempt, fetch_attempt_by_id, latest_attempt_for_application,
    list_uncertain_attempts, mark_attempt_failed, mark_attempt_submitted, mark_attempt_uncertain,
    SubmissionAttempt,
};
pub use timing::{
    best_submission_windows, submission_timing_stats, DayOfWeekStats, TimeWindowStats,
};
pub use variants::{
    list_variants_with_outcome, record_variant, update_variant_response, VariantWithOutcome,
};

#[cfg(test)]
pub(crate) mod test_support {
    use crate::models::{NewApplication, NewListing};

    pub fn fixture(source: &str, ext: &str) -> NewListing {
        NewListing {
            source: source.into(),
            external_id: ext.into(),
            title: "Senior ML Engineer".into(),
            company: "Acme Robotics".into(),
            location: Some("Remote".into()),
            url: format!("https://example.com/{ext}"),
            description: "Build embedded LLM perception systems.".into(),
            raw_json: None,
        }
    }

    pub fn new_app(listing_id: &str) -> NewApplication {
        NewApplication {
            listing_id: listing_id.into(),
            profile_hash: "sha256:abc".into(),
            prompt_version: "tailor.v1".into(),
            llm_model: "claude-3-5-sonnet-20241022".into(),
        }
    }
}
