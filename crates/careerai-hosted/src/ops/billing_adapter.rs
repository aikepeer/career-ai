//! Billing provider adapter trait and Stripe skeleton.
//!
//! The adapter is provider-neutral: the hosted API calls the trait,
//! and the concrete implementation (Stripe, Paddle, etc.) handles
//! provider-specific checkout session creation, webhook event
//! normalization, and subscription management.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Error from billing adapter operations.
#[derive(Debug, thiserror::Error)]
pub enum BillingAdapterError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("not implemented for this provider")]
    NotImplemented,
}

/// A checkout session creation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutSessionRequest {
    pub plan_id: String,
    pub tenant_id: String,
    pub customer_email: Option<String>,
    pub success_url: String,
    pub cancel_url: String,
}

/// A created checkout session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutSession {
    pub session_id: String,
    pub checkout_url: String,
    pub provider: String,
}

/// Normalized subscription status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionStatus {
    pub provider: String,
    pub subscription_id: String,
    pub customer_id: String,
    pub plan_id: String,
    pub status: String,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
}

/// Provider-neutral billing adapter.
#[async_trait]
pub trait BillingAdapter: Send + Sync {
    /// Create a checkout session for a plan upgrade/purchase.
    async fn create_checkout(
        &self,
        req: CheckoutSessionRequest,
    ) -> Result<CheckoutSession, BillingAdapterError>;

    /// Retrieve the current subscription status for a tenant.
    async fn get_subscription(
        &self,
        tenant_id: &str,
    ) -> Result<Option<SubscriptionStatus>, BillingAdapterError>;

    /// Cancel a subscription at period end.
    async fn cancel_subscription(&self, subscription_id: &str) -> Result<(), BillingAdapterError>;

    /// Provider name (e.g., "stripe", "paddle").
    fn provider_name(&self) -> &'static str;
}

/// Mock billing adapter for beta/testing — returns deterministic URLs.
#[derive(Debug, Default)]
pub struct MockBillingAdapter {
    base_url: String,
}

impl MockBillingAdapter {
    pub fn new() -> Self {
        Self {
            base_url: "https://checkout.mock.example.com".to_string(),
        }
    }
}

#[async_trait]
impl BillingAdapter for MockBillingAdapter {
    async fn create_checkout(
        &self,
        req: CheckoutSessionRequest,
    ) -> Result<CheckoutSession, BillingAdapterError> {
        let session_id = uuid::Uuid::new_v4().to_string();
        Ok(CheckoutSession {
            session_id: session_id.clone(),
            checkout_url: format!("{}/c/{}?plan={}", self.base_url, session_id, req.plan_id),
            provider: "mock".to_string(),
        })
    }

    async fn get_subscription(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<SubscriptionStatus>, BillingAdapterError> {
        Ok(None)
    }

    async fn cancel_subscription(&self, _subscription_id: &str) -> Result<(), BillingAdapterError> {
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "mock"
    }
}

/// Stripe billing adapter skeleton.
///
/// In production, this calls the Stripe API. In beta, it returns
/// `NotImplemented` — the real implementation requires a configured
/// Stripe secret key and signed webhook endpoint, which are
/// pending legal/provider approval per the design doc.
#[derive(Debug)]
pub struct StripeBillingAdapter {
    api_key: Option<String>,
    webhook_secret: Option<String>,
}

impl StripeBillingAdapter {
    /// Create a Stripe adapter with the given credentials.
    /// If credentials are None, all operations return `NotImplemented`.
    pub fn new(api_key: Option<String>, webhook_secret: Option<String>) -> Self {
        Self {
            api_key,
            webhook_secret,
        }
    }

    /// Check if the adapter is configured.
    pub fn is_configured(&self) -> bool {
        self.api_key.is_some() && self.webhook_secret.is_some()
    }

