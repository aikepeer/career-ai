//! Session management: opaque hashed cookies, rotation, and timeouts.
//!
//! Sessions are opaque tokens hashed server-side with SHA-256. Cookies are
//! Secure, HttpOnly, SameSite. Sessions rotate after login and privilege
//! change. Idle timeout 30 minutes, absolute timeout 30 days.

use chrono::{DateTime, Duration, Utc};
use constant_time_eq::constant_time_eq;
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Session idle timeout (30 minutes per design doc).
const IDLE_TIMEOUT: Duration = Duration::minutes(30);
/// Session absolute timeout (30 days per design doc).
const ABSOLUTE_TIMEOUT: Duration = Duration::days(30);
/// Session token entropy in bytes.
const SESSION_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session not found")]
    NotFound,
    #[error("session token hash mismatch")]
    InvalidToken,
    #[error("session expired (idle timeout)")]
    IdleExpired,
    #[error("session expired (absolute timeout)")]
    AbsoluteExpired,
    #[error("session revoked")]
    Revoked,
}

/// A session record stored server-side.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// SHA-256 hash of the raw session token.
    pub token_hash: String,
    pub tenant_id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub revoked: bool,
}

/// Session manager handles creation, verification, rotation, and timeout.
#[derive(Debug, Clone)]
pub struct SessionManager;

impl SessionManager {
    /// Generate a new session: returns (raw_token, record).
    /// The raw token is set as a cookie; only the hash is stored.
    pub fn create(
        tenant_id: &str,
        user_id: &str,
        workspace_id: &str,
        role: &str,
    ) -> (String, SessionRecord) {
        let mut bytes = [0u8; SESSION_BYTES];
        rand::thread_rng().fill_bytes(&mut bytes);
        let raw_token = hex::encode(bytes);
        let token_hash = hash_session(&raw_token);
        let now = Utc::now();
        let record = SessionRecord {
            token_hash,
            tenant_id: tenant_id.to_string(),
            user_id: user_id.to_string(),
            workspace_id: workspace_id.to_string(),
            role: role.to_string(),
            created_at: now,
            last_active_at: now,
            revoked: false,
        };
        (raw_token, record)
    }

    /// Verify a raw session token against a stored record.
    /// Checks: hash match, not revoked, idle timeout, absolute timeout.
    pub fn verify(
        record: &SessionRecord,
        raw_token: &str,
        now: DateTime<Utc>,
    ) -> Result<(), SessionError> {
        if record.revoked {
            return Err(SessionError::Revoked);
        }
        let provided_hash = hash_session(raw_token);
        if !constant_time_eq(provided_hash.as_bytes(), record.token_hash.as_bytes()) {
            return Err(SessionError::InvalidToken);
        }
        if now > record.last_active_at + IDLE_TIMEOUT {
            return Err(SessionError::IdleExpired);
        }
        if now > record.created_at + ABSOLUTE_TIMEOUT {
            return Err(SessionError::AbsoluteExpired);
        }
        Ok(())
    }

    /// Rotate a session: generates a new token hash.
    /// Used after login and privilege changes.
    /// Returns (new_raw_token, new_record) preserving the original created_at.
    pub fn rotate(record: &SessionRecord) -> (String, SessionRecord) {
        let mut bytes = [0u8; SESSION_BYTES];
        rand::thread_rng().fill_bytes(&mut bytes);
        let raw_token = hex::encode(bytes);
        let token_hash = hash_session(&raw_token);
        let now = Utc::now();
        let new_record = SessionRecord {
            token_hash,
            tenant_id: record.tenant_id.clone(),
            user_id: record.user_id.clone(),
            workspace_id: record.workspace_id.clone(),
            role: record.role.clone(),
            created_at: record.created_at,
            last_active_at: now,
            revoked: false,
        };
        (raw_token, new_record)
    }

