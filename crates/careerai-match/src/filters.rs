//! Hard pass/reject filters applied before scoring.
//!
//! Checks are ordered cheap-first: title exclusions (short string) →
//! location allowlist → JD keyword exclusions (longer string scan) →
//! domain keywords + required-keyword rule.

use careerai_core::config::{CoreConfig, Domain};
use careerai_sources::RawListing;

use crate::rules::FilterRules;

/// Outcome of running the filters against one listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Keep,
    Reject(&'static str),
}

impl Decision {
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

pub fn classify(listing: &RawListing, cfg: &CoreConfig, rules: &FilterRules) -> Decision {
    let title_lc = listing.title.to_lowercase();
    let desc_lc = listing.description.to_lowercase();

    if rules
        .exclude_titles
        .iter()
        .any(|t| title_lc.contains(&t.to_lowercase()))
    {
        return Decision::Reject("excluded title");
    }

    if !location_ok(listing, &cfg.user.locations) {
        return Decision::Reject("location not in allowlist");
    }

    if rules
        .exclude_keywords_in_jd
        .iter()
        .any(|k| desc_lc.contains(&k.to_lowercase()))
    {
        return Decision::Reject("excluded JD keyword");
    }

    if !rules.require_any_keyword_in_jd.is_empty()
        && !rules
            .require_any_keyword_in_jd
            .iter()
            .any(|k| desc_lc.contains(&k.to_lowercase()))
    {
        return Decision::Reject("no required JD keyword");
    }

    if !cfg.domains.is_empty() && !any_domain_matches(&title_lc, &desc_lc, &cfg.domains) {
        return Decision::Reject("no configured domain keyword in title or JD");
    }

    Decision::Keep
}

/// Location check:
/// - Empty allowlist → keep everything.
/// - Listing location missing → reject (caller can override by adding "" to
///   the allowlist if they want unknown-location jobs to pass).
/// - Otherwise: allowed iff the listing location contains any allowed token
///   (case-insensitive substring match). "Remote" in allowlist matches
///   "Remote - India", "Remote, US", etc.
fn location_ok(listing: &RawListing, allowlist: &[String]) -> bool {
    if allowlist.is_empty() {
        return true;
    }
    let Some(loc) = listing.location.as_deref() else {
        return false;
    };
    let loc_lc = loc.to_lowercase();
    allowlist
        .iter()
        .any(|allowed| loc_lc.contains(&allowed.to_lowercase()))
}

fn any_domain_matches(title_lc: &str, desc_lc: &str, domains: &[Domain]) -> bool {
    domains.iter().any(|d| {
        d.keywords_any
            .iter()
            .any(|k| title_lc.contains(&k.to_lowercase()) || desc_lc.contains(&k.to_lowercase()))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_core::config::{MatchConfig, UserConfig};
    use std::collections::HashMap;

    fn cfg(locations: &[&str], domain_kws: &[&str]) -> CoreConfig {
        CoreConfig {
            user: UserConfig {
                locations: locations.iter().map(|s| (*s).to_string()).collect(),
                timezone: "UTC".into(),
                work_auth: HashMap::new(),
            },
            domains: if domain_kws.is_empty() {
                vec![]
            } else {
                vec![Domain {
                    name: "test".into(),
                    keywords_any: domain_kws.iter().map(|s| (*s).to_string()).collect(),
                }]
            },
            matching: MatchConfig {
                embedding_model: String::new(),
                score_threshold: 0.0,
            },
            rates: careerai_core::config::RatesConfig::default(),
            submit: careerai_core::config::SubmitConfig::default(),
            llm: careerai_core::config::LlmConfig::default(),
            scheduler: careerai_core::config::SchedulerConfig::default(),
            sources: careerai_core::config::SourcesConfig::default(),
        }
    }

    fn listing(title: &str, location: Option<&str>, desc: &str) -> RawListing {
        RawListing {
            source: "test".into(),
            external_id: "1".into(),
            title: title.into(),
            company: "Acme".into(),
            location: location.map(str::to_string),
            url: "https://example.com/1".into(),
            description: desc.into(),
            raw_json: None,
        }
    }

    #[test]
    fn keeps_listing_that_matches_domain_and_location() {
        let cfg = cfg(&["Remote", "Delhi"], &["robotics", "llm"]);
        let rules = FilterRules::default();
        let l = listing(
            "Senior ML Engineer",
            Some("Remote - India"),
            "Build LLM apps for robotics.",
        );
        assert_eq!(classify(&l, &cfg, &rules), Decision::Keep);
    }

    #[test]
    fn rejects_excluded_title() {
        let cfg = cfg(&["Remote"], &[]);
        let rules = FilterRules {
            exclude_titles: vec!["recruiter".into()],
            ..Default::default()
        };
        let l = listing("Senior Technical Recruiter", Some("Remote"), "anything");
        assert!(matches!(
            classify(&l, &cfg, &rules),
            Decision::Reject("excluded title")
        ));
    }

    #[test]
    fn rejects_location_not_in_allowlist() {
        let cfg = cfg(&["Remote", "Delhi"], &[]);
        let rules = FilterRules::default();
        let l = listing("ML Engineer", Some("San Francisco"), "anything");
        assert!(matches!(
            classify(&l, &cfg, &rules),
            Decision::Reject("location not in allowlist")
        ));
    }

    #[test]
    fn rejects_listing_missing_location_when_allowlist_set() {
        let cfg = cfg(&["Remote"], &[]);
        let rules = FilterRules::default();
        let l = listing("ML Engineer", None, "anything");
        assert!(matches!(
            classify(&l, &cfg, &rules),
            Decision::Reject("location not in allowlist")
        ));
    }

    #[test]
    fn rejects_jd_with_excluded_keyword() {
        let cfg = cfg(&["Remote"], &[]);
        let rules = FilterRules {
            exclude_keywords_in_jd: vec!["on-site only".into()],
            ..Default::default()
        };
        let l = listing("ML Engineer", Some("Remote"), "Hybrid, on-site only.");
        assert!(matches!(
            classify(&l, &cfg, &rules),
            Decision::Reject("excluded JD keyword")
        ));
    }

    #[test]
    fn rejects_when_no_required_keyword_present() {
        let cfg = cfg(&["Remote"], &[]);
        let rules = FilterRules {
            require_any_keyword_in_jd: vec!["rust".into(), "embedded".into()],
            ..Default::default()
        };
        let l = listing("Engineer", Some("Remote"), "We use Java.");
        assert!(matches!(
            classify(&l, &cfg, &rules),
            Decision::Reject("no required JD keyword")
        ));
    }

    #[test]
    fn rejects_when_no_configured_domain_keyword_hits() {
        let cfg = cfg(&["Remote"], &["robotics", "llm"]);
        let rules = FilterRules::default();
        let l = listing("Frontend Engineer", Some("Remote"), "React, CSS, UI.");
        assert!(matches!(
            classify(&l, &cfg, &rules),
            Decision::Reject("no configured domain keyword in title or JD")
        ));
    }
}
