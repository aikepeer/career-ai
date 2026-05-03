#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use careerai_core::config::{McpSourceConfig, McpTransportConfig};

use super::discover::expand_env;
use super::parser::{derive_external_id, parse_listings};
use super::source::McpJobsSource;
use crate::base::Source;

#[test]
fn parses_rapidapi_shape() {
    // Mimic RapidAPI's linkedin-jobs response: array of objects
    // with `job_title` / `company_name` / `apply_url` / `posted_at`.
    let body = r#"[
        {
            "job_title": "AI Engineer",
            "company_name": "Acme",
            "location": "Remote",
            "apply_url": "https://example.com/jobs/1",
            "summary": "Build LLM apps",
            "job_id": "rapid-1",
            "posted_at": "2026-04-26T10:00:00Z"
        }
    ]"#;
    let listings = parse_listings(body, "linkedin-jobs-mcp").expect("parse");
    assert_eq!(listings.len(), 1);
    let l = &listings[0];
    assert_eq!(l.title, "AI Engineer");
    assert_eq!(l.company, "Acme");
    assert_eq!(l.url, "https://example.com/jobs/1");
    assert_eq!(l.location.as_deref(), Some("Remote"));
    assert_eq!(l.description, "Build LLM apps");
    assert_eq!(l.external_id, "rapid-1");
    assert_eq!(l.source, "linkedin-jobs-mcp");
}

#[test]
fn parses_mcp_linkedin_shape() {
    // mcp-linkedin (Adhikary97 et al) returns objects under a
    // `jobs` envelope and uses `title` / `company` / `link`.
    let body = r#"{
        "jobs": [
            {
                "title": "ML Researcher",
                "company": "Beta",
                "place": "Bangalore-remote",
                "link": "https://example.com/jobs/2",
                "description": "<p>Train models.</p>",
                "id": "ml-2"
            },
            {
                "title": "Robotics Engineer",
                "employer": "Gamma",
                "city": "Delhi NCR",
                "url": "https://example.com/jobs/3",
                "snippet": "ROS2",
                "external_id": "rb-3"
            }
        ]
    }"#;
    let listings = parse_listings(body, "mcp-linkedin").expect("parse");
    assert_eq!(listings.len(), 2);
    assert_eq!(listings[0].title, "ML Researcher");
    assert_eq!(listings[0].company, "Beta");
    assert_eq!(listings[0].location.as_deref(), Some("Bangalore-remote"));
    assert_eq!(listings[1].company, "Gamma");
    assert_eq!(listings[1].location.as_deref(), Some("Delhi NCR"));
    assert_eq!(listings[1].url, "https://example.com/jobs/3");
    assert_eq!(listings[1].description, "ROS2");
}

#[test]
fn parses_generic_shape_with_results_envelope() {
    let body = r#"{
        "results": [
            {
                "position": "LLM Platform Engineer",
                "employer": "Delta",
                "url": "https://example.com/jobs/4"
            }
        ]
    }"#;
    let listings = parse_listings(body, "generic-mcp").expect("parse");
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].title, "LLM Platform Engineer");
    assert_eq!(listings[0].company, "Delta");
    // No id field anywhere → SHA256-derived external_id.
    assert_eq!(listings[0].external_id.len(), 16);
    assert!(listings[0]
        .external_id
        .chars()
        .all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn missing_keys_yield_none_or_empty_no_panic() {
    let body = r#"[
        { "title": "Only a title" },
        { "url": "https://example.com/jobs/empty", "company": "Eps" },
        {}
    ]"#;
    let listings = parse_listings(body, "test").expect("parse");
    // The empty object has no usable fields; it should be skipped.
    assert_eq!(listings.len(), 2);
    assert_eq!(listings[0].title, "Only a title");
    assert_eq!(listings[0].company, "");
    assert_eq!(listings[0].location, None);
    assert_eq!(listings[1].url, "https://example.com/jobs/empty");
}

#[test]
fn external_id_is_stable_for_same_inputs() {
    let a = derive_external_id("src", "u", "t", "c");
    let b = derive_external_id("src", "u", "t", "c");
    assert_eq!(a, b);
    let c = derive_external_id("src", "u", "t", "different");
    assert_ne!(a, c);
}

