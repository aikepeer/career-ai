#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{ai_ml_domain, cfg_with_domains, robotics_domain};

use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::super::partition::sync_with_base_urls;
use super::super::probe::{probe_one, BaseUrls, ProbeOutcome};
use super::super::seed::{AtsVendor, SeedEntry};

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn sync_partitions_add_keep_remove() {
    let server = MockServer::start().await;

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

    let mut cfg = cfg_with_domains(vec![ai_ml_domain(), robotics_domain()]);
    cfg.sources.greenhouse.companies =
        vec!["pre-existing".into(), "stale".into(), "manual-add".into()];

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

    let keep_slugs: Vec<_> = report.keep.iter().map(|h| h.slug.clone()).collect();
    assert!(
        keep_slugs.contains(&"pre-existing".to_string()),
        "keep: {keep_slugs:?}"
    );

    let remove_slugs: Vec<_> = report.remove.iter().map(|(_, s)| s.clone()).collect();
    assert!(
        remove_slugs.contains(&"stale".to_string()),
        "remove: {remove_slugs:?}"
    );
    assert!(
        !remove_slugs.contains(&"manual-add".to_string()),
        "remove: {remove_slugs:?}"
    );

    assert!(
        report.probe_failures.is_empty(),
        "failures: {:?}",
        report.probe_failures
    );
}

#[tokio::test]
async fn sync_records_probe_failures_without_aborting() {
    let server = MockServer::start().await;
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

#[tokio::test]
async fn probe_failure_does_not_propose_removal_of_configured_slug() {
    let server = MockServer::start().await;
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

    let failure_slugs: Vec<_> = report
        .probe_failures
        .iter()
        .map(|(s, _)| s.clone())
        .collect();
    assert!(
        failure_slugs.contains(&"flaky".to_string()),
        "expected probe failure for `flaky`; got: {failure_slugs:?}"
    );

    let remove_slugs: Vec<_> = report.remove.iter().map(|(_, s)| s.clone()).collect();
    assert!(
        !remove_slugs.contains(&"flaky".to_string()),
        "probe-failed slug should not be marked for removal; got: {remove_slugs:?}"
    );
}

#[tokio::test]
async fn probe_timeout_with_large_delay() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex(r"^/v1/boards/slow/jobs"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30)))
        .mount(&server)
        .await;

    let cfg = cfg_with_domains(vec![ai_ml_domain()]);
    let entry = SeedEntry {
        slug: "slow".into(),
        ats: AtsVendor::Greenhouse,
        domain_hint: vec![],
    };
    let bases = BaseUrls {
        greenhouse: server.uri(),
        lever: server.uri(),
        ashby: server.uri(),
    };
    let outcome = probe_one(entry, bases, cfg.domains, std::time::Duration::from_secs(1)).await;
    match outcome {
        ProbeOutcome::Failure { slug, reason } => {
            assert_eq!(slug, "slow");
            assert!(reason.contains("timeout"), "reason: {reason}");
        }
        other => panic!("expected timeout Failure, got {other:?}"),
    }
}
