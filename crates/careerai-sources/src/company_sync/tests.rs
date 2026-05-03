#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_core::config::{CoreConfig, Domain};

use super::probe::{listing_matches, score_listings};
use super::seed::{load_embedded_seed, AtsVendor, SeedEntry};

mod sync_tests;

pub(crate) fn cfg_with_domains(domains: Vec<Domain>) -> CoreConfig {
    let mut cfg: CoreConfig =
        serde_yaml::from_str(careerai_core::config::EMBEDDED_DEFAULTS).unwrap();
    cfg.domains = domains;
    cfg.sources.greenhouse.companies.clear();
    cfg.sources.lever.companies.clear();
    cfg.sources.ashby.companies.clear();
    cfg
}

pub(crate) fn ai_ml_domain() -> Domain {
    Domain {
        name: "ai_ml".into(),
        keywords_any: vec!["machine learning".into(), "llm".into()],
    }
}

pub(crate) fn robotics_domain() -> Domain {
    Domain {
        name: "robotics_embedded".into(),
        keywords_any: vec!["robotics".into(), "embedded".into()],
    }
}

fn raw(title: &str, description: &str) -> crate::base::RawListing {
    crate::base::RawListing {
        source: "test".into(),
        external_id: "id".into(),
        title: title.into(),
        company: "co".into(),
        location: None,
        url: "u".into(),
        description: description.into(),
        raw_json: None,
    }
}

#[test]
fn listing_matches_is_case_insensitive_across_title_and_description() {
    let l = raw("Senior LLM Engineer", "Build large language model agents.");
    let kws = vec!["LLM".to_string()];
    assert!(listing_matches(&l, &kws));

    let l2 = raw("Senior Backend", "We deploy machine learning systems.");
    let kws2 = vec!["machine learning".to_string()];
    assert!(listing_matches(&l2, &kws2));

    let l3 = raw("Senior Backend", "Plain ol web servers.");
    assert!(!listing_matches(&l3, &kws2));
}

#[test]
fn empty_keywords_never_match() {
    let l = raw("LLM Engineer", "Build LLM agents.");
    assert!(!listing_matches(&l, &[]));
}

#[test]
fn empty_keyword_string_is_skipped() {
    let l = raw("Backend Engineer", "boring");
    let kws = vec![String::new(), "backend".to_string()];
    assert!(listing_matches(&l, &kws));
}

#[test]
fn score_listings_aggregates_across_domains() {
    let entry = SeedEntry {
        slug: "acme".into(),
        ats: AtsVendor::Greenhouse,
        domain_hint: vec![],
    };
    let listings = vec![
        raw("LLM Engineer", "machine learning systems"),
        raw("Robotics Engineer", "ROS2 and embedded"),
        raw("Marketing", "spreadsheet"),
    ];
    let domains = vec![ai_ml_domain(), robotics_domain()];
    let hit = score_listings(&entry, &listings, &domains).unwrap();
    assert_eq!(hit.slug, "acme");
    assert_eq!(hit.ats, AtsVendor::Greenhouse);
    assert_eq!(hit.matched_domains, vec!["ai_ml", "robotics_embedded"]);
    // listing 0 hits ai_ml (LLM + ml), listing 1 hits robotics
    // (embedded). One JD can match multiple keywords in one
    // domain — we count the listing once per domain.
    assert!(hit.matched_jobs >= 2, "matched_jobs={}", hit.matched_jobs);
}

#[test]
fn score_listings_returns_none_when_nothing_matches() {
    let entry = SeedEntry {
        slug: "acme".into(),
        ats: AtsVendor::Lever,
        domain_hint: vec![],
    };
    let listings = vec![raw("Recruiter", "recruiting"), raw("Sales", "sales")];
    let hit = score_listings(&entry, &listings, &[ai_ml_domain()]);
    assert!(hit.is_none());
}

#[test]
fn embedded_seed_parses_cleanly() {
    let entries = load_embedded_seed().expect("seed yaml is well-formed");
    assert!(!entries.is_empty(), "seed file should be non-empty");
    // Verify every entry has a non-empty slug + a valid ats.
    for e in &entries {
        assert!(!e.slug.is_empty(), "empty slug");
    }
    // Distribution sanity: at least one of each ats kind.
    let has_gh = entries.iter().any(|e| e.ats == AtsVendor::Greenhouse);
    let has_lev = entries.iter().any(|e| e.ats == AtsVendor::Lever);
    let has_ash = entries.iter().any(|e| e.ats == AtsVendor::Ashby);
    assert!(has_gh && has_lev && has_ash, "missing ATS coverage");
}

#[test]
fn ats_vendor_str() {
    assert_eq!(AtsVendor::Greenhouse.as_str(), "greenhouse");
    assert_eq!(AtsVendor::Lever.as_str(), "lever");
    assert_eq!(AtsVendor::Ashby.as_str(), "ashby");
}
