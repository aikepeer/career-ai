//! Shared application state for the hosted API.
//!
//! All stores are in-memory `RwLock<HashMap<…>>` for the beta.
//! In production these are backed by PostgreSQL with RLS.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::action::ExternalAction;
use crate::auth::magic_link::MagicLinkToken;
use crate::auth::rate_limit::RateLimiter;
use crate::auth::recovery::RecoveryCodeSet;
use crate::auth::session::SessionRecord;
use crate::auth::totp::Totp;
use crate::auth::workspace::{Workspace, WorkspaceMembership};
use crate::entitlement::admission::EntitlementDecision;
use crate::entitlement::billing::WebhookDedup;
use crate::ops::metrics::MetricsCollector;
use crate::ops::object_store::ArtifactStore;

/// Shared state accessible to all Axum handlers.
///
/// Each store is an in-memory map guarded by `RwLock`.
/// The `master_key` is used for AES-256-GCM export encryption.
pub struct AppState {
    /// `token_hash → SessionRecord`
    pub sessions: RwLock<HashMap<String, SessionRecord>>,
    /// `token_hash → MagicLinkToken` (for verification)
    pub magic_links: RwLock<HashMap<String, MagicLinkToken>>,
    /// `login_tx_id → raw_token` (dev-mode: lets tests retrieve the token)
    pub dev_tokens: RwLock<HashMap<String, String>>,
    /// `email → Totp` (enrolled TOTP secrets)
    pub totp_secrets: RwLock<HashMap<String, Totp>>,
    /// `email → RecoveryCodeSet`
    pub recovery_codes: RwLock<HashMap<String, RecoveryCodeSet>>,
    /// `tenant_id → EntitlementDecision`
    pub entitlements: RwLock<HashMap<String, EntitlementDecision>>,
    /// `action_id → ExternalAction`
    pub actions: RwLock<HashMap<String, ExternalAction>>,
    /// `workspace_id → Workspace`
    pub workspaces: RwLock<HashMap<String, Workspace>>,
    /// `user_id → Vec<WorkspaceMembership>`
    pub memberships: RwLock<HashMap<String, Vec<WorkspaceMembership>>>,
    /// Per-IP rate limiter for login/recovery/import.
    pub rate_limiter: RwLock<RateLimiter>,
    /// Webhook event dedup tracker.
    pub webhook_dedup: RwLock<WebhookDedup>,
    /// `provider → webhook_secret_bytes`
    pub webhook_secrets: RwLock<HashMap<String, Vec<u8>>>,
    /// Master key for export encryption (32 bytes).
    pub master_key: [u8; 32],
    /// Beta exit metrics collector.
    pub metrics: Arc<MetricsCollector>,
    /// Artifact object store with capability-based access.
    pub artifacts: Arc<ArtifactStore>,
}

impl AppState {
    /// Create a new empty state with the given master key.
    pub fn new(master_key: [u8; 32]) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            magic_links: RwLock::new(HashMap::new()),
            dev_tokens: RwLock::new(HashMap::new()),
            totp_secrets: RwLock::new(HashMap::new()),
            recovery_codes: RwLock::new(HashMap::new()),
            entitlements: RwLock::new(HashMap::new()),
            actions: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            memberships: RwLock::new(HashMap::new()),
            rate_limiter: RwLock::new(RateLimiter::new()),
            webhook_dedup: RwLock::new(WebhookDedup::new()),
            webhook_secrets: RwLock::new(HashMap::new()),
            master_key,
            metrics: Arc::new(MetricsCollector::new()),
            artifacts: ArtifactStore::arc(b"artifact-cap-key-32-bytes-ok!".to_vec()),
        }
    }

    /// Convenience wrapper for tests.
    pub fn arc(master_key: [u8; 32]) -> Arc<Self> {
        Arc::new(Self::new(master_key))
    }

    /// Register a webhook secret for a provider.
    pub async fn set_webhook_secret(&self, provider: &str, secret: Vec<u8>) {
        self.webhook_secrets
            .write()
            .await
            .insert(provider.to_string(), secret);
    }

    /// Seed an active entitlement for a tenant.
    pub async fn seed_entitlement(&self, tenant_id: &str, limit: u64) {
        use crate::entitlement::admission::EntitlementStatus;
        use chrono::Utc;
        self.entitlements.write().await.insert(
            tenant_id.to_string(),
            EntitlementDecision {
                plan_id: "beta".to_string(),
                plan_version: 1,
                feature: "metered".to_string(),
                limit,
                used: 0,
                reserved: 0,
                period_start: Utc::now(),
                period_end: Utc::now() + chrono::Duration::days(30),
                status: EntitlementStatus::Active,
            },
        );
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState").finish()
    }
}
