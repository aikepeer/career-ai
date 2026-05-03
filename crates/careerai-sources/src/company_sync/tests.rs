#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_core::config::{CoreConfig, Domain};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::partition::sync_with_base_urls;
use super::probe::{listing_matches, score_listings, BaseUrls};
use super::seed::{load_embedded_seed, AtsVendor, SeedEntry};

fn cfg_with_domains(domains: Vec<Domain>) -> CoreConfig {
    let mut cfg: CoreConfig =
        serde_yaml::from_str(careerai_core::config::EMBEDDED_DEFAULTS).unwrap();
    cfg.domains = domains;
    cfg.sources.greenhouse.companies.clear();
    cfg.sources.lever.companies.clear();
    cfg.sources.ashby.companies.clear();
    cfg
}

fn ai_ml_domain() -> Domain {
    Domain {
        name: "ai_ml".into(),
        keywords_any: vec!["machine learning".into(), "llm".into()],
    }
}

fn robotics_domain() -> Domain {
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

#[tokio::test]
#[allow(clippy::too_many_lines)] // exercises every partition path
async fn sync_partitions_add_keep_remove() {
    let server = MockServer::start().await;

    // Greenhouse: `acme` has an LLM job → should land in `add`.
    // Greenhouse: `pre-existing` (already configured) has an LLM
    // job → should land in `keep`. Greenhouse: `stale` (already
    // configured but seeded) has no matching job → should land
    // in `remove`. Greenhouse: `boring` (seeded) has no
    // matching job → should be silently ignored (a Miss, not a
    // remove because not configured).
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/boards/acme/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [{
                "id": 1,
                "title": "Senior LLM Engineer",
                "absolute_url": "https://greenhouse.io/acme/1",
                "content": "<p>Build agents.</p>"
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/boards/pre-existing/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [{
                "id": 2,
                "title": "ML Researcher",
                "absolute_url": "https://greenhouse.io/pre-existing/2",
                "content": "<p>machine learning</p>"
            }]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/boards/(stale|boring)/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [{
                "id": 3,
                "title": "Recruiter",
                "absolute_url": "https://greenhouse.io/stale/3",
                "content": "<p>hiring</p>"
            }]
        })))
        .mount(&server)
        .await;

    // Lever: `lev-co` has matching JD → add.
    Mock::given(method("GET"))
        .and(path_regex(r"^/v0/postings/lev-co"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "id": "p1",
                "text": "Robotics Engineer",
                "descriptionPlain": "ROS2 and embedded systems.",
                "categories": {},
                "hostedUrl": "https://lever.co/lev-co/p1"
            }])),
        )
        .mount(&server)
        .await;

    // Ashby: `ash-co` has matching JD → add.
    Mock::given(method("GET"))
        .and(path_regex(r"^/posting-api/job-board/ash-co"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [{
                "id": "a1",
                "title": "Applied ML",
                "location": "Remote",
                "jobUrl": "https://ashby.co/ash-co/a1",
                "descriptionHtml": "<p>We use machine learning end to end.</p>"
            }]
        })))
        .mount(&server)
        .await;

    // Build cfg with `pre-existing` and `stale` already configured.
    let mut cfg = cfg_with_domains(vec![ai_ml_domain(), robotics_domain()]);
    cfg.sources.greenhouse.companies =
        vec!["pre-existing".into(), "stale".into(), "manual-add".into()];

    // Seed slugs (note `stale` is in the seed, `manual-add` is
    // not — so `manual-add` should NOT show up in `remove`).
    let seed = vec![
        SeedEntry {
            slug: "acme".into(),
            ats: AtsVendor::Greenhouse,
            domain_hint: vec![],
        },
        SeedEntry {
            slug: "pre-existing".into(),
            ats: AtsVendor::Greenhouse,
            domain_hint: vec![],
        },
        SeedEntry {
            slug: "stale".into(),
            ats: AtsVendor::Greenhouse,
            domain_hint: vec![],
        },
        SeedEntry {
            slug: "boring".into(),
            ats: AtsVendor::Greenhouse,
            domain_hint: vec![],
        },
        SeedEntry {
            slug: "lev-co".into(),
            ats: AtsVendor::Lever,
            domain_hint: vec![],
        },
        SeedEntry {
            slug: "ash-co".into(),
            ats: AtsVendor::Ashby,
            domain_hint: vec![],
        },
    ];

    let bases = BaseUrls {
        greenhouse: server.uri(),
        lever: server.uri(),
        ashby: server.uri(),
    };
    let report = sync_with_base_urls(&cfg, &seed, &bases).await.unwrap();

    // add: acme (gh), lev-co (lever), ash-co (ashby)
    let add_slugs: Vec<_> = report.add.iter().map(|h| h.slug.clone()).collect();
    assert!(
        add_slugs.contains(&"acme".to_string()),
        "add: {add_slugs:?}"
    );
    assert!(
        add_slugs.contains(&"lev-co".to_string()),
        "add: {add_slugs:?}"
    );
    assert!(
        add_slugs.contains(&"ash-co".to_string()),
        "add: {add_slugs:?}"
    );

    // keep: pre-existing
    let keep_slugs: Vec<_> = report.keep.iter().map(|h| h.slug.clone()).collect();
    assert!(
        keep_slugs.contains(&"pre-existing".to_string()),
        "keep: {keep_slugs:?}"
    );

    // remove: stale (in seed, configured, but missed). manual-add
    // (configured but not in seed) must NOT appear.
    let remove_slugs: Vec<_> = report.remove.iter().map(|(_, s)| s.clone()).collect();
    assert!(
        remove_slugs.contains(&"stale".to_string()),
        "remove: {remove_slugs:?}"
    );
    assert!(
        !remove_slugs.contains(&"manual-add".to_string()),
        "remove: {remove_slugs:?}"
    );

    // probe_failures should be empty (all probes returned 200).
    assert!(
        report.probe_failures.is_empty(),
        "failures: {:?}",
        report.probe_failures
    );
}

