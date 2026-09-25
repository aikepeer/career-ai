//! Workspace creation, switching, and membership.
//!
//! A personal workspace is created at first verified login. One account may
//! later join multiple workspaces and must explicitly switch context. Beta
//! roles are owner and support_readonly.

use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace not found")]
    NotFound,
    #[error("user is not a member of this workspace")]
    NotMember,
    #[error("cannot switch to a workspace without membership")]
    SwitchDenied,
    #[error("workspace already exists for this account")]
    AlreadyExists,
}

/// A workspace record.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub tenant_id: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// Workspace membership linking a user to a workspace with a role.
#[derive(Debug, Clone)]
pub struct WorkspaceMembership {
    pub id: String,
    pub workspace_id: String,
    pub user_id: String,
    pub role: super::roles::Role,
    pub joined_at: DateTime<Utc>,
}

impl Workspace {
    /// Create a new personal workspace at first verified login.
    pub fn new(tenant_id: &str, display_name: &str, user_id: &str) -> Self {
        let id = generate_id();
        Self {
            id,
            tenant_id: tenant_id.to_string(),
            display_name: display_name.to_string(),
            created_at: Utc::now(),
            created_by: user_id.to_string(),
        }
    }
}

impl WorkspaceMembership {
    /// Create the initial owner membership for a new workspace.
    pub fn owner(workspace_id: &str, user_id: &str) -> Self {
        Self {
            id: generate_id(),
            workspace_id: workspace_id.to_string(),
            user_id: user_id.to_string(),
            role: super::roles::Role::Owner,
            joined_at: Utc::now(),
        }
    }

    /// Check if this membership allows switching to the workspace.
    pub fn can_switch(&self) -> bool {
        true
    }
}

/// Validate a workspace switch: user must be a member of the target workspace.
pub fn validate_switch<'a>(
    memberships: &'a [WorkspaceMembership],
    target_workspace_id: &'a str,
) -> Result<&'a WorkspaceMembership, WorkspaceError> {
    memberships
        .iter()
        .find(|m| m.workspace_id == target_workspace_id)
        .ok_or(WorkspaceError::SwitchDenied)
}

fn generate_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::roles::Role;
    use super::*;

    #[test]
    fn new_workspace_has_unique_id() {
        let w1 = Workspace::new("t1", "Alice", "u1");
        let w2 = Workspace::new("t1", "Bob", "u2");
        assert_ne!(w1.id, w2.id);
    }

    #[test]
    fn owner_membership_has_owner_role() {
        let m = WorkspaceMembership::owner("w1", "u1");
        assert_eq!(m.role, Role::Owner);
    }

    #[test]
    fn validate_switch_allows_member() {
        let m = WorkspaceMembership::owner("w1", "u1");
        let memberships = vec![m];
        let result = validate_switch(&memberships, "w1");
        assert!(result.is_ok());
    }

    #[test]
    fn validate_switch_denies_non_member() {
        let m = WorkspaceMembership::owner("w1", "u1");
        let memberships = vec![m];
        let err = validate_switch(&memberships, "w2").unwrap_err();
        assert!(matches!(err, WorkspaceError::SwitchDenied));
    }

    #[test]
    fn membership_can_switch() {
        let m = WorkspaceMembership::owner("w1", "u1");
        assert!(m.can_switch());
    }
}