    /// Check if a session needs reauthentication for sensitive operations.
    /// Reauth required for: export, delete, approve side effects.
    /// Requires recent activity (within 5 minutes).
    pub fn requires_reauth(record: &SessionRecord, now: DateTime<Utc>) -> bool {
        now > record.last_active_at + Duration::minutes(5)
    }

    /// Cookie attributes for a session token.
    pub fn cookie_attributes() -> &'static str {
        "HttpOnly; Secure; SameSite=Lax; Path=/"
    }

    /// Update last active timestamp.
    pub fn touch(record: &mut SessionRecord, now: DateTime<Utc>) {
        record.last_active_at = now;
    }

    /// Hash a raw session token for storage/lookup.
    /// The raw token is never stored; only its SHA-256 hash.
    pub fn hash_token(raw_token: &str) -> String {
        hash_session(raw_token)
    }
}

fn hash_session(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_session() -> (String, SessionRecord) {
        SessionManager::create("t1", "u1", "w1", "owner")
    }

    #[test]
    fn session_create_and_verify() {
        let (raw, record) = make_session();
        SessionManager::verify(&record, &raw, Utc::now()).unwrap();
    }

    #[test]
    fn session_rejects_wrong_token() {
        let (_, record) = make_session();
        let err = SessionManager::verify(&record, "wrong", Utc::now()).unwrap_err();
        assert!(matches!(err, SessionError::InvalidToken));
    }

    #[test]
    fn session_rejects_revoked() {
        let (raw, mut record) = make_session();
        record.revoked = true;
        let err = SessionManager::verify(&record, &raw, Utc::now()).unwrap_err();
        assert!(matches!(err, SessionError::Revoked));
    }

    #[test]
    fn session_idle_timeout() {
        let (raw, record) = make_session();
        let future = Utc::now() + Duration::minutes(31);
        let err = SessionManager::verify(&record, &raw, future).unwrap_err();
        assert!(matches!(err, SessionError::IdleExpired));
    }

    #[test]
    fn session_absolute_timeout() {
        let (raw, mut record) = make_session();
        // Simulate last_active within idle window but created long ago
        record.created_at = Utc::now() - Duration::days(31);
        let now = Utc::now();
        record.last_active_at = now - Duration::minutes(1);
        let err = SessionManager::verify(&record, &raw, now).unwrap_err();
        assert!(matches!(err, SessionError::AbsoluteExpired));
    }

    #[test]
    fn session_rotation_preserves_created_at() {
        let (_, record) = make_session();
        let (new_raw, new_record) = SessionManager::rotate(&record);
        assert_ne!(new_raw, "");
        assert_eq!(new_record.created_at, record.created_at);
        assert_ne!(new_record.token_hash, record.token_hash);
        // New token should verify
        SessionManager::verify(&new_record, &new_raw, Utc::now()).unwrap();
    }

    #[test]
    fn session_touch_updates_last_active() {
        let (_, mut record) = make_session();
        let original = record.last_active_at;
        std::thread::sleep(std::time::Duration::from_millis(10));
        SessionManager::touch(&mut record, Utc::now());
        assert!(record.last_active_at > original);
    }

    #[test]
    fn reauth_required_after_5_minutes() {
        let (_, record) = make_session();
        let now = Utc::now() + Duration::minutes(6);
        assert!(SessionManager::requires_reauth(&record, now));
    }

    #[test]
    fn reauth_not_required_recently() {
        let (_, record) = make_session();
        let now = Utc::now() + Duration::minutes(3);
        assert!(!SessionManager::requires_reauth(&record, now));
    }

    #[test]
    fn cookie_attributes_are_secure() {
        let attrs = SessionManager::cookie_attributes();
        assert!(attrs.contains("HttpOnly"));
        assert!(attrs.contains("Secure"));
        assert!(attrs.contains("SameSite"));
    }

    #[test]
    fn session_hash_not_in_raw() {
        let (raw, record) = make_session();
        assert!(!record.token_hash.contains(&raw));
    }
}
