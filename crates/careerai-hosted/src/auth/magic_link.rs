//! Magic link token generation, hashing, and verification.
//!
//! Each magic link is a single-use, 10-minute hashed token bound to a login
//! transaction and device risk context. A changed context requires restarting
//! login. Links are never accepted twice; concurrent transactions are
//! invalidated on completion.

use chrono::{DateTime, Duration, Utc};
use constant_time_eq::constant_time_eq;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

/// Magic link token validity window (10 minutes per design doc).
const TOKEN_TTL: Duration = Duration::minutes(10);

/// Token length in raw bytes (32 bytes = 256 bits entropy).
const TOKEN_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum MagicLinkError {
    #[error("magic link token has expired")]
    Expired,
    #[error("magic link token has already been used")]
    AlreadyUsed,
    #[error("magic link token hash does not match")]
    InvalidHash,
    #[error("device context mismatch — restart login")]
    ContextMismatch,
    #[error("concurrent login transaction invalidated")]
    ConcurrentInvalidated,
}

/// Device risk context bound to a magic link.
/// A changed context requires restarting login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceContext {
    /// Normalized User-Agent hash (SHA-256 hex).
    pub user_agent_hash: String,
    /// Client IP address.
    pub client_ip: String,
}

impl DeviceContext {
    pub fn from_request(user_agent: &str, client_ip: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(user_agent.as_bytes());
        let ua_hash = hex::encode(hasher.finalize());
        Self {
            user_agent_hash: ua_hash,
            client_ip: client_ip.to_string(),
        }
    }
}

/// A magic link token record stored in the database.
#[derive(Debug, Clone)]
pub struct MagicLinkToken {
    /// Opaque token sent to the user (not stored).
    /// Only the hash is persisted.
    pub token_hash: String,
    /// Login transaction ID — concurrent transactions invalidated on completion.
    pub login_tx_id: String,
    /// Email address the link was sent to.
    pub email: String,
    /// Device context at issuance time.
    pub device_context: DeviceContext,
    /// When the token was created.
    pub created_at: DateTime<Utc>,
    /// When the token expires (created_at + 10 min).
    pub expires_at: DateTime<Utc>,
    /// Whether the token has been used.
    pub used: bool,
    /// Requested workspace ID (where practical).
    pub requested_workspace: Option<String>,
}

impl MagicLinkToken {
    /// Generate a new magic link token.
    /// Returns (raw_token, token_record) where raw_token is sent to user.
    pub fn generate(
        email: &str,
        device_context: DeviceContext,
        login_tx_id: &str,
        requested_workspace: Option<String>,
    ) -> (String, Self) {
        let mut raw_bytes = [0u8; TOKEN_BYTES];
        rand::thread_rng().fill_bytes(&mut raw_bytes);
        let raw_token = hex::encode(raw_bytes);
        let token_hash = hash_token(&raw_token);
        let now = Utc::now();
        let record = Self {
            token_hash,
            login_tx_id: login_tx_id.to_string(),
            email: email.to_string(),
            device_context,
            created_at: now,
            expires_at: now + TOKEN_TTL,
            used: false,
            requested_workspace,
        };
        (raw_token, record)
    }

    /// Verify a raw token against a stored record.
    /// Checks: hash match, expiry, single-use, and device context.
    pub fn verify(
        &self,
        raw_token: &str,
        device_context: &DeviceContext,
        now: DateTime<Utc>,
    ) -> Result<(), MagicLinkError> {
        if self.used {
            return Err(MagicLinkError::AlreadyUsed);
        }
        if now > self.expires_at {
            return Err(MagicLinkError::Expired);
        }
        let provided_hash = hash_token(raw_token);
        if !constant_time_eq(provided_hash.as_bytes(), self.token_hash.as_bytes()) {
            return Err(MagicLinkError::InvalidHash);
        }
        if self.device_context != *device_context {
            return Err(MagicLinkError::ContextMismatch);
        }
        Ok(())
    }

    /// Mark the token as used (single-use enforcement).
    pub fn mark_used(&mut self) {
        self.used = true;
    }

    /// Check if another login transaction with the same email invalidates this one.
    pub fn is_invalidated_by(&self, other_tx_id: &str) -> bool {
        self.login_tx_id != other_tx_id
    }
}

