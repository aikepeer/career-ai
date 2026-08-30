#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::classify::{apply_must_include_filter, classify, Decision};
use crate::rules::FilterRules;
use careerai_core::config::{CoreConfig, Domain, UserConfig};
use careerai_sources::RawListing;
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
        matching: crate::tests_shared::test_match_config(),
        rates: careerai_core::config::RatesConfig::default(),
        submit: careerai_core::config::SubmitConfig::default(),
        llm: careerai_core::config::LlmConfig::default(),
        scheduler: careerai_core::config::SchedulerConfig::default(),
        sources: careerai_core::config::SourcesConfig::default(),
        render: careerai_core::config::RenderConfig::default(),
        notify: careerai_notify::NotifyConfig::default(),
        dashboard: careerai_core::config::DashboardConfig::default(),
    }
}

#[test]
fn rejects_blacklisted_company() {
    let mut c = cfg(&[], &[]);
    c.matching.company_blacklist = vec!["wayfair".into(), "crossover".into()];
    let listing = RawListing {
        source: "t".into(),
        external_id: "1".into(),
        title: "Senior ML Engineer".into(),
        company: "Wayfair".into(),
        location: Some("Remote".into()),
        url: "https://x".into(),
        description: "machine learning".into(),
        raw_json: None,
    };
    assert_eq!(
        classify(&listing, &c, &FilterRules::default()),
        Decision::Reject("blacklisted company")
    );
}

#[test]
fn rejects_blacklisted_title_and_location() {
    let mut c = cfg(&[], &[]);
    c.matching.title_blacklist = vec!["sales representative".into()];
    c.matching.location_blacklist = vec!["brazil".into()];

    let bad_title = RawListing {
        source: "t".into(),
        external_id: "2".into(),
        title: "Sales Representative".into(),
        company: "Acme".into(),
        location: Some("Berlin".into()),
        url: "https://x".into(),
        description: "sales".into(),
        raw_json: None,
    };
    assert_eq!(
        classify(&bad_title, &c, &FilterRules::default()),
        Decision::Reject("blacklisted title")
    );

    let bad_loc = RawListing {
        source: "t".into(),
        external_id: "3".into(),
        title: "ML Engineer".into(),
        company: "Acme".into(),
        location: Some("São Paulo, Brazil".into()),
        url: "https://x".into(),
        description: "machine learning".into(),
        raw_json: None,
    };
    assert_eq!(
        classify(&bad_loc, &c, &FilterRules::default()),
        Decision::Reject("blacklisted location")
    );
}

