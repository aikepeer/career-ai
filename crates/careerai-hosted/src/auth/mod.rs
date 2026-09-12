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
pub use magic_link::{MagicLinkToken, MagicLinkError};
pub use rate_limit::{RateLimiter, RateLimitDecision};
pub use recovery::{RecoveryCodeSet, RecoveryCodeError};
pub use roles::{Role, Permission, AuthzDecision};
pub use session::{SessionManager, SessionError};
pub use totp::{Totp, TotpError};
pub use workspace::{Workspace, WorkspaceMembership, WorkspaceError};
