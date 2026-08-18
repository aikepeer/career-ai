#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::classify::{apply_must_include_filter, classify, Decision};
use crate::rules::FilterRules;
use careerai_core::config::{CoreConfig, Domain, MatchConfig, UserConfig};
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
        matching: MatchConfig {
            embedding_model: String::new(),
            score_threshold: 0.0,
            must_include_skills: vec![],
            notify_threshold: 0.85,
        },
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
        MatchConfig {
            embedding_model: "x".into(),
            score_threshold: 0.0,
            must_include_skills: skills.iter().map(|s| (*s).into()).collect(),
            notify_threshold: 0.85,
        }
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
