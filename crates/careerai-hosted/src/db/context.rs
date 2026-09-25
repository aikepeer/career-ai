//! Tenant context propagation for RLS.
//!
//! Every transaction begins by authenticating an internal signed context
//! and executing `SET LOCAL app.tenant_id`, `app.actor_id`, `app.job_id`,
//! and `app.access_reason`. Absent or malformed context causes a hard
//! failure. Pool checkout resets/discards the connection.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("missing tenant context — hard failure")]
    Missing,
    #[error("invalid context signature")]
    InvalidSignature,
    #[error("malformed context")]
    Malformed,
}

/// Tenant context set per-transaction via `SET LOCAL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantContext {
    pub tenant_id: String,
    pub actor_id: String,
    pub job_id: Option<String>,
    pub access_reason: AccessReason,
}

/// Why a transaction is accessing tenant data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessReason {
    ApiRequest,
    WorkerJob,
    Webhook,
    Export,
    Report,
    Support,
}

impl TenantContext {
    /// Create a context for an API request.
    pub fn api(tenant_id: &str, actor_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            actor_id: actor_id.to_string(),
            job_id: None,
            access_reason: AccessReason::ApiRequest,
        }
    }

    /// Create a context for a worker job.
    pub fn worker(tenant_id: &str, actor_id: &str, job_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            actor_id: actor_id.to_string(),
            job_id: Some(job_id.to_string()),
            access_reason: AccessReason::WorkerJob,
        }
    }

    /// Create a context for a webhook handler.
    pub fn webhook(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            actor_id: "webhook".to_string(),
            job_id: None,
            access_reason: AccessReason::Webhook,
        }
    }

    /// Generate SQL SET LOCAL statements for this context.
    /// These are executed at the start of every transaction.
    pub fn to_sql(&self) -> Vec<String> {
        let mut stmts = vec![
            format!(
                "SET LOCAL app.tenant_id = '{}'",
                escape_sql(&self.tenant_id)
            ),
            format!("SET LOCAL app.actor_id = '{}'", escape_sql(&self.actor_id)),
            format!(
                "SET LOCAL app.access_reason = '{}'",
                match self.access_reason {
                    AccessReason::ApiRequest => "api_request",
                    AccessReason::WorkerJob => "worker_job",
                    AccessReason::Webhook => "webhook",
                    AccessReason::Export => "export",
                    AccessReason::Report => "report",
                    AccessReason::Support => "support",
                }
            ),
        ];
        if let Some(ref job_id) = self.job_id {
            stmts.push(format!("SET LOCAL app.job_id = '{}'", escape_sql(job_id)));
        }
        stmts
    }

    /// Sign the context with HMAC-SHA256 for tamper resistance.
    /// Workers receive this signed context from a signed queue envelope.
    pub fn sign(&self, secret: &[u8]) -> SignedContext {
        let payload = self.canonical_payload();
        let mut mac = HmacSha256::new_from_slice(secret)
            .unwrap_or_else(|_| unreachable!("HMAC accepts any key length"));
        mac.update(payload.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());
        SignedContext {
            context: self.clone(),
            signature,
            signed_at: Utc::now(),
        }
    }
}

impl TenantContext {
    fn canonical_payload(&self) -> String {
        format!(
            "{}|{}|{}|{:?}",
            self.tenant_id,
            self.actor_id,
            self.job_id.as_deref().unwrap_or(""),
            self.access_reason,
        )
    }
}

/// A signed tenant context for passing to workers via queue envelopes.
#[derive(Debug, Clone)]
pub struct SignedContext {
    pub context: TenantContext,
    pub signature: String,
    pub signed_at: DateTime<Utc>,
}

