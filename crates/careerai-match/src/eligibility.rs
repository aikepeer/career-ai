//! Eligibility gate (ported from ai-job-search).
//!
//! Checks citizenship, work-permit, and security-clearance requirements
//! before scoring. Saves LLM tokens by skipping listings the candidate is
//! ineligible for (hard requirements) or would need sponsorship for.

/// The candidate's authorization profile.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthProfile {
    /// Countries the candidate holds citizenship in (e.g., "US", "India").
    pub citizenships: Vec<String>,
    /// Countries/regions where the candidate has work authorization.
    pub work_authorization: Vec<String>,
    /// Whether the candidate holds an active security clearance.
    pub security_clearance: bool,
}

/// Result of an eligibility check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EligibilityResult {
    /// Candidate meets all requirements.
    Eligible,
    /// Candidate could apply but would need visa sponsorship.
    RequiresSponsorship,
    /// Candidate cannot apply (hard requirement not met).
    Ineligible,
}

/// Check whether the candidate is eligible for a listing based on its JD.
///
/// Checks in order of severity: security clearance (hard) → citizenship
/// (hard) → work authorization (soft → sponsorship). A listing with no
/// requirements returns `Eligible`.
#[must_use]
pub fn check_eligibility(jd_text: &str, user_auth: &AuthProfile) -> EligibilityResult {
    let lower = jd_text.to_lowercase();

    // 1. Security clearance (hard requirement).
    let needs_clearance =
        lower.contains("security clearance") || lower.contains("clearance required");
    if needs_clearance && !user_auth.security_clearance {
        return EligibilityResult::Ineligible;
    }

    // 2. Citizenship (hard requirement).
    let needs_citizenship = lower.contains("us citizen")
        || lower.contains("must be a us citizen")
        || lower.contains("citizenship required")
        || lower.contains("must be citizen");
    if needs_citizenship {
        // Check for US-specific citizenship requirement.
        let needs_us = lower.contains("us citizen") || lower.contains("u.s. citizen");
        if needs_us {
            if !user_auth
                .citizenships
                .iter()
                .any(|c| c.eq_ignore_ascii_case("US"))
            {
                return EligibilityResult::Ineligible;
            }
        } else if user_auth.citizenships.is_empty() {
            return EligibilityResult::Ineligible;
        }
    }

    // 3. Work authorization (soft → sponsorship).
    let needs_work_auth = lower.contains("work permit")
        || lower.contains("work authorization")
        || lower.contains("authorized to work");
    if needs_work_auth && user_auth.work_authorization.is_empty() {
        return EligibilityResult::RequiresSponsorship;
    }

    EligibilityResult::Eligible
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn auth(citizenships: &[&str], work_auth: &[&str], clearance: bool) -> AuthProfile {
        AuthProfile {
            citizenships: citizenships.iter().map(|s| (*s).to_string()).collect(),
            work_authorization: work_auth.iter().map(|s| (*s).to_string()).collect(),
            security_clearance: clearance,
        }
    }

    // --- No requirements ---

    #[test]
    fn no_requirements_is_eligible() {
        let user = auth(&["US"], &["US"], false);
        assert_eq!(
            check_eligibility("We are hiring a software engineer.", &user),
            EligibilityResult::Eligible
        );
    }

    // --- Citizenship checks ---

    #[test]
    fn us_citizen_required_user_has_us_is_eligible() {
        let user = auth(&["US"], &["US"], false);
        assert_eq!(
            check_eligibility("Must be a US citizen to apply.", &user),
            EligibilityResult::Eligible
        );
    }

    #[test]
    fn us_citizen_required_user_lacks_us_is_ineligible() {
        let user = auth(&["India"], &["US"], false);
        assert_eq!(
            check_eligibility("Must be a US citizen to apply.", &user),
            EligibilityResult::Ineligible
        );
    }

    #[test]
    fn citizenship_required_user_has_some_is_eligible() {
        let user = auth(&["India"], &[], false);
        assert_eq!(
            check_eligibility("Citizenship required for this role.", &user),
            EligibilityResult::Eligible
        );
    }

    #[test]
    fn citizenship_required_user_has_none_is_ineligible() {
        let user = auth(&[], &[], false);
        assert_eq!(
            check_eligibility("Citizenship required for this role.", &user),
            EligibilityResult::Ineligible
        );
    }

    #[test]
    fn must_be_citizen_user_has_none_is_ineligible() {
        let user = auth(&[], &[], false);
        assert_eq!(
            check_eligibility("Candidate must be citizen of an EU country.", &user),
            EligibilityResult::Ineligible
        );
    }

    // --- Work authorization checks ---

    #[test]
    fn work_permit_required_user_has_auth_is_eligible() {
        let user = auth(&["India"], &["US"], false);
        assert_eq!(
            check_eligibility("Work permit required for this position.", &user),
            EligibilityResult::Eligible
        );
    }

    #[test]
    fn work_authorization_required_user_has_auth_is_eligible() {
        let user = auth(&["India"], &["US"], false);
        assert_eq!(
            check_eligibility("Work authorization required to apply.", &user),
            EligibilityResult::Eligible
        );
    }

    #[test]
    fn work_permit_required_user_lacks_auth_requires_sponsorship() {
        let user = auth(&["India"], &[], false);
        assert_eq!(
            check_eligibility("Work permit required for this position.", &user),
            EligibilityResult::RequiresSponsorship
        );
    }

    #[test]
    fn must_be_authorized_user_lacks_auth_requires_sponsorship() {
        let user = auth(&["India"], &[], false);
        assert_eq!(
            check_eligibility("Candidate must be authorized to work.", &user),
            EligibilityResult::RequiresSponsorship
        );
    }

    // --- Security clearance checks ---

    #[test]
    fn security_clearance_required_user_has_it_is_eligible() {
        let user = auth(&["US"], &["US"], true);
        assert_eq!(
            check_eligibility("Security clearance required for this role.", &user),
            EligibilityResult::Eligible
        );
    }

    #[test]
    fn clearance_required_user_lacks_it_is_ineligible() {
        let user = auth(&["US"], &["US"], false);
        assert_eq!(
            check_eligibility("Clearance required for this position.", &user),
            EligibilityResult::Ineligible
        );
    }

    // --- Combined / priority checks ---

    #[test]
    fn clearance_ineligible_overrides_work_auth_sponsorship() {
        // JD requires both clearance and work permit; user lacks clearance.
        let user = auth(&["India"], &[], false);
        assert_eq!(
            check_eligibility("Security clearance required. Work permit required.", &user),
            EligibilityResult::Ineligible
        );
    }

    #[test]
    fn citizenship_ineligible_overrides_work_auth_sponsorship() {
        // JD requires citizenship and work permit; user lacks citizenship.
        let user = auth(&[], &[], false);
        assert_eq!(
            check_eligibility("Must be US citizen. Work authorization required.", &user),
            EligibilityResult::Ineligible
        );
    }

    #[test]
    fn citizenship_eligible_but_no_work_auth_requires_sponsorship() {
        // User has citizenship but no work authorization.
        let user = auth(&["US"], &[], false);
        assert_eq!(
            check_eligibility("Must be a US citizen. Work permit required.", &user),
            EligibilityResult::RequiresSponsorship
        );
    }
}