#[tokio::test]
async fn sync_records_probe_failures_without_aborting() {
    let server = MockServer::start().await;
    // good: returns a matching JD
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/boards/good/jobs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [{
                "id": 1,
                "title": "ML Engineer",
                "absolute_url": "https://greenhouse.io/good/1",
                "content": "machine learning"
            }]
        })))
        .mount(&server)
        .await;
    // bad: returns 500. Probe should soft-fail.
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/boards/bad/jobs"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let cfg = cfg_with_domains(vec![ai_ml_domain()]);
    let seed = vec![
        SeedEntry {
            slug: "good".into(),
            ats: AtsVendor::Greenhouse,
            domain_hint: vec![],
        },
        SeedEntry {
            slug: "bad".into(),
            ats: AtsVendor::Greenhouse,
            domain_hint: vec![],
        },
    ];
    let bases = BaseUrls {
        greenhouse: server.uri(),
        lever: server.uri(),
        ashby: server.uri(),
    };
    let report = sync_with_base_urls(&cfg, &seed, &bases).await.unwrap();
    assert_eq!(report.add.len(), 1);
    assert_eq!(report.add[0].slug, "good");
    assert_eq!(report.probe_failures.len(), 1);
    assert_eq!(report.probe_failures[0].0, "bad");
}

// Regression: a slug that is configured AND in the seed list AND
// probe-failed (timeout, 5xx, parse error, etc.) must NOT be
// surfaced as "consider removing". A failed probe is "we have no
// signal", not "we confirmed zero matches" — recommending removal
// would silently drop user-configured companies on a flaky
// network. See PR feedback on `careerai sources sync` output that
// marked anthropic/stripe for removal after a 10s timeout.
#[tokio::test]
async fn probe_failure_does_not_propose_removal_of_configured_slug() {
    let server = MockServer::start().await;
    // 500 → ProbeOutcome::Failure on the only configured slug.
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/boards/flaky/jobs"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let mut cfg = cfg_with_domains(vec![ai_ml_domain()]);
    cfg.sources.greenhouse.companies = vec!["flaky".into()];

    let seed = vec![SeedEntry {
        slug: "flaky".into(),
        ats: AtsVendor::Greenhouse,
        domain_hint: vec![],
    }];
    let bases = BaseUrls {
        greenhouse: server.uri(),
        lever: server.uri(),
        ashby: server.uri(),
    };
    let report = sync_with_base_urls(&cfg, &seed, &bases).await.unwrap();

    // The probe must be recorded as a failure.
    let failure_slugs: Vec<_> = report
        .probe_failures
        .iter()
        .map(|(s, _)| s.clone())
        .collect();
    assert!(
        failure_slugs.contains(&"flaky".to_string()),
        "expected probe failure for `flaky`; got: {failure_slugs:?}"
    );

    // And it must NOT be in the remove list — we have no evidence
    // it has zero matches, only that we couldn't reach it.
    let remove_slugs: Vec<_> = report.remove.iter().map(|(_, s)| s.clone()).collect();
    assert!(
        !remove_slugs.contains(&"flaky".to_string()),
        "probe-failed slug should not be marked for removal; got: {remove_slugs:?}"
    );
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
