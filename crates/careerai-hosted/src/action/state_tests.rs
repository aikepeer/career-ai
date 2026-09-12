use super::*;

#[test]
fn full_happy_path() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    assert_eq!(action.state, ActionState::Approved);

    let _lease = action.lease("worker1", now).unwrap();
    assert_eq!(action.state, ActionState::Leased);

    action.begin_execution(1, 1, "worker1", now).unwrap();
    assert_eq!(action.state, ActionState::Executing);

    action.succeed("worker1", now).unwrap();
    assert_eq!(action.state, ActionState::Succeeded);
    assert!(action.state.is_terminal());
}

#[test]
fn crash_produces_unknown() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    action.lease("worker1", now).unwrap();
    action.begin_execution(1, 1, "worker1", now).unwrap();

    // Worker crashes
    action.mark_unknown("worker1", now).unwrap();
    assert_eq!(action.state, ActionState::Unknown);
}

#[test]
fn unknown_cannot_retry_directly() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    action.lease("worker1", now).unwrap();
    action.begin_execution(1, 1, "worker1", now).unwrap();
    action.mark_unknown("worker1", now).unwrap();

    // Cannot lease or execute from unknown
    assert!(!ActionState::Unknown.can_transition_to(ActionState::Leased));
    assert!(!ActionState::Unknown.can_transition_to(ActionState::Executing));
}

#[test]
fn unknown_reconciliation_no_side_effect() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    action.lease("worker1", now).unwrap();
    action.begin_execution(1, 1, "worker1", now).unwrap();
    action.mark_unknown("worker1", now).unwrap();

    // Reconciliation proves no side effect
    action.confirm_no_side_effect("worker1", now).unwrap();
    assert_eq!(action.state, ActionState::NoSideEffectConfirmed);

    // Can get fresh approval
    assert!(ActionState::NoSideEffectConfirmed.can_transition_to(ActionState::Approved));
}

#[test]
fn unknown_reconciliation_side_effect_confirmed() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    action.lease("worker1", now).unwrap();
    action.begin_execution(1, 1, "worker1", now).unwrap();
    action.mark_unknown("worker1", now).unwrap();

    action.confirm_side_effect("worker1", now).unwrap();
    assert_eq!(action.state, ActionState::SideEffectConfirmed);
    assert!(action.state.is_terminal());
}

#[test]
fn unknown_permanently_failed() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    action.lease("worker1", now).unwrap();
    action.begin_execution(1, 1, "worker1", now).unwrap();
    action.mark_unknown("worker1", now).unwrap();

    action.permanently_fail("worker1", now).unwrap();
    assert_eq!(action.state, ActionState::PermanentlyFailed);
    assert!(action.state.is_terminal());
}

#[test]
fn approval_expiry_blocks_lease() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    // 11 minutes later
    let later = now + chrono::Duration::minutes(11);
    let err = action.lease("worker1", later).unwrap_err();
    assert!(matches!(err, TransitionError::ApprovalExpired));
}

#[test]
fn reject_from_created() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();
    action.reject("u1", now).unwrap();
    assert_eq!(action.state, ActionState::Rejected);
    assert!(action.state.is_terminal());
}

#[test]
fn invalid_transition_rejected() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();
    // Cannot go from created directly to executing
    let err = action.begin_execution(1, 1, "w1", now).unwrap_err();
    assert!(matches!(err, TransitionError::InvalidTransition { .. }));
}

#[test]
fn state_history_recorded() {
    let mut action = ExternalAction::new("t1", "u1", "submit", "abc123");
    let now = Utc::now();

    action.approve("u1", now).unwrap();
    action.lease("w1", now).unwrap();

    assert_eq!(action.state_history.len(), 2);
    assert_eq!(action.state_history[0].from, ActionState::Created);
    assert_eq!(action.state_history[0].to, ActionState::Approved);
    assert_eq!(action.state_history[1].from, ActionState::Approved);
    assert_eq!(action.state_history[1].to, ActionState::Leased);
}
