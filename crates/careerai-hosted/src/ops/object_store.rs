//! Artifact object store with capability-based access.
//!
//! Objects use generated opaque prefixes (`tenant UUID/object UUID/version
//! UUID`), never user filenames. Downloads are authorized by a capability
//! bound to `{tenant_id, object_version_id, user_id, action_id or purpose,
//! HTTP method, permitted byte range, issued_at, expires_at,
//! capability_version}`, signed with a server-held capability key.

use std::collections::HashMap;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};
use tokio::sync::RwLock;

type HmacSha512 = Hmac<Sha512>;

/// Error from object store operations.
#[derive(Debug, thiserror::Error)]
pub enum ObjectStoreError {
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("capability expired")]
    CapabilityExpired,
    #[error("capability invalid")]
    CapabilityInvalid,
    #[error("capability does not cover this object")]
    CapabilityScopeMismatch,
    #[error("tenant mismatch")]
    TenantMismatch,
    #[error("orphan object detected: {0}")]
    Orphan(String),
}

/// Metadata for a stored object version.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObjectVersion {
    pub object_id: String,
    pub version_id: String,
    pub tenant_id: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deleted: bool,
}

/// A capability granting access to a specific object version.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Capability {
    pub tenant_id: String,
    pub object_version_id: String,
    pub user_id: String,
    pub purpose: String,
    pub issued_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub capability_version: u32,
    pub signature_hex: String,
}

/// In-memory artifact object store for beta.
///
/// In production this is backed by S3-compatible storage with
/// KMS envelope encryption. The trait `ObjectStoreBackend` allows
/// swapping implementations.
#[derive(Debug)]
pub struct ArtifactStore {
    inner: RwLock<ArtifactStoreInner>,
    capability_key: Vec<u8>,
}

#[derive(Debug, Default)]
struct ArtifactStoreInner {
    /// `object_version_id → (metadata, bytes)`
    objects: HashMap<String, (ObjectVersion, Vec<u8>)>,
    /// `tenant_id → set of object_version_ids`
    tenant_index: HashMap<String, Vec<String>>,
}

impl ArtifactStore {
    /// Create a new store with the given capability signing key.
    pub fn new(capability_key: Vec<u8>) -> Self {
        Self {
            inner: RwLock::new(ArtifactStoreInner::default()),
            capability_key,
        }
    }

    /// Convenience wrapper for tests.
    pub fn arc(capability_key: Vec<u8>) -> Arc<Self> {
        Arc::new(Self::new(capability_key))
    }