/// Hash a raw token with SHA-256 for storage.
/// Never store the raw token.
fn hash_token(raw_token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate an HMAC-SHA256 signature for login transaction IDs.
/// Used to ensure transaction IDs are tamper-resistant.
pub fn sign_login_tx(tx_id: &str, secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC accepts any key length"));
    mac.update(tx_id.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn test_ctx() -> DeviceContext {
        DeviceContext::from_request("Mozilla/5.0", "192.168.1.1")
    }

    #[test]
    fn token_generation_produces_different_tokens() {
        let ctx = test_ctx();
        let (raw1, record1) = MagicLinkToken::generate("a@b.com", ctx.clone(), "tx1", None);
        let (raw2, record2) = MagicLinkToken::generate("a@b.com", ctx.clone(), "tx2", None);
        assert_ne!(raw1, raw2);
        assert_ne!(record1.token_hash, record2.token_hash);
    }

    #[test]
    fn verify_accepts_correct_token() {
        let ctx = test_ctx();
        let (raw, record) = MagicLinkToken::generate("a@b.com", ctx.clone(), "tx1", None);
        record.verify(&raw, &ctx, Utc::now()).unwrap();
    }

    #[test]
    fn verify_rejects_wrong_token() {
        let ctx = test_ctx();
        let (raw, record) = MagicLinkToken::generate("a@b.com", ctx.clone(), "tx1", None);
        let wrong = format!("{raw}xyz");
        let err = record.verify(&wrong, &ctx, Utc::now()).unwrap_err();
        assert!(matches!(err, MagicLinkError::InvalidHash));
    }

    #[test]
    fn verify_rejects_expired_token() {
        let ctx = test_ctx();
        let (raw, record) = MagicLinkToken::generate("a@b.com", ctx.clone(), "tx1", None);
        let future = Utc::now() + Duration::minutes(11);
        let err = record.verify(&raw, &ctx, future).unwrap_err();
        assert!(matches!(err, MagicLinkError::Expired));
    }

    #[test]
    fn verify_rejects_used_token() {
        let ctx = test_ctx();
        let (raw, mut record) = MagicLinkToken::generate("a@b.com", ctx.clone(), "tx1", None);
        record.mark_used();
        let err = record.verify(&raw, &ctx, Utc::now()).unwrap_err();
        assert!(matches!(err, MagicLinkError::AlreadyUsed));
    }

    #[test]
    fn verify_rejects_context_mismatch() {
        let ctx = test_ctx();
        let (raw, record) = MagicLinkToken::generate("a@b.com", ctx, "tx1", None);
        let different_ctx = DeviceContext::from_request("Chrome/120", "10.0.0.1");
        let err = record.verify(&raw, &different_ctx, Utc::now()).unwrap_err();
        assert!(matches!(err, MagicLinkError::ContextMismatch));
    }

    #[test]
    fn concurrent_transaction_detected() {
        let ctx = test_ctx();
        let (_, record) = MagicLinkToken::generate("a@b.com", ctx, "tx1", None);
        assert!(record.is_invalidated_by("tx2"));
        assert!(!record.is_invalidated_by("tx1"));
    }

    #[test]
    fn login_tx_signature_is_deterministic() {
        let secret = b"test-secret";
        let sig1 = sign_login_tx("tx-123", secret);
        let sig2 = sign_login_tx("tx-123", secret);
        assert_eq!(sig1, sig2);
        let sig3 = sign_login_tx("tx-456", secret);
        assert_ne!(sig1, sig3);
    }

    #[test]
    fn token_hash_does_not_contain_raw_token() {
        let ctx = test_ctx();
        let (raw, record) = MagicLinkToken::generate("a@b.com", ctx, "tx1", None);
        assert!(!record.token_hash.contains(&raw));
    }

    #[test]
    fn forwarded_link_rejected_on_different_device() {
        let ctx = DeviceContext::from_request("Firefox/121", "192.168.1.50");
        let (raw, record) = MagicLinkToken::generate("victim@b.com", ctx, "tx1", None);
        // Attacker forwards link to victim's different device
        let attacker_ctx = DeviceContext::from_request("Chrome/120", "10.0.0.99");
        let err = record.verify(&raw, &attacker_ctx, Utc::now()).unwrap_err();
        assert!(matches!(err, MagicLinkError::ContextMismatch));
    }

    #[test]
    fn replayed_link_after_new_login_invalidated() {
        let ctx = test_ctx();
        let (_, record) = MagicLinkToken::generate("a@b.com", ctx.clone(), "tx1", None);
        // User requests a new login — tx1 is invalidated by tx2
        assert!(record.is_invalidated_by("tx2"));
        // Even if the raw token is correct, a new login transaction
        // supersedes the old one
        let (_, record2) = MagicLinkToken::generate("a@b.com", ctx, "tx2", None);
        assert_ne!(record.login_tx_id, record2.login_tx_id);
    }
}
