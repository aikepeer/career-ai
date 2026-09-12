//! Hosted multi-tenant API surface for career-ai.
//!
//! This crate will host the authenticated API, tenant context middleware,
//! entitlement admission, and worker adapters for the hosted subscription
//! product. PR 1 scaffolds the crate with only type re-exports from
//! [`careerai_core`] — no business logic, no API endpoints, no database.
//!
//! See:
//! - [`docs/HOSTED_PRODUCT_DESIGN.md`] — full design document and PR plan.
//! - [`docs/HOSTED_BETA_SCOPE.md`] — beta inclusions, exclusions, exit criteria.
//! - [`docs/HOSTED_DATA_CLASSIFICATION.md`] — C0–C4 classification and retention.
//! - [`docs/HOSTED_THREAT_MODEL.md`] — trust boundaries, attack surfaces, mitigations.
//! - [`docs/HOSTED_UNVERIFIED_DEPENDENCIES.md`] — pending vendor/legal items.

/// Re-export core types so hosted modules share a single source of truth.
///
/// PR 2 will add auth, session, and workspace types. PR 3 will add tenant
/// context and RLS middleware. Until then this crate compiles independently
/// and depends only on [`careerai_core`].
pub use careerai_core;

/// Data classification levels for the hosted product.
///
/// See `docs/HOSTED_DATA_CLASSIFICATION.md` for the full table, processing
/// rules, and retention policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DataClass {
    /// Public data (listing text, public news).
    C0,
    /// Personal data (profile, resume, interview answers).
    C1,
    /// Sensitive data (ID/onboarding, offer, receipts, compensation).
    C2,
    /// Secret data (OAuth tokens, cookies, provider keys).
    C3,
    /// Restricted data (recordings, support exports, legal hold).
    C4,
}

impl DataClass {
    /// Returns `true` if this class may be processed by hosted LLM workers
    /// by default (without explicit opt-in).
    pub fn default_llm_allowed(self) -> bool {
        matches!(self, DataClass::C0)
    }

    /// Returns `true` if this class requires proxy-mediated object access
    /// (no storage-native signed URLs).
    pub fn requires_proxy(self) -> bool {
        matches!(self, DataClass::C1 | DataClass::C2 | DataClass::C3 | DataClass::C4)
    }
}

/// Beta inclusion/exclusion check.
///
/// See `docs/HOSTED_BETA_SCOPE.md` for the full beta scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BetaFeature {
    /// Account and workspace creation, profile import, target company selection.
    AccountAndWorkspace,
    /// Permitted ATS/feed discovery from reviewed allowlist.
    PermittedDiscovery,
    /// Target-company preparation program.
    PreparationProgram,
    /// Review-only tailored artifacts (preview/download, no live submission).
    ReviewOnlyArtifacts,
    /// Manual application, interview, email/call outcome records.
    ManualOutcomeTracking,
    /// Versioned encrypted export and user deletion request.
    ExportAndDeletion,
    /// Free/paid entitlement admission and usage visibility.
    EntitlementAdmission,
}

impl BetaFeature {
    /// Returns `true` if the feature is included in the hosted beta.
    pub fn is_beta_included(self) -> bool {
        true
    }
}

/// Features explicitly excluded from the hosted beta.
///
/// Each of these requires a separate PR and legal/vendor approval before
/// activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludedFromBeta {
    /// Live submission to ATS or employer portals.
    LiveSubmission,
    /// All browser automation.
    BrowserAutomation,
    /// Mailbox OAuth/sync.
    MailboxSync,
    /// Call recording or transcript storage.
    CallRecording,
    /// Document vault beyond generated artifacts.
    DocumentVault,
    /// Tax cards, calculations, or recommendations.
    TaxCalculation,
    /// Reimbursement processing or claim submission.
    ReimbursementProcessing,
    /// Market news aggregation.
    MarketNewsAggregation,
    /// Coach or concierge access.
    ConciergeAccess,
    /// Automatic communication (email send, message posting).
    AutomaticCommunication,
}

impl ExcludedFromBeta {
    /// Returns `true` if the feature is excluded from beta (always `true`
    /// for all variants — this method exists for symmetric API usage and
    /// to document the intent).
    pub fn is_beta_excluded(self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_class_default_llm() {
        assert!(DataClass::C0.default_llm_allowed());
        assert!(!DataClass::C1.default_llm_allowed());
        assert!(!DataClass::C2.default_llm_allowed());
        assert!(!DataClass::C3.default_llm_allowed());
        assert!(!DataClass::C4.default_llm_allowed());
    }

    #[test]
    fn data_class_proxy_requirement() {
        assert!(!DataClass::C0.requires_proxy());
        assert!(DataClass::C1.requires_proxy());
        assert!(DataClass::C2.requires_proxy());
        assert!(DataClass::C3.requires_proxy());
        assert!(DataClass::C4.requires_proxy());
    }

    #[test]
    fn beta_features_all_included() {
        let features = [
            BetaFeature::AccountAndWorkspace,
            BetaFeature::PermittedDiscovery,
            BetaFeature::PreparationProgram,
            BetaFeature::ReviewOnlyArtifacts,
            BetaFeature::ManualOutcomeTracking,
            BetaFeature::ExportAndDeletion,
            BetaFeature::EntitlementAdmission,
        ];
        assert!(features.iter().all(|f| f.is_beta_included()));
    }

    #[test]
    fn excluded_features_all_excluded() {
        let excluded = [
            ExcludedFromBeta::LiveSubmission,
            ExcludedFromBeta::BrowserAutomation,
            ExcludedFromBeta::MailboxSync,
            ExcludedFromBeta::CallRecording,
            ExcludedFromBeta::DocumentVault,
            ExcludedFromBeta::TaxCalculation,
            ExcludedFromBeta::ReimbursementProcessing,
            ExcludedFromBeta::MarketNewsAggregation,
            ExcludedFromBeta::ConciergeAccess,
            ExcludedFromBeta::AutomaticCommunication,
        ];
        assert!(excluded.iter().all(|f| f.is_beta_excluded()));
    }

    #[test]
    fn data_class_serializes_uppercase() {
        let json = serde_json::to_string(&DataClass::C2).unwrap();
        assert_eq!(json, "\"C2\"");
    }

    #[test]
    fn beta_feature_serializes_snake_case() {
        let json = serde_json::to_string(&BetaFeature::PreparationProgram).unwrap();
        assert_eq!(json, "\"preparation_program\"");
    }
}