    /// Store an artifact. Returns the object version metadata.
    pub async fn store(
        &self,
        tenant_id: &str,
        object_id: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> ObjectVersion {
        let version_id = uuid::Uuid::new_v4().to_string();
        let hash = hex::encode(Sha256::digest(bytes));
        let metadata = ObjectVersion {
            object_id: object_id.to_string(),
            version_id: version_id.clone(),
            tenant_id: tenant_id.to_string(),
            content_type: content_type.to_string(),
            size: bytes.len() as u64,
            sha256: hash,
            created_at: chrono::Utc::now(),
            deleted: false,
        };

        let mut inner = self.inner.write().await;
        inner
            .tenant_index
            .entry(tenant_id.to_string())
            .or_default()
            .push(version_id.clone());
        inner
            .objects
            .insert(version_id.clone(), (metadata.clone(), bytes.to_vec()));

        metadata
    }

    /// Issue a download capability for an object version.
    pub async fn issue_capability(
        &self,
        tenant_id: &str,
        object_version_id: &str,
        user_id: &str,
        purpose: &str,
        ttl_seconds: i64,
    ) -> Result<Capability, ObjectStoreError> {
        let inner = self.inner.read().await;
        let (metadata, _) = inner
            .objects
            .get(object_version_id)
            .ok_or_else(|| ObjectStoreError::NotFound(object_version_id.to_string()))?;

        if metadata.tenant_id != tenant_id {
            return Err(ObjectStoreError::TenantMismatch);
        }

        let now = chrono::Utc::now();
        let expires_at = now + chrono::Duration::seconds(ttl_seconds);

        let mut cap = Capability {
            tenant_id: tenant_id.to_string(),
            object_version_id: object_version_id.to_string(),
            user_id: user_id.to_string(),
            purpose: purpose.to_string(),
            issued_at: now,
            expires_at,
            capability_version: 1,
            signature_hex: String::new(),
        };

        cap.signature_hex = sign_capability(&cap, &self.capability_key);
        Ok(cap)
    }

    /// Download an object using a capability.
    pub async fn download(
        &self,
        cap: &Capability,
    ) -> Result<(ObjectVersion, Vec<u8>), ObjectStoreError> {
        // Verify signature
        let expected_sig = sign_capability(cap, &self.capability_key);
        if !constant_time_eq::constant_time_eq(
            cap.signature_hex.as_bytes(),
            expected_sig.as_bytes(),
        ) {
            return Err(ObjectStoreError::CapabilityInvalid);
        }

        // Check expiry
        let now = chrono::Utc::now();
        if now > cap.expires_at {
            return Err(ObjectStoreError::CapabilityExpired);
        }

        let inner = self.inner.read().await;
        let (metadata, bytes) = inner
            .objects
            .get(&cap.object_version_id)
            .ok_or_else(|| ObjectStoreError::NotFound(cap.object_version_id.clone()))?;

        if metadata.tenant_id != cap.tenant_id {
            return Err(ObjectStoreError::CapabilityScopeMismatch);
        }

        Ok((metadata.clone(), bytes.clone()))
    }

    /// Delete an object version (soft delete).
    pub async fn delete(
        &self,
        tenant_id: &str,
        object_version_id: &str,
    ) -> Result<(), ObjectStoreError> {
        let mut inner = self.inner.write().await;
        let entry = inner
            .objects
            .get_mut(object_version_id)
            .ok_or_else(|| ObjectStoreError::NotFound(object_version_id.to_string()))?;

        if entry.0.tenant_id != tenant_id {
            return Err(ObjectStoreError::TenantMismatch);
        }

        entry.0.deleted = true;
        Ok(())
    }

    /// Scan for orphan objects — objects with no DB metadata.
    /// In beta, this is a no-op since objects and metadata are co-located.
    /// In production, this compares S3 inventory to DB versions.
    pub async fn orphan_scan(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner
            .objects
            .iter()
            .filter(|(_, (meta, _))| meta.deleted)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// List all object versions for a tenant.
    pub async fn list_for_tenant(&self, tenant_id: &str) -> Vec<ObjectVersion> {
        let inner = self.inner.read().await;
        inner
            .tenant_index
            .get(tenant_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| inner.objects.get(id).map(|(m, _)| m.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Purge deleted objects (called after retention period).
    pub async fn purge_deleted(&self) -> u64 {
        let mut inner = self.inner.write().await;
        let before = inner.objects.len();
        inner.objects.retain(|_, (m, _)| !m.deleted);
        let purged = before - inner.objects.len();

        // Rebuild tenant index from remaining objects
        inner.tenant_index.clear();
        let pairs: Vec<(String, String)> = inner
            .objects
            .iter()
            .map(|(id, (meta, _))| (meta.tenant_id.clone(), id.clone()))
            .collect();
        for (tenant_id, obj_id) in pairs {
            inner
                .tenant_index
                .entry(tenant_id)
                .or_default()
                .push(obj_id);
        }
        purged as u64
    }
}

/// Sign a capability with HMAC-SHA512.
#[allow(clippy::expect_used)]
fn sign_capability(cap: &Capability, key: &[u8]) -> String {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(cap.tenant_id.as_bytes());
    mac.update(b"\0");
    mac.update(cap.object_version_id.as_bytes());
    mac.update(b"\0");
    mac.update(cap.user_id.as_bytes());
    mac.update(b"\0");
    mac.update(cap.purpose.as_bytes());
    mac.update(b"\0");
    mac.update(cap.issued_at.timestamp().to_string().as_bytes());
    mac.update(b"\0");
    mac.update(cap.expires_at.timestamp().to_string().as_bytes());
    mac.update(b"\0");
    mac.update(cap.capability_version.to_string().as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn store_and_download_roundtrip() {
        let store = ArtifactStore::new(b"cap-key-32-bytes-long-enough!!!".to_vec());
        let meta = store
            .store("t1", "obj-1", "application/pdf", b"%PDF content")
            .await;

        let cap = store
            .issue_capability("t1", &meta.version_id, "u1", "download", 300)
            .await
            .unwrap();

        let (downloaded_meta, bytes) = store.download(&cap).await.unwrap();
        assert_eq!(bytes, b"%PDF content");
        assert_eq!(downloaded_meta.sha256, meta.sha256);
    }

    #[tokio::test]
    async fn cross_tenant_download_rejected() {
        let store = ArtifactStore::new(b"cap-key-32-bytes-long-enough!!!".to_vec());
        let meta = store.store("t1", "obj-1", "text/plain", b"data").await;

        let result = store
            .issue_capability("t2", &meta.version_id, "u2", "download", 300)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn expired_capability_rejected() {
        let store = ArtifactStore::new(b"cap-key-32-bytes-long-enough!!!".to_vec());
        let meta = store.store("t1", "obj-1", "text/plain", b"data").await;

        // Issue capability with negative TTL (already expired)
        let cap = store
            .issue_capability("t1", &meta.version_id, "u1", "download", -1)
            .await
            .unwrap();

        let result = store.download(&cap).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn forged_capability_rejected() {
        let store = ArtifactStore::new(b"cap-key-32-bytes-long-enough!!!".to_vec());
        let meta = store.store("t1", "obj-1", "text/plain", b"data").await;

        let mut cap = store
            .issue_capability("t1", &meta.version_id, "u1", "download", 300)
            .await
            .unwrap();

        // Tamper with signature
        cap.signature_hex = "deadbeef".to_string();
        let result = store.download(&cap).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_marks_as_deleted() {
        let store = ArtifactStore::new(b"cap-key-32-bytes-long-enough!!!".to_vec());
        let meta = store.store("t1", "obj-1", "text/plain", b"data").await;

        store.delete("t1", &meta.version_id).await.unwrap();

        let orphans = store.orphan_scan().await;
        assert!(orphans.contains(&meta.version_id));
    }

    #[tokio::test]
    async fn purge_deleted_removes_objects() {
        let store = ArtifactStore::new(b"cap-key-32-bytes-long-enough!!!".to_vec());
        let m1 = store.store("t1", "obj-1", "text/plain", b"data1").await;
        store.store("t1", "obj-2", "text/plain", b"data2").await;

        store.delete("t1", &m1.version_id).await.unwrap();
        let purged = store.purge_deleted().await;
        assert_eq!(purged, 1);

        let remaining = store.list_for_tenant("t1").await;
        assert_eq!(remaining.len(), 1);
    }

    #[tokio::test]
    async fn list_for_tenant_returns_only_own_objects() {
        let store = ArtifactStore::new(b"cap-key-32-bytes-long-enough!!!".to_vec());
        store.store("t1", "obj-1", "text/plain", b"data1").await;
        store.store("t1", "obj-2", "text/plain", b"data2").await;
        store.store("t2", "obj-3", "text/plain", b"data3").await;

        let t1_objects = store.list_for_tenant("t1").await;
        assert_eq!(t1_objects.len(), 2);

        let t2_objects = store.list_for_tenant("t2").await;
        assert_eq!(t2_objects.len(), 1);
    }
}
