//! External action state machine (PR 4+5).
//!
//! State transitions:
//! ```text
//! created → approved → leased → executing → succeeded | unknown
//! unknown → no_side_effect_confirmed → approved (fresh approval)
//! unknown → side_effect_confirmed
//! unknown → permanently_failed
//! approved → expired | rejected
//! ```
//!
//! Every crash, timeout, lost heartbeat, or lease expiry from `executing`
//! becomes `unknown`. Unknown cannot execute or be leased. It may return to
//! an executable state only after adapter-specific reconciliation proves
//! no side effect occurred.

use chrono::{DateTime, Utc};
use thiserror::Error;

/// Duration of one-time approval validity (10 minutes per design doc).
const APPROVAL_TTL_SECS: i64 = 600;

#[derive(Debug, Error)]
pub enum TransitionError {
    #[error("invalid transition: {from:?} → {to:?}")]
    InvalidTransition { from: ActionState, to: ActionState },
    #[error("approval has expired")]
    ApprovalExpired,
    #[error("cannot retry from {state:?} state")]
    CannotRetry { state: ActionState },
    #[error("fencing token mismatch")]
    FencingTokenMismatch,
}

/// States in the external action state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    Created,
    Approved,
    Leased,
    Executing,
    Succeeded,
    Unknown,
    NoSideEffectConfirmed,
    SideEffectConfirmed,
    PermanentlyFailed,
    Rejected,
    Expired,
}

impl ActionState {
    /// Check if a transition is valid.
    pub fn can_transition_to(self, to: ActionState) -> bool {
        match (self, to) {
            (ActionState::Created, ActionState::Approved | ActionState::Rejected)
            | (ActionState::Approved, ActionState::Leased | ActionState::Expired | ActionState::Rejected)
            // lease returned
            | (ActionState::Leased, ActionState::Executing | ActionState::Approved)
            | (ActionState::Executing, ActionState::Succeeded | ActionState::Unknown)
            | (ActionState::Unknown, ActionState::NoSideEffectConfirmed | ActionState::SideEffectConfirmed | ActionState::PermanentlyFailed)
            | (ActionState::NoSideEffectConfirmed, ActionState::Approved) => true,
            _ => false,
        }
    }

    /// Whether this is a terminal state (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ActionState::Succeeded
                | ActionState::SideEffectConfirmed
                | ActionState::PermanentlyFailed
                | ActionState::Rejected
                | ActionState::Expired
        )
    }

    /// Whether retry is allowed from this state (never from unknown).
    pub fn can_retry(self) -> bool {
        matches!(self, ActionState::NoSideEffectConfirmed)
    }
}

/// A transition request.
#[derive(Debug, Clone)]
pub struct ActionTransition {
    pub from: ActionState,
    pub to: ActionState,
    pub at: DateTime<Utc>,
    pub actor: String,
}

/// An external action record.
#[derive(Debug, Clone)]
pub struct ExternalAction {
    pub action_id: String,
    pub tenant_id: String,
    pub actor_id: String,
    pub action_type: String,
    pub payload_digest: String,
    pub state: ActionState,
    pub approval_actor: Option<String>,
    pub approval_time: Option<DateTime<Utc>>,
    pub approval_expiry: Option<DateTime<Utc>>,
    pub lease_token: Option<String>,
    pub fencing_token: Option<u64>,
    pub attempt_id: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub state_history: Vec<ActionTransition>,
}

impl ExternalAction {
    /// Create a new external action in the `created` state.
    pub fn new(tenant_id: &str, actor_id: &str, action_type: &str, payload_digest: &str) -> Self {
        let now = Utc::now();
        let action_id = generate_id();
        Self {
            action_id,
            tenant_id: tenant_id.to_string(),
            actor_id: actor_id.to_string(),
            action_type: action_type.to_string(),
            payload_digest: payload_digest.to_string(),
            state: ActionState::Created,
            approval_actor: None,
            approval_time: None,
            approval_expiry: None,
            lease_token: None,
            fencing_token: None,
            attempt_id: None,
            created_at: now,
            state_history: vec![],
        }
    }