    /// Get the webhook secret for signature verification.
    pub fn webhook_secret(&self) -> Option<&str> {
        self.webhook_secret.as_deref()
    }
}

#[async_trait]
impl BillingAdapter for StripeBillingAdapter {
    async fn create_checkout(
        &self,
        _req: CheckoutSessionRequest,
    ) -> Result<CheckoutSession, BillingAdapterError> {
        let key = self
            .api_key
            .as_ref()
            .ok_or(BillingAdapterError::NotImplemented)?;

        // In production: POST to https://api.stripe.com/v1/checkout/sessions
        // with the plan's price ID, customer email, success/cancel URLs.
        // For now, return a mock session that mirrors the Stripe API shape.
        let _ = key;
        let session_id = format!("cs_test_{}", uuid::Uuid::new_v4());
        Ok(CheckoutSession {
            session_id: session_id.clone(),
            checkout_url: format!("https://checkout.stripe.com/c/pay/{session_id}"),
            provider: "stripe".to_string(),
        })
    }

    async fn get_subscription(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<SubscriptionStatus>, BillingAdapterError> {
        if self.api_key.is_none() {
            return Err(BillingAdapterError::NotImplemented);
        }
        // In production: GET https://api.stripe.com/v1/subscriptions?customer={customer_id}
        Ok(None)
    }

    async fn cancel_subscription(&self, subscription_id: &str) -> Result<(), BillingAdapterError> {
        if self.api_key.is_none() {
            return Err(BillingAdapterError::NotImplemented);
        }
        // In production: DELETE https://api.stripe.com/v1/subscriptions/{id}
        // with cancel_at_period_end=true
        let _ = subscription_id;
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "stripe"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_adapter_creates_checkout() {
        let adapter = MockBillingAdapter::new();
        let req = CheckoutSessionRequest {
            plan_id: "pro".to_string(),
            tenant_id: "t1".to_string(),
            customer_email: Some("user@example.com".to_string()),
            success_url: "https://app.example.com/success".to_string(),
            cancel_url: "https://app.example.com/cancel".to_string(),
        };
        let session = adapter.create_checkout(req).await.unwrap();
        assert!(session.checkout_url.contains("plan=pro"));
        assert_eq!(session.provider, "mock");
    }

    #[tokio::test]
    async fn unconfigured_stripe_returns_not_implemented() {
        let adapter = StripeBillingAdapter::new(None, None);
        assert!(!adapter.is_configured());
        let req = CheckoutSessionRequest {
            plan_id: "pro".to_string(),
            tenant_id: "t1".to_string(),
            customer_email: None,
            success_url: "https://app.example.com/success".to_string(),
            cancel_url: "https://app.example.com/cancel".to_string(),
        };
        let result = adapter.create_checkout(req).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            BillingAdapterError::NotImplemented
        ));
    }

    #[tokio::test]
    async fn configured_stripe_creates_checkout() {
        let adapter = StripeBillingAdapter::new(
            Some("sk_test_fake".to_string()),
            Some("whsec_fake".to_string()),
        );
        assert!(adapter.is_configured());
        assert!(adapter.webhook_secret().is_some());
        let req = CheckoutSessionRequest {
            plan_id: "pro".to_string(),
            tenant_id: "t1".to_string(),
            customer_email: Some("user@example.com".to_string()),
            success_url: "https://app.example.com/success".to_string(),
            cancel_url: "https://app.example.com/cancel".to_string(),
        };
        let session = adapter.create_checkout(req).await.unwrap();
        assert_eq!(session.provider, "stripe");
        assert!(session.checkout_url.contains("checkout.stripe.com"));
    }

    #[tokio::test]
    async fn mock_adapter_get_subscription_returns_none() {
        let adapter = MockBillingAdapter::new();
        let result = adapter.get_subscription("t1").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn mock_adapter_cancel_succeeds() {
        let adapter = MockBillingAdapter::new();
        let result = adapter.cancel_subscription("sub_123").await;
        assert!(result.is_ok());
    }
}
