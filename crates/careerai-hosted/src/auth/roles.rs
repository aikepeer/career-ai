//! Role-based authorization matrix.
//!
//! Beta roles: `owner` and `support_readonly`. The matrix defines which
//! operations each role may perform. Support cannot impersonate or create
//! approvals.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthzError {
    #[error("operation not permitted for role {role}")]
    Denied { role: String },
    #[error("reauthentication required for {operation}")]
    ReauthRequired { operation: String },
}

/// Roles in the hosted beta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    SupportReadonly,
}

/// Permissions checked against the role matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ReadWriteProfile,
    GenerateArtifacts,
    ExportDelete,
    ApproveSideEffect,
    BillingManagement,
    ViewSecurityAudit,
    SupportDiagnosis,
    SwitchWorkspace,
}

/// Authorization decision returned by the matrix check.
#[derive(Debug, Clone)]
pub struct AuthzDecision {
    pub allowed: bool,
    pub requires_reauth: bool,
}

impl Role {
    /// Check if a role may perform an operation.
    /// Returns whether allowed and whether reauth is required.
    pub fn check(self, perm: Permission) -> AuthzDecision {
        match (self, perm) {
            // Allowed without reauth
            (
                Role::Owner,
                Permission::ReadWriteProfile
                | Permission::GenerateArtifacts
                | Permission::BillingManagement
                | Permission::ViewSecurityAudit
                | Permission::SwitchWorkspace,
            )
            | (
                Role::SupportReadonly,
                Permission::ViewSecurityAudit | Permission::SupportDiagnosis,
            ) => allow(false),

            // Sensitive operations require reauth
            (Role::Owner, Permission::ExportDelete | Permission::ApproveSideEffect) => allow(true),

            // Denied
            (Role::Owner, Permission::SupportDiagnosis)
            | (
                Role::SupportReadonly,
                Permission::ReadWriteProfile
                | Permission::GenerateArtifacts
                | Permission::ExportDelete
                | Permission::ApproveSideEffect
                | Permission::BillingManagement
                | Permission::SwitchWorkspace,
            ) => deny(),
        }
    }

    /// Whether this role can impersonate (never for either beta role).
    pub fn can_impersonate(self) -> bool {
        false
    }

    /// Whether this role can create approvals (only owner).
    pub fn can_create_approvals(self) -> bool {
        matches!(self, Role::Owner)
    }

    /// Whether this role can issue capabilities or signed URLs.
    pub fn can_issue_capabilities(self) -> bool {
        matches!(self, Role::Owner)
    }
}

fn allow(reauth: bool) -> AuthzDecision {
    AuthzDecision {
        allowed: true,
        requires_reauth: reauth,
    }
}

fn deny() -> AuthzDecision {
    AuthzDecision {
        allowed: false,
        requires_reauth: false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn owner_can_read_write_profile() {
        let d = Role::Owner.check(Permission::ReadWriteProfile);
        assert!(d.allowed);
        assert!(!d.requires_reauth);
    }

    #[test]
    fn owner_export_requires_reauth() {
        let d = Role::Owner.check(Permission::ExportDelete);
        assert!(d.allowed);
        assert!(d.requires_reauth);
    }

    #[test]
    fn owner_approve_requires_reauth() {
        let d = Role::Owner.check(Permission::ApproveSideEffect);
        assert!(d.allowed);
        assert!(d.requires_reauth);
    }

    #[test]
    fn support_cannot_modify() {
        let d = Role::SupportReadonly.check(Permission::ReadWriteProfile);
        assert!(!d.allowed);
    }

    #[test]
    fn support_cannot_approve() {
        let d = Role::SupportReadonly.check(Permission::ApproveSideEffect);
        assert!(!d.allowed);
    }

    #[test]
    fn support_cannot_export() {
        let d = Role::SupportReadonly.check(Permission::ExportDelete);
        assert!(!d.allowed);
    }

    #[test]
    fn support_can_view_audit() {
        let d = Role::SupportReadonly.check(Permission::ViewSecurityAudit);
        assert!(d.allowed);
    }

    #[test]
    fn support_can_diagnose() {
        let d = Role::SupportReadonly.check(Permission::SupportDiagnosis);
        assert!(d.allowed);
    }

    #[test]
    fn owner_cannot_impersonate() {
        assert!(!Role::Owner.can_impersonate());
        assert!(!Role::SupportReadonly.can_impersonate());
    }

    #[test]
    fn only_owner_creates_approvals() {
        assert!(Role::Owner.can_create_approvals());
        assert!(!Role::SupportReadonly.can_create_approvals());
    }

    #[test]
    fn only_owner_issues_capabilities() {
        assert!(Role::Owner.can_issue_capabilities());
        assert!(!Role::SupportReadonly.can_issue_capabilities());
    }
}