#[test]
fn blacklists_are_case_insensitive_and_empty_company_passes() {
    let mut c = cfg(&[], &[]);
    c.matching.company_blacklist = vec!["Acme".into()];
    // Case-insensitive: lowercase company still rejected.
    let listing = RawListing {
        source: "t".into(),
        external_id: "4".into(),
        title: "Engineer".into(),
        company: "acme robotics".into(),
        location: None,
        url: "https://x".into(),
        description: "x".into(),
        raw_json: None,
    };
    assert!(matches!(
        classify(&listing, &c, &FilterRules::default()),
        Decision::Reject("blacklisted company")
    ));
    // Empty company is never blacklisted (unknown, not blocked).
    let no_company = RawListing {
        source: "t".into(),
        external_id: "5".into(),
        title: "Engineer".into(),
        company: String::new(),
        location: None,
        url: "https://x".into(),
        description: "x".into(),
        raw_json: None,
    };
    assert_eq!(
        classify(&no_company, &c, &FilterRules::default()),
        Decision::Keep
    );
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
fn classify_rejects_listing_missing_required_skill() {
    let mut cfg = cfg(&["Remote"], &[]);
    cfg.matching.must_include_skills = vec!["rust".into()];
    let rules = FilterRules::default();
    let l = listing("Senior ML Engineer", Some("Remote"), "Python, TensorFlow.");
    assert!(matches!(
        classify(&l, &cfg, &rules),
        Decision::Reject("missing required skill")
    ));
}

#[test]
fn must_include_filter_passes_when_any_one_skill_present() {
    // `must_include_skills` is an OR-list (documented "at least one"), not
    // an AND-list. Pins the semantics against regressions to `.all()`.
    let mut cfg = cfg(&[], &[]);
    cfg.matching.must_include_skills = vec!["rust".into(), "tokio".into()];
    let l = listing("Backend Engineer", None, "We write Rust services.");
    assert!(apply_must_include_filter(&l, &cfg.matching));
}

#[test]
fn classify_allows_missing_location_when_empty_sentinel_present() {
    let mut cfg = cfg(&[], &[]);
    cfg.user.locations = vec![String::new(), "Remote".into()];
    let rules = FilterRules::default();
    let l = listing("Embedded Engineer", None, "firmware rust");
    assert_eq!(classify(&l, &cfg, &rules), Decision::Keep);
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
fn allows_missing_location_when_empty_string_in_allowlist() {
    let cfg = cfg(&["Remote", ""], &[]);
    let rules = FilterRules::default();
    let l = listing("ML Engineer", None, "anything");
    assert_eq!(classify(&l, &cfg, &rules), Decision::Keep);
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

// --- must_include tests ---

mod must_include_tests {
    use super::*;
    use careerai_core::config::MatchConfig;

    fn cfg_with_required(skills: &[&str]) -> MatchConfig {
        let mut c = crate::tests_shared::test_match_config();
        c.must_include_skills = skills.iter().map(|s| (*s).into()).collect();
        c
    }

    fn make_listing(title: &str, desc: &str) -> RawListing {
        RawListing {
            source: "test".into(),
            external_id: "1".into(),
            title: title.into(),
            company: "Acme".into(),
            location: None,
            url: "https://example.com/1".into(),
            description: desc.into(),
            raw_json: None,
        }
    }

    #[test]
    fn missing_required_skill_filters_out() {
        let listing = make_listing("Frontend Engineer", "We use React and GraphQL.");
        let cfg = cfg_with_required(&["rust", "tokio"]);
        assert!(!apply_must_include_filter(&listing, &cfg));
    }

    #[test]
    fn matching_required_skill_passes() {
        let listing = make_listing("Backend Engineer", "Rust + tokio shop, async-heavy.");
        let cfg = cfg_with_required(&["rust"]);
        assert!(apply_must_include_filter(&listing, &cfg));
    }

    #[test]
    fn empty_required_list_passes_everything() {
        let listing = make_listing("Anything", "Anything");
        let cfg = cfg_with_required(&[]);
        assert!(apply_must_include_filter(&listing, &cfg));
    }

    #[test]
    fn match_is_case_insensitive() {
        let listing = make_listing("Senior Engineer", "We use RUST and Tokio.");
        let cfg = cfg_with_required(&["rust"]);
        assert!(apply_must_include_filter(&listing, &cfg));
    }
}

mod boundary_keyword_tests {
    use super::super::classify::contains_keyword_boundary;

    #[test]
    fn single_letter_c_does_not_match_inside_words() {
        assert!(!contains_keyword_boundary(
            "we are looking for a candidate to scale our company",
            "c"
        ));
        assert!(contains_keyword_boundary(
            "requires experience in c and assembly",
            "c"
        ));
        assert!(contains_keyword_boundary("c, c++, linux", "c"));
        assert!(contains_keyword_boundary("c/c++ embedded", "c"));
    }

    #[test]
    fn ai_ml_does_not_match_inside_english_words() {
        assert!(!contains_keyword_boundary(
            "maintain email detail contain claims",
            "ai"
        ));
        assert!(!contains_keyword_boundary(
            "html yaml xml seamless workflow",
            "ml"
        ));
        assert!(contains_keyword_boundary(
            "senior ai engineer building llms",
            "ai"
        ));
        assert!(contains_keyword_boundary("hands-on ml experience", "ml"));
        assert!(contains_keyword_boundary("edge ai/ml applications", "ai"));
    }

    #[test]
    fn go_does_not_match_algorithm_or_ongoing() {
        assert!(!contains_keyword_boundary(
            "designing algorithms for ongoing projects",
            "go"
        ));
        assert!(contains_keyword_boundary(
            "backend services in go and rust",
            "go"
        ));
    }
}