impl SignedContext {
    /// Verify the signature.
    pub fn verify(&self, secret: &[u8]) -> Result<(), ContextError> {
        let payload = self.context.canonical_payload();
        let mut mac = HmacSha256::new_from_slice(secret)
            .unwrap_or_else(|_| unreachable!("HMAC accepts any key length"));
        mac.update(payload.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        if expected != self.signature {
            return Err(ContextError::InvalidSignature);
        }
        Ok(())
    }
}

/// Parse a tenant context from SET LOCAL values.
/// Used to verify context was properly set after pool checkout.
pub fn parse_context(
    tenant_id: Option<&str>,
    actor_id: Option<&str>,
    access_reason: Option<&str>,
) -> Result<TenantContext, ContextError> {
    let tenant_id = tenant_id.ok_or(ContextError::Missing)?;
    if tenant_id.is_empty() {
        return Err(ContextError::Missing);
    }
    let actor_id = actor_id.ok_or(ContextError::Missing)?;
    let access_reason = match access_reason {
        Some("api_request") => AccessReason::ApiRequest,
        Some("worker_job") => AccessReason::WorkerJob,
        Some("webhook") => AccessReason::Webhook,
        Some("export") => AccessReason::Export,
        Some("report") => AccessReason::Report,
        Some("support") => AccessReason::Support,
        _ => return Err(ContextError::Malformed),
    };
    Ok(TenantContext {
        tenant_id: tenant_id.to_string(),
        actor_id: actor_id.to_string(),
        job_id: None,
        access_reason,
    })
}

fn escape_sql(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn api_context_generates_correct_sql() {
        let ctx = TenantContext::api("t1", "u1");
        let sql = ctx.to_sql();
        assert!(sql[0].contains("app.tenant_id = 't1'"));
        assert!(sql[1].contains("app.actor_id = 'u1'"));
        assert!(sql[2].contains("app.access_reason = 'api_request'"));
        assert_eq!(sql.len(), 3); // no job_id for API
    }

    #[test]
    fn worker_context_includes_job_id() {
        let ctx = TenantContext::worker("t1", "u1", "j1");
        let sql = ctx.to_sql();
        assert_eq!(sql.len(), 4);
        assert!(sql[3].contains("app.job_id = 'j1'"));
    }

    #[test]
    fn sql_injection_in_tenant_id_escaped() {
        let ctx = TenantContext::api("t1'; DROP TABLE users;--", "u1");
        let sql = ctx.to_sql();
        // Single quotes in the input are doubled to stay inside the string literal
        assert!(sql[0].contains("''"));
        // The statement is a single SET LOCAL — no bare semicolons outside quotes
        let inner = sql[0]
            .strip_prefix("SET LOCAL app.tenant_id = '")
            .unwrap_or("");
        assert!(inner.ends_with('\''));
        assert!(!inner.ends_with("';"));
    }

    #[test]
    fn signed_context_roundtrip() {
        let secret = b"test-secret";
        let ctx = TenantContext::worker("t1", "u1", "j1");
        let signed = ctx.sign(secret);
        signed.verify(secret).unwrap();
    }

    #[test]
    fn signed_context_rejects_tampered() {
        let secret = b"test-secret";
        let ctx = TenantContext::worker("t1", "u1", "j1");
        let mut signed = ctx.sign(secret);
        signed.context.tenant_id = "t2".to_string();
        assert!(signed.verify(secret).is_err());
    }

    #[test]
    fn parse_context_success() {
        let ctx = parse_context(Some("t1"), Some("u1"), Some("api_request")).unwrap();
        assert_eq!(ctx.tenant_id, "t1");
        assert_eq!(ctx.access_reason, AccessReason::ApiRequest);
    }

    #[test]
    fn parse_context_missing_tenant_fails() {
        let err = parse_context(None, Some("u1"), Some("api_request")).unwrap_err();
        assert!(matches!(err, ContextError::Missing));
    }

    #[test]
    fn parse_context_empty_tenant_fails() {
        let err = parse_context(Some(""), Some("u1"), Some("api_request")).unwrap_err();
        assert!(matches!(err, ContextError::Missing));
    }

    #[test]
    fn parse_context_invalid_reason_fails() {
        let err = parse_context(Some("t1"), Some("u1"), Some("invalid")).unwrap_err();
        assert!(matches!(err, ContextError::Malformed));
    }

    #[test]
    fn webhook_context_has_webhook_actor() {
        let ctx = TenantContext::webhook("t1");
        assert_eq!(ctx.actor_id, "webhook");
        assert_eq!(ctx.access_reason, AccessReason::Webhook);
    }
}
