//! Data classification levels and beta scope types.
//!
//! See `docs/HOSTED_DATA_CLASSIFICATION.md` and `docs/HOSTED_BETA_SCOPE.md`.

/// Data classification levels for the hosted product.
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
    pub fn default_llm_allowed(self) -> bool {
        matches!(self, DataClass::C0)
    }

    pub fn requires_proxy(self) -> bool {
        matches!(self, DataClass::C1 | DataClass::C2 | DataClass::C3 | DataClass::C4)
    }
}

/// Beta inclusion/exclusion check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BetaFeature {
    AccountAndWorkspace,
    PermittedDiscovery,
    PreparationProgram,
    ReviewOnlyArtifacts,
    ManualOutcomeTracking,
    ExportAndDeletion,
    EntitlementAdmission,
}

impl BetaFeature {
    pub fn is_beta_included(self) -> bool {
        true
    }
}

/// Features explicitly excluded from the hosted beta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludedFromBeta {
    LiveSubmission,
    BrowserAutomation,
    MailboxSync,
    CallRecording,
    DocumentVault,
    TaxCalculation,
    ReimbursementProcessing,
    MarketNewsAggregation,
    ConciergeAccess,
    AutomaticCommunication,
}

impl ExcludedFromBeta {
    pub fn is_beta_excluded(self) -> bool {
        true
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn data_class_default_llm() {
        assert!(DataClass::C0.default_llm_allowed());
        assert!(!DataClass::C1.default_llm_allowed());
    }

    #[test]
    fn data_class_proxy_requirement() {
        assert!(!DataClass::C0.requires_proxy());
        assert!(DataClass::C1.requires_proxy());
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
    fn data_class_serializes_uppercase() {
        let json = serde_json::to_string(&DataClass::C2).unwrap();
        assert_eq!(json, "\"C2\"");
    }
}
