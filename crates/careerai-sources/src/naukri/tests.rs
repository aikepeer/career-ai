#![allow(clippy::unwrap_used, clippy::expect_used)]

use wiremock::matchers::{header, header_regex, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::source::NaukriSource;
use crate::base::{Source, SourceError};

fn sample_body() -> serde_json::Value {
    serde_json::json!({
        "jobDetails": [
            {
                "jobId": "280125500001",
                "title": "Senior ML Engineer",
                "companyName": "Acme India",
                "placeholders": [
                    {"type": "experience", "label": "5-10 Yrs"},
                    {"type": "salary", "label": "Not disclosed"},
                    {"type": "location", "label": "Bangalore, Delhi / NCR"}
                ],
                "jdURL": "/job-listings-senior-ml-engineer-acme-india-280125500001",
                "jobDescription": "Build LLM systems on Kubernetes.",
                "tagsAndSkills": "Python,ML,LLM,Kubernetes",
                "createdDate": 1_712_345_678_000_i64
            },
            {
                "jobId": "280125500002",
                "title": "Robotics Perception Engineer",
                "companyName": "Beta Robotics",
                "placeholders": [
                    {"type": "location", "label": "Delhi / NCR"}
                ],
                "jdURL": "/job-listings-robotics-perception-engineer-beta-robotics-280125500002",
                "jobDescription": "ROS2, SLAM, perception.",
                "tagsAndSkills": "ROS2,SLAM",
                "createdDate": 1_712_345_679_000_i64
            }
        ],
        "totalJobsDisplayed": 2
    })
}

#[tokio::test]
async fn discovers_listings_against_mock() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jobapi/v3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_body()))
        .mount(&server)
        .await;

    let src = NaukriSource::new().with_base_url(server.uri());
    let listings = src.discover().await.unwrap();
    assert_eq!(listings.len(), 2);

    let first = &listings[0];
    assert_eq!(first.source, "naukri");
    assert_eq!(first.external_id, "280125500001");
    assert_eq!(first.title, "Senior ML Engineer");
    assert_eq!(first.company, "Acme India");
    assert_eq!(first.location.as_deref(), Some("Bangalore, Delhi / NCR"));
    assert_eq!(
        first.url,
        "https://www.naukri.com/job-listings-senior-ml-engineer-acme-india-280125500001"
    );

    let second = &listings[1];
    assert_eq!(second.external_id, "280125500002");
    assert_eq!(second.location.as_deref(), Some("Delhi / NCR"));
}

#[tokio::test]
async fn strips_html_from_job_description() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "jobDetails": [{
            "jobId": "1",
            "title": "X",
            "companyName": "Y",
            "placeholders": [],
            "jdURL": "/x",
            "jobDescription": "<p>Build <strong>LLM</strong> apps.</p>"
        }],
        "totalJobsDisplayed": 1
    });
    Mock::given(method("GET"))
        .and(path("/jobapi/v3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let src = NaukriSource::new().with_base_url(server.uri());
    let listings = src.discover().await.unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].description, "Build LLM apps.");
}

#[tokio::test]
async fn sends_required_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jobapi/v3/search"))
        .and(header("appid", "109"))
        .and(header("systemid", "109"))
        .and(header_regex("user-agent", "careerai"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_body()))
        .mount(&server)
        .await;

    let src = NaukriSource::new().with_base_url(server.uri());
    let listings = src.discover().await.unwrap();
    assert_eq!(listings.len(), 2);
}

#[tokio::test]
async fn http_error_surfaces_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jobapi/v3/search"))
        .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
        .mount(&server)
        .await;

    let src = NaukriSource::new().with_base_url(server.uri());
    let err = src.discover().await.unwrap_err();
    assert!(matches!(err, SourceError::HttpStatus { status: 403, .. }));
}

#[tokio::test]
async fn passes_keywords_and_location_as_query_params() {
    // Regression test: if someone interpolates user input into the URL
    // path instead of using `.query(...)`, this test fires because
    // wiremock's `query_param` matcher only matches URL-encoded params.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jobapi/v3/search"))
        .and(query_param("keywords", "machine learning,llm"))
        .and(query_param("location", "Delhi / NCR"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sample_body()))
        .mount(&server)
        .await;

    let src = NaukriSource::new()
        .with_base_url(server.uri())
        .with_keywords(vec!["machine learning".to_string(), "llm".to_string()])
        .with_location("Delhi / NCR".to_string());
    let listings = src.discover().await.unwrap();
    assert_eq!(listings.len(), 2);
}

#[test]
fn absolutize_jd_url_rejects_off_origin_absolute() {
    // Import the function under test.
    use super::source::absolutize_jd_url;

    // Site-relative path stays site-relative.
    assert_eq!(
        absolutize_jd_url("/foo-bar-1234"),
        Some("https://www.naukri.com/foo-bar-1234".to_owned())
    );
    // Same-origin absolute URL passes through.
    assert_eq!(
        absolutize_jd_url("https://www.naukri.com/foo-1234"),
        Some("https://www.naukri.com/foo-1234".to_owned())
    );
    // Off-origin absolute URL → rejected.
    assert_eq!(absolutize_jd_url("https://evil.com/phish"), None);
    // Protocol-relative → rejected.
    assert_eq!(absolutize_jd_url("//evil.com/foo"), None);
    // Empty / whitespace → rejected.
    assert_eq!(absolutize_jd_url(""), None);
    assert_eq!(absolutize_jd_url("   "), None);
    // Bare token (no leading slash) → rejected.
    assert_eq!(absolutize_jd_url("foo"), None);
}

#[tokio::test]
async fn skips_listings_with_empty_or_missing_job_id() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "jobDetails": [
            {
                "jobId": "real-1",
                "title": "Engineer",
                "companyName": "Co",
                "placeholders": [],
                "jdURL": "/jd/1",
                "jobDescription": "x"
            },
            {
                "jobId": "",
                "title": "Empty ID",
                "companyName": "Co",
                "placeholders": [],
                "jdURL": "/jd/2",
                "jobDescription": "x"
            },
            {
                // jobId omitted entirely
                "title": "Missing ID",
                "companyName": "Co",
                "placeholders": [],
                "jdURL": "/jd/3",
                "jobDescription": "x"
            }
        ]
    });
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let src = NaukriSource::new().with_base_url(server.uri());
    let listings = src.discover().await.unwrap();
    // Only the row with a real jobId survives.
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].external_id, "real-1");
}

#[tokio::test]
async fn accepts_numeric_job_id_in_response() {
    let server = MockServer::start().await;
    let body = serde_json::json!({
        "jobDetails": [
            {
                // Number, not string — must round-trip via the
                // string-or-number deserializer.
                "jobId": 280_125_500_001_i64,
                "title": "ML Engineer",
                "companyName": "Co",
                "placeholders": [],
                "jdURL": "/jd/1",
                "jobDescription": "x"
            }
        ]
    });
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let src = NaukriSource::new().with_base_url(server.uri());
    let listings = src.discover().await.unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].external_id, "280125500001");
}
