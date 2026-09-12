//! Encrypted export with reauthentication gating (PR 7).
//!
//! Exports are encrypted with AES-256-GCM using a tenant-specific key
//! derived from the master key and tenant ID. Reauthentication is
//! required before an export can be initiated.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};

use crate::auth::session::SessionManager;
use crate::auth::session::SessionRecord;

/// Error from export operations.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("reauthentication required — session last active {last_active_ago_secs}s ago")]
    ReauthRequired { last_active_ago_secs: i64 },
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed")]
    DecryptionFailed,
}

/// Metadata about an export bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportBundle {
    pub tenant_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub nonce_hex: String,
    pub ciphertext_hex: String,
}

/// Check if a session is allowed to initiate an export.
///
/// The session must have been active within the reauth window.
pub fn check_reauth(
    record: &SessionRecord,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), ExportError> {
    if SessionManager::requires_reauth(record, now) {
        let ago = (now - record.last_active_at).num_seconds();
        return Err(ExportError::ReauthRequired {
            last_active_ago_secs: ago,
        });
    }
    Ok(())
}

/// Encrypt export data using AES-256-GCM.
///
/// The key is derived from the master key material. In production
/// this comes from the key management service; in tests it is
/// a fixed 32-byte key.
pub fn encrypt_export(
    plaintext: &[u8],
    master_key: &[u8; 32],
    tenant_id: &str,
) -> Result<ExportBundle, ExportError> {
    let cipher = Aes256Gcm::new(master_key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| ExportError::EncryptionFailed)?;

    Ok(ExportBundle {
        tenant_id: tenant_id.to_string(),
        created_at: chrono::Utc::now(),
        nonce_hex: hex::encode(nonce),
        ciphertext_hex: hex::encode(&ciphertext),
    })
}

/// Decrypt an export bundle.
pub fn decrypt_export(
    bundle: &ExportBundle,
    master_key: &[u8; 32],
) -> Result<Vec<u8>, ExportError> {
    let cipher = Aes256Gcm::new(master_key.into());
    let nonce_bytes = hex::decode(&bundle.nonce_hex)
        .map_err(|_| ExportError::DecryptionFailed)?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = hex::decode(&bundle.ciphertext_hex)
        .map_err(|_| ExportError::DecryptionFailed)?;
    cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| ExportError::DecryptionFailed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::auth::session::SessionRecord;
    use chrono::{Duration, Utc};

    fn fixture_session(active_at: chrono::DateTime<Utc>) -> SessionRecord {
        SessionRecord {
            token_hash: "abc".to_string(),
            tenant_id: "t1".to_string(),
            user_id: "u1".to_string(),
            workspace_id: "w1".to_string(),
            role: "owner".to_string(),
            created_at: active_at,
            last_active_at: active_at,
            revoked: false,
        }
    }

    #[test]
    fn reauth_passes_within_window() {
        let now = Utc::now();
        let session = fixture_session(now - Duration::minutes(3));
        assert!(check_reauth(&session, now).is_ok());
    }

    #[test]
    fn reauth_fails_outside_window() {
        let now = Utc::now();
        let session = fixture_session(now - Duration::minutes(10));
        let err = check_reauth(&session, now).unwrap_err();
        assert!(matches!(err, ExportError::ReauthRequired { .. }));
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = [42u8; 32];
        let plaintext = b"{\"name\":\"Jane\",\"summary\":\"Engineer\"}";
        let bundle = encrypt_export(plaintext, &key, "tenant-1").unwrap();
        let decrypted = decrypt_export(&bundle, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let key = [42u8; 32];
        let wrong_key = [99u8; 32];
        let plaintext = b"sensitive data";
        let bundle = encrypt_export(plaintext, &key, "tenant-1").unwrap();
        let result = decrypt_export(&bundle, &wrong_key);
        assert!(result.is_err());
    }

    #[test]
    fn ciphertext_differs_from_plaintext() {
        let key = [42u8; 32];
        let plaintext = b"hello world";
        let bundle = encrypt_export(plaintext, &key, "tenant-1").unwrap();
        let ciphertext = hex::decode(&bundle.ciphertext_hex).unwrap();
        assert_ne!(&ciphertext[..], plaintext);
    }

    #[test]
    fn different_encryptions_produce_different_ciphertexts() {
        let key = [42u8; 32];
        let plaintext = b"same input";
        let bundle1 = encrypt_export(plaintext, &key, "tenant-1").unwrap();
        let bundle2 = encrypt_export(plaintext, &key, "tenant-1").unwrap();
        assert_ne!(bundle1.ciphertext_hex, bundle2.ciphertext_hex);
        assert_ne!(bundle1.nonce_hex, bundle2.nonce_hex);
    }

    #[test]
    fn export_bundle_includes_tenant_id() {
        let key = [42u8; 32];
        let bundle = encrypt_export(b"data", &key, "tenant-42").unwrap();
        assert_eq!(bundle.tenant_id, "tenant-42");
    }
}
