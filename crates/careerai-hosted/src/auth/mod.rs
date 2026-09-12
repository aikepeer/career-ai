//! Authentication and workspace authorization (PR 2).
//!
//! Implements email magic-link login with mandatory TOTP MFA, session
//! management, workspace creation/switching, role-based authorization,
//! reauthentication, recovery codes, and abuse rate limiting.
//!
//! Pure logic (token generation/hashing, TOTP, session timeout) is in
//! submodules and unit-tested without a database.

pub mod magic_link;
pub mod rate_limit;
pub mod recovery;
pub mod roles;
pub mod session;
pub mod totp;
pub mod workspace;

// Re-export key types
pub use magic_link::{MagicLinkError, MagicLinkToken};
pub use rate_limit::{RateLimitDecision, RateLimiter};
pub use recovery::{RecoveryCodeError, RecoveryCodeSet};
pub use roles::{AuthzDecision, Permission, Role};
pub use session::{SessionError, SessionManager};
pub use totp::{Totp, TotpError};
pub use workspace::{Workspace, WorkspaceError, WorkspaceMembership};
