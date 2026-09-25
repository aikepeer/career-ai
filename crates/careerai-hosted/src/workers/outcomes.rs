//! Manual outcome tracking (PR 7).
//!
//! Users manually record interactions (applications, interviews, emails,
//! calls) and set follow-up reminders. No automated mailbox sync or call
//! recording in the beta — outcomes are user-entered.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Type of tracked interaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeType {
    Application,
    Interview,
    Email,
    Call,
}

/// Status of a tracked outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Pending,
    Completed,
    NoResponse,
    Rejected,
    Offer,
    Declined,
}

/// A manually-recorded interaction with a recruiter or company.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualOutcome {
    pub id: uuid::Uuid,
    pub tenant_id: uuid::Uuid,
    pub application_id: Option<uuid::Uuid>,
    pub outcome_type: OutcomeType,
    pub status: OutcomeStatus,
    pub contact_name: Option<String>,
    pub contact_email: Option<String>,
    pub summary: String,
    pub occurred_at: DateTime<Utc>,
    pub follow_up: Option<FollowUpReminder>,
}

/// A follow-up reminder attached to an outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowUpReminder {
    pub due_at: DateTime<Utc>,
    pub note: String,
    pub completed: bool,
}

impl FollowUpReminder {
    /// Check if the reminder is overdue (past due and not completed).
    pub fn is_overdue(&self, now: DateTime<Utc>) -> bool {
        !self.completed && self.due_at < now
    }

    /// Check if the reminder is due soon (within the next 24 hours).
    pub fn is_due_soon(&self, now: DateTime<Utc>) -> bool {
        !self.completed && self.due_at >= now && self.due_at <= now + chrono::Duration::hours(24)
    }
}

/// Validate an outcome before persistence.
///
/// Returns `Ok(())` if the outcome is well-formed, or an error message
/// describing the validation failure.
pub fn validate_outcome(outcome: &ManualOutcome) -> Result<(), String> {
    if outcome.summary.trim().is_empty() {
        return Err("summary must not be empty".to_string());
    }

    if outcome.outcome_type == OutcomeType::Email && outcome.contact_email.is_none() {
        return Err("email outcomes require a contact_email".to_string());
    }

    if outcome.outcome_type == OutcomeType::Call && outcome.contact_name.is_none() {
        return Err("call outcomes require a contact_name".to_string());
    }

    if let Some(ref follow_up) = outcome.follow_up {
        if follow_up.note.trim().is_empty() {
            return Err("follow-up note must not be empty".to_string());
        }
    }

    Ok(())
}

/// Filter outcomes that have overdue follow-up reminders.
pub fn overdue_follow_ups(outcomes: &[ManualOutcome], now: DateTime<Utc>) -> Vec<&ManualOutcome> {
    outcomes
        .iter()
        .filter(|o| o.follow_up.as_ref().is_some_and(|f| f.is_overdue(now)))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn fixture_outcome(outcome_type: OutcomeType) -> ManualOutcome {
        ManualOutcome {
            id: uuid::Uuid::new_v4(),
            tenant_id: uuid::Uuid::new_v4(),
            application_id: None,
            outcome_type,
            status: OutcomeStatus::Pending,
            contact_name: Some("Jane".to_string()),
            contact_email: Some("jane@acme.com".to_string()),
            summary: "Phone screen with recruiter".to_string(),
            occurred_at: now(),
            follow_up: None,
        }
    }

    #[test]
    fn validate_email_requires_contact_email() {
        let mut o = fixture_outcome(OutcomeType::Email);
        o.contact_email = None;
        let err = validate_outcome(&o).unwrap_err();
        assert!(err.contains("contact_email"));
    }

    #[test]
    fn validate_call_requires_contact_name() {
        let mut o = fixture_outcome(OutcomeType::Call);
        o.contact_name = None;
        let err = validate_outcome(&o).unwrap_err();
        assert!(err.contains("contact_name"));
    }

    #[test]
    fn validate_rejects_empty_summary() {
        let mut o = fixture_outcome(OutcomeType::Application);
        o.summary = "   ".to_string();
        assert!(validate_outcome(&o).is_err());
    }

    #[test]
    fn validate_rejects_empty_follow_up_note() {
        let mut o = fixture_outcome(OutcomeType::Application);
        o.follow_up = Some(FollowUpReminder {
            due_at: now(),
            note: "  ".to_string(),
            completed: false,
        });
        assert!(validate_outcome(&o).is_err());
    }

    #[test]
    fn validate_accepts_well_formed_outcome() {
        let o = fixture_outcome(OutcomeType::Email);
        assert!(validate_outcome(&o).is_ok());
    }

    #[test]
    fn follow_up_overdue_detection() {
        let n = now();
        let reminder = FollowUpReminder {
            due_at: n - chrono::Duration::hours(1),
            note: "Send thank-you email".to_string(),
            completed: false,
        };
        assert!(reminder.is_overdue(n));
        assert!(!reminder.is_due_soon(n));
    }

    #[test]
    fn follow_up_due_soon_detection() {
        let n = now();
        let reminder = FollowUpReminder {
            due_at: n + chrono::Duration::hours(12),
            note: "Follow up on offer".to_string(),
            completed: false,
        };
        assert!(!reminder.is_overdue(n));
        assert!(reminder.is_due_soon(n));
    }

    #[test]
    fn follow_up_completed_not_overdue() {
        let n = now();
        let reminder = FollowUpReminder {
            due_at: n - chrono::Duration::hours(48),
            note: "Done".to_string(),
            completed: true,
        };
        assert!(!reminder.is_overdue(n));
    }

    #[test]
    fn overdue_follow_ups_filter() {
        let n = now();
        let mut outcomes: Vec<ManualOutcome> = Vec::new();

        let mut overdue = fixture_outcome(OutcomeType::Call);
        overdue.follow_up = Some(FollowUpReminder {
            due_at: n - chrono::Duration::days(1),
            note: "Overdue".to_string(),
            completed: false,
        });

        let mut future = fixture_outcome(OutcomeType::Call);
        future.follow_up = Some(FollowUpReminder {
            due_at: n + chrono::Duration::days(3),
            note: "Future".to_string(),
            completed: false,
        });

        outcomes.push(overdue);
        outcomes.push(future);

        let result = overdue_follow_ups(&outcomes, n);
        assert_eq!(result.len(), 1);
    }
}