#[test]
fn external_id_differs_across_sources_for_same_url() {
    // Two MCP sources can legitimately surface the same job (same
    // URL, title, company, no upstream id). The downstream upsert
    // key is `(source, external_id)`; if the SHA didn't include the
    // source name, both rows would collapse into one and we'd
    // silently drop the second source's listing.
    let a = derive_external_id("source-one", "u", "t", "c");
    let b = derive_external_id("source-two", "u", "t", "c");
    assert_ne!(a, b, "external_id must vary by source_name");
}

#[test]
fn description_html_is_stripped_to_plain_text() {
    // mcp-linkedin-style payload: HTML inside `description`. The
    // matcher embeddings expect plain text; assert we strip tags
    // the same way greenhouse / lever / remoteok do.
    let body = r#"[{
        "title": "X",
        "company": "Y",
        "url": "https://example.com/x",
        "description": "<p>Build <strong>LLM</strong> apps.</p>"
    }]"#;
    let listings = parse_listings(body, "src").expect("parse");
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].description, "Build LLM apps.");
}

#[test]
fn intern_source_state_reuses_static_name_and_limiter() {
    // Two `McpJobsSource` constructed with the same `name` must
    // share the same `&'static str` (so the cache is hit, no
    // per-tick leak) and the same rate-limiter `Arc` (so the
    // token-bucket state survives `build_sources()` rebuild).
    let cfg = |rpm: u32| McpSourceConfig {
        name: "intern-test-shared".into(),
        enabled: true,
        submit_enabled: false,
        cron: None,
        rate_per_minute: rpm,
        mcp: McpTransportConfig::default(),
    };
    let a = McpJobsSource::new(cfg(60));
    let b = McpJobsSource::new(cfg(60));
    // Same `&'static str` (pointer identity, not just string equality).
    assert!(
        std::ptr::eq(a.name(), b.name()),
        "name_static must be interned across constructions"
    );
    // Same limiter `Arc`.
    let arc_a = a.rate_limiter.as_ref().expect("limiter set");
    let arc_b = b.rate_limiter.as_ref().expect("limiter set");
    assert!(
        Arc::ptr_eq(arc_a, arc_b),
        "rate_limiter Arc must be shared across constructions"
    );
}

#[test]
fn parse_listings_propagates_error_instead_of_empty() {
    // Regression: previously `run_call` swallowed parse errors and
    // returned an empty Vec, hiding upstream schema breakage from
    // the operator. The mapper itself returns Result; assert that
    // a malformed body surfaces the error rather than `Ok(vec![])`.
    let err = parse_listings("not json", "x").unwrap_err();
    assert!(err.contains("invalid JSON"), "got: {err}");
    let err = parse_listings(r#"{"unexpected": true}"#, "x").unwrap_err();
    assert!(err.contains("expected JSON array"), "got: {err}");
}

#[test]
fn rejects_non_array_non_envelope() {
    let body = r#"{ "error": "no jobs found" }"#;
    let err = parse_listings(body, "x").unwrap_err();
    assert!(err.contains("expected JSON array"));
}

#[test]
fn invalid_json_returns_err_not_panic() {
    let body = "not json";
    let err = parse_listings(body, "x").unwrap_err();
    assert!(err.contains("invalid JSON"));
}

#[test]
fn expand_env_substitutes_known_var() {
    // SAFETY: setting an env var in a test isn't safe under
    // parallelism, but this test only reads its own write and
    // doesn't observe other tests' state.
    std::env::set_var("CAREERAI_TEST_FAKE_KEY", "secret-123");
    let out = expand_env("Bearer ${CAREERAI_TEST_FAKE_KEY}");
    assert_eq!(out, "Bearer secret-123");
    std::env::remove_var("CAREERAI_TEST_FAKE_KEY");
}

#[test]
fn expand_env_missing_var_collapses_to_empty() {
    let out = expand_env("X=${CAREERAI_DEFINITELY_UNSET_VAR_XYZ};");
    assert_eq!(out, "X=;");
}

#[test]
fn expand_env_passthrough_when_no_placeholder() {
    let out = expand_env("plain string");
    assert_eq!(out, "plain string");
}
