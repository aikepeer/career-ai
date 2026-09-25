//! Posting legitimacy assessment (ported from career-ops Block G).
//!
//! Heuristic red-flag detection that scores postings as High Confidence,
//! Proceed with Caution, or Suspicious — separate from the match score.
//! A listing can score well on keyword match but still be flagged
//! suspicious (e.g., generic email domain, spam patterns).

use careerai_sources::RawListing;

/// Result of a posting legitimacy assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegitimacyScore {
    /// No red flags detected.
    HighConfidence,
    /// Minor red flags — review before applying.
    ProceedWithCaution,
    /// Strong red flags — likely a scam or ghost posting.
    Suspicious,
}

/// Assess a listing's legitimacy using heuristic red-flag checks.
///
/// Checks run most-severe-first: suspicious conditions (no company,
/// generic email domains, spam patterns) are checked before caution-level
/// conditions (no description, short description, unrealistic salary).
#[must_use]
pub fn assess_legitimacy(listing: &RawListing) -> LegitimacyScore {
    let desc_lower = listing.description.to_lowercase();
    let title_lower = listing.title.to_lowercase();

    // --- Suspicious conditions (highest priority) ---

    // No company name.
    if listing.company.trim().is_empty() {
        return LegitimacyScore::Suspicious;
    }

    // Generic email domain in the description.
    let has_generic_email = desc_lower.contains("@gmail.com")
        || desc_lower.contains("@yahoo.com")
        || desc_lower.contains("@hotmail.com");
    if has_generic_email {
        return LegitimacyScore::Suspicious;
    }

    // Spam patterns.
    let spam_patterns = ["urgent hiring", "immediate start", "work from home earn"];
    for pattern in &spam_patterns {
        if desc_lower.contains(pattern) {
            return LegitimacyScore::Suspicious;
        }
    }

    // --- Caution conditions ---

    // Empty description.
    if listing.description.trim().is_empty() {
        return LegitimacyScore::ProceedWithCaution;
    }

    // Very short description (< 50 chars).
    if listing.description.len() < 50 {
        return LegitimacyScore::ProceedWithCaution;
    }

    // High salary for non-executive roles.
    let is_executive = title_lower.contains("vp ")
        || title_lower.contains("vice president")
        || title_lower.contains("cto")
        || title_lower.contains("cfo")
        || title_lower.contains("ceo")
        || title_lower.contains("chief");
    if !is_executive {
        // Look for salary figures >= $500k (with or without commas).
        let high_salary = desc_lower.contains("$500k")
            || desc_lower.contains("$600k")
            || desc_lower.contains("$700k")
            || desc_lower.contains("$750k")
            || desc_lower.contains("$500,000")
            || desc_lower.contains("$600,000")
            || desc_lower.contains("$700,000")
            || desc_lower.contains("$750,000");
        if high_salary {
            return LegitimacyScore::ProceedWithCaution;
        }
    }

    LegitimacyScore::HighConfidence
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_sources::RawListing;

    fn listing(company: &str, title: &str, desc: &str) -> RawListing {
        RawListing {
            source: "test".into(),
            external_id: "1".into(),
            title: title.into(),
            company: company.into(),
            location: Some("Remote".into()),
            url: "https://example.com/1".into(),
            description: desc.into(),
            raw_json: None,
        }
    }

    // --- Suspicious conditions ---

    #[test]
    fn no_company_name_is_suspicious() {
        let l = listing("", "ML Engineer", "Build LLM applications for production.");
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn generic_email_domain_gmail_is_suspicious() {
        let l = listing(
            "Acme Corp",
            "Data Scientist",
            "We are hiring a data scientist. Contact recruiter@gmail.com to apply.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn generic_email_domain_yahoo_is_suspicious() {
        let l = listing(
            "Acme Corp",
            "Data Scientist",
            "Send your resume to hiring@yahoo.com for consideration.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn generic_email_domain_hotmail_is_suspicious() {
        let l = listing(
            "Acme Corp",
            "Data Scientist",
            "Email recruiter@hotmail.com with your portfolio.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn spam_pattern_urgent_hiring_is_suspicious() {
        let l = listing(
            "Acme Corp",
            "ML Engineer",
            "We are urgent hiring for an ML engineer to join our team. Great pay and benefits.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn spam_pattern_immediate_start_is_suspicious() {
        let l = listing(
            "Acme Corp",
            "ML Engineer",
            "Looking for an ML engineer. Immediate start available for the right candidate.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn spam_pattern_work_from_home_earn_is_suspicious() {
        let l = listing(
            "Acme Corp",
            "Data Entry",
            "Amazing opportunity! Work from home earn $5000 weekly. No experience needed.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    // --- Caution conditions ---

    #[test]
    fn company_but_no_description_is_caution() {
        let l = listing("Acme Corp", "ML Engineer", "");
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::ProceedWithCaution);
    }

    #[test]
    fn very_short_description_is_caution() {
        let l = listing("Acme Corp", "ML Engineer", "Hiring now. Apply today.");
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::ProceedWithCaution);
    }

    #[test]
    fn high_salary_non_executive_is_caution() {
        let l = listing(
            "Acme Corp",
            "Software Engineer",
            "We are hiring a software engineer. Salary $600k per year with full benefits.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::ProceedWithCaution);
    }

    #[test]
    fn high_salary_with_commas_non_executive_is_caution() {
        let l = listing(
            "Acme Corp",
            "Developer",
            "Looking for a developer. Compensation: $750,000 annually.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::ProceedWithCaution);
    }

    // --- High confidence conditions ---

    #[test]
    fn normal_listing_is_high_confidence() {
        let l = listing(
            "Acme Corp",
            "Senior ML Engineer",
            "We are looking for a senior ML engineer to build and deploy LLM-based \
             applications. You will work with a team of researchers and engineers \
             to ship production systems. Rust + Python stack.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::HighConfidence);
    }

    #[test]
    fn high_salary_executive_is_high_confidence() {
        let l = listing(
            "Acme Corp",
            "VP of Engineering",
            "We are hiring a VP of Engineering. Salary $600k per year with equity.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::HighConfidence);
    }

    #[test]
    fn high_salary_cto_is_high_confidence() {
        let l = listing(
            "Acme Corp",
            "CTO",
            "Seeking a CTO. Compensation $700,000 plus significant equity package.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::HighConfidence);
    }

    #[test]
    fn reasonable_salary_is_high_confidence() {
        let l = listing(
            "Acme Corp",
            "ML Engineer",
            "ML engineer role. Salary $150k-$200k depending on experience. Remote.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::HighConfidence);
    }

    // --- Priority / override behavior ---

    #[test]
    fn suspicious_overrides_caution() {
        // Has both a spam pattern (suspicious) and short description (caution).
        let l = listing("Acme", "ML Engineer", "Urgent hiring! Apply now.");
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn no_company_overrides_short_description() {
        // No company (suspicious) + short description (caution) → suspicious.
        let l = listing("", "ML Engineer", "Short desc.");
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }

    #[test]
    fn gmail_email_overrides_high_salary_executive() {
        // Email domain (suspicious) + high salary + executive title.
        let l = listing(
            "Acme",
            "CTO",
            "Hiring a CTO. Salary $700k. Contact recruiter@gmail.com.",
        );
        assert_eq!(assess_legitimacy(&l), LegitimacyScore::Suspicious);
    }
}