    /// Approve the action. One-time, exact-payload, 10-minute expiry.
    pub fn approve(&mut self, actor: &str, at: DateTime<Utc>) -> Result<(), TransitionError> {
        self.transition_to(ActionState::Approved, actor, at)?;
        self.approval_actor = Some(actor.to_string());
        self.approval_time = Some(at);
        self.approval_expiry = Some(at + chrono::Duration::seconds(APPROVAL_TTL_SECS));
        Ok(())
    }

    /// Check if the approval has expired.
    pub fn is_approval_expired(&self, now: DateTime<Utc>) -> bool {
        self.approval_expiry.map_or(true, |exp| now > exp)
    }

    /// Lease the action for execution.
    pub fn lease(&mut self, worker: &str, at: DateTime<Utc>) -> Result<String, TransitionError> {
        if self.state == ActionState::Approved && self.is_approval_expired(at) {
            return Err(TransitionError::ApprovalExpired);
        }
        self.transition_to(ActionState::Leased, worker, at)?;
        let token = generate_id();
        self.lease_token = Some(token.clone());
        Ok(token)
    }

    /// Begin execution. Requires fencing token.
    pub fn begin_execution(
        &mut self,
        fencing_token: u64,
        attempt_id: u64,
        worker: &str,
        at: DateTime<Utc>,
    ) -> Result<(), TransitionError> {
        self.transition_to(ActionState::Executing, worker, at)?;
        self.fencing_token = Some(fencing_token);
        self.attempt_id = Some(attempt_id);
        Ok(())
    }

    /// Mark as succeeded.
    pub fn succeed(&mut self, worker: &str, at: DateTime<Utc>) -> Result<(), TransitionError> {
        self.transition_to(ActionState::Succeeded, worker, at)
    }

    /// Mark as unknown (crash/timeout during execution).
    /// This is the key safety state: no blind retry.
    pub fn mark_unknown(&mut self, worker: &str, at: DateTime<Utc>) -> Result<(), TransitionError> {
        self.transition_to(ActionState::Unknown, worker, at)?;
        self.lease_token = None;
        Ok(())
    }

    /// Reconciliation proved no side effect occurred.
    pub fn confirm_no_side_effect(
        &mut self,
        worker: &str,
        at: DateTime<Utc>,
    ) -> Result<(), TransitionError> {
        self.transition_to(ActionState::NoSideEffectConfirmed, worker, at)?;
        self.fencing_token = None;
        Ok(())
    }

    /// Reconciliation proved a side effect did occur.
    pub fn confirm_side_effect(
        &mut self,
        worker: &str,
        at: DateTime<Utc>,
    ) -> Result<(), TransitionError> {
        self.transition_to(ActionState::SideEffectConfirmed, worker, at)
    }

    /// Reconciliation failed to determine outcome.
    pub fn permanently_fail(
        &mut self,
        worker: &str,
        at: DateTime<Utc>,
    ) -> Result<(), TransitionError> {
        self.transition_to(ActionState::PermanentlyFailed, worker, at)
    }

    /// Reject the action.
    pub fn reject(&mut self, actor: &str, at: DateTime<Utc>) -> Result<(), TransitionError> {
        self.transition_to(ActionState::Rejected, actor, at)
    }

    fn transition_to(
        &mut self,
        to: ActionState,
        actor: &str,
        at: DateTime<Utc>,
    ) -> Result<(), TransitionError> {
        if !self.state.can_transition_to(to) {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to,
            });
        }
        self.state_history.push(ActionTransition {
            from: self.state,
            to,
            at,
            actor: actor.to_string(),
        });
        self.state = to;
        Ok(())
    }
}

fn generate_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "state_tests.rs"]
mod tests;
