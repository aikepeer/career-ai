//! Referral finder — match companies in the user's career history against
//! companies in discovered/shortlisted listings to surface "you know
//! someone here" opportunities.
//!
//! The user's `Experience` entries list companies they've worked at. Any
//! listing at one of those companies is a potential referral path — the
//! user can reach out to former colleagues for a warm intro.

#![allow(clippy::cast_precision_loss)]

use serde::Serialize;

/// A single referral opportunity: a listing at a company the user has
/// worked at before.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReferralMatch {
    pub listing_id: String,
    pub company: String,
    pub title: String,
    pub url: String,
    /// Where the user knows the company from.
    pub connection_source: String,
    /// The user's role at that company.
    pub user_role: String,
}

/// A simplified listing shape — the caller passes whatever listing data
/// is available (from DB or in-memory).
#[derive(Debug, Clone)]
pub struct ListingRef<'a> {
    pub id: &'a str,
    pub company: &'a str,
    pub title: &'a str,
    pub url: &'a str,
}

/// Find referral opportunities by matching listing companies against
/// the user's past employers.
///
/// Matching is case-insensitive and uses substring containment in both
/// directions to catch "Google LLC" vs "Google" and "Meta" vs "Meta Platforms".
/// Common suffixes (Inc, LLC, Corp, Ltd, GmbH) are stripped before comparison.
pub fn find_referral_opportunities(
    listings: &[ListingRef<'_>],
    past_companies: &[(String, String)], // (company_name, user_role)
) -> Vec<ReferralMatch> {
    let normalized_past: Vec<(String, String)> = past_companies
        .iter()
        .map(|(c, r)| (normalize_company(c), r.clone()))
        .collect();

    let mut matches = Vec::new();

    for listing in listings {
        let norm_listing = normalize_company(listing.company);
        if norm_listing.is_empty() {
            continue;
        }

        for (norm_past, role) in &normalized_past {
            if norm_past.is_empty() {
                continue;
            }

            // Bidirectional substring match to catch partial names.
            let is_match = norm_listing.contains(norm_past)
                || norm_past.contains(&norm_listing)
                || fuzzy_eq(&norm_listing, norm_past);

            if is_match {
                matches.push(ReferralMatch {
                    listing_id: listing.id.to_string(),
                    company: listing.company.to_string(),
                    title: listing.title.to_string(),
                    url: listing.url.to_string(),
                    connection_source: format!(
                        "Previously worked at {} as {}",
                        listing.company, role
                    ),
                    user_role: role.clone(),
                });
                break; // One match per listing is enough.
            }
        }
    }

    matches
}

/// Extract past companies and roles from a profile's experience entries.
pub fn extract_past_companies(experience: &[crate::schema::Experience]) -> Vec<(String, String)> {
    experience
        .iter()
        .map(|e| (e.company.clone(), e.title.clone()))
        .collect()
}

/// Normalize a company name for comparison: lowercase, strip common
/// suffixes, trim whitespace.
fn normalize_company(name: &str) -> String {
    let lower = name.to_ascii_lowercase().trim().to_string();
    let suffixes = [
        " inc.",
        " inc",
        " llc",
        " corp.",
        " corp",
        " ltd",
        " ltd.",
        " gmbh",
        " co.",
        " co",
        " pvt ltd",
        " pvt. ltd.",
        " technologies",
        " tech",
        " labs",
        " systems",
        " software",
        " solutions",
        " group",
        " holdings",
    ];
    let mut result = lower.clone();
    for suffix in &suffixes {
        if result.ends_with(suffix) {
            result = result[..result.len() - suffix.len()].trim().to_string();
            break;
        }
    }
    result
}

/// Fuzzy equality: check if two normalized names share enough characters
/// to be considered the same company. Uses a simple Jaccard similarity
/// on token sets.
fn fuzzy_eq(a: &str, b: &str) -> bool {
    let tokens_a: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let tokens_b: std::collections::HashSet<&str> = b.split_whitespace().collect();
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return false;
    }
    let intersection = tokens_a.intersection(&tokens_b).count();
    let union = tokens_a.union(&tokens_b).count();
    // If >60% of tokens overlap, consider it a match.
    (intersection as f64 / union as f64) > 0.6
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_company_match() {
        let listings = vec![ListingRef {
            id: "l1",
            company: "Acme Corp",
            title: "Senior Engineer",
            url: "https://example.com/1",
        }];
        let past = vec![("Acme Corp".to_string(), "Junior Engineer".to_string())];
        let matches = find_referral_opportunities(&listings, &past);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].company, "Acme Corp");
        assert!(matches[0].connection_source.contains("Junior Engineer"));
    }

    #[test]
    fn test_suffix_stripped_match() {
        let listings = vec![ListingRef {
            id: "l1",
            company: "Acme",
            title: "Engineer",
            url: "https://example.com",
        }];
        let past = vec![("Acme Corp".to_string(), "Eng".to_string())];
        let matches = find_referral_opportunities(&listings, &past);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_no_match() {
        let listings = vec![ListingRef {
            id: "l1",
            company: "UnknownCo",
            title: "Engineer",
            url: "https://example.com",
        }];
        let past = vec![("Acme".to_string(), "Eng".to_string())];
        let matches = find_referral_opportunities(&listings, &past);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_fuzzy_match() {
        let listings = vec![ListingRef {
            id: "l1",
            company: "DeepMind Technologies",
            title: "ML Engineer",
            url: "https://example.com",
        }];
        let past = vec![(
            "deepmind technologies".to_string(),
            "Researcher".to_string(),
        )];
        let matches = find_referral_opportunities(&listings, &past);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn test_extract_past_companies() {
        use crate::schema::Experience;
        let exp = vec![
            Experience {
                title: "Engineer".into(),
                company: "Acme".into(),
                location: String::new(),
                start: "2020-01".into(),
                end: "2022-01".into(),
                bullets: vec![],
            },
            Experience {
                title: "Senior Engineer".into(),
                company: "Globex".into(),
                location: String::new(),
                start: "2022-02".into(),
                end: "present".into(),
                bullets: vec![],
            },
        ];
        let companies = extract_past_companies(&exp);
        assert_eq!(companies.len(), 2);
        assert_eq!(companies[0], ("Acme".to_string(), "Engineer".to_string()));
        assert_eq!(
            companies[1],
            ("Globex".to_string(), "Senior Engineer".to_string())
        );
    }
}
