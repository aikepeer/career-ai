use super::*;
use careerai_core::config::{LinkedinBrowserFilters, LinkedinBrowserSourceConfig};

#[test]
fn url_encodes_spaces_as_plus() {
    assert_eq!(urlencode("AI engineer"), "AI+engineer");
}

#[test]
fn url_encodes_special_chars() {
    assert_eq!(urlencode("c++"), "c%2B%2B");
}

#[test]
fn build_url_page_zero_omits_start() {
    let cfg = LinkedinBrowserSourceConfig {
        keywords: "AI engineer".to_string(),
        location: "Worldwide".to_string(),
        ..LinkedinBrowserSourceConfig::default()
    };
    let url = build_search_url(&cfg, 0);
    assert!(url.contains("keywords=AI+engineer"), "got: {url}");
    assert!(url.contains("location=Worldwide"), "got: {url}");
    assert!(
        !url.contains("start="),
        "page 0 should omit start, got: {url}"
    );
}

#[test]
fn build_url_pagination_steps_by_25() {
    let cfg = LinkedinBrowserSourceConfig::default();
    assert!(build_search_url(&cfg, 1).contains("start=25"));
    assert!(build_search_url(&cfg, 2).contains("start=50"));
    assert!(build_search_url(&cfg, 3).contains("start=75"));
}

#[test]
fn build_url_remote_filter_emits_f_wt_2() {
    let cfg = LinkedinBrowserSourceConfig {
        filters: LinkedinBrowserFilters {
            remote: true,
            ..LinkedinBrowserFilters::default()
        },
        ..LinkedinBrowserSourceConfig::default()
    };
    assert!(build_search_url(&cfg, 0).contains("f_WT=2"));
}

#[test]
fn build_url_posted_within_days_emits_f_tpr_seconds() {
    let cfg = LinkedinBrowserSourceConfig {
        filters: LinkedinBrowserFilters {
            posted_within_days: Some(7),
            ..LinkedinBrowserFilters::default()
        },
        ..LinkedinBrowserSourceConfig::default()
    };
    assert!(build_search_url(&cfg, 0).contains("f_TPR=r604800"));
}

#[test]
fn build_url_experience_levels_map_to_codes() {
    let cfg = LinkedinBrowserSourceConfig {
        filters: LinkedinBrowserFilters {
            experience_level: vec!["mid".to_string(), "senior".to_string()],
            ..LinkedinBrowserFilters::default()
        },
        ..LinkedinBrowserSourceConfig::default()
    };
    let url = build_search_url(&cfg, 0);
    assert!(url.contains("f_E="), "got: {url}");
    assert!(url.contains('4') && url.contains('5'), "got: {url}");
}

#[test]
fn parse_empty_html_returns_empty() {
    let listings = parse_search_html("<html><body></body></html>");
    assert!(listings.is_empty());
}

#[test]
fn extract_id_from_urn() {
    let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item" data-entity-urn="urn:li:jobPosting:9876543"><a class="base-card__full-link" href="/jobs/view/9876543/">Title</a></li></ul>"#;
    let listings = parse_search_html(html);
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].external_id, "9876543");
}

#[test]
fn extract_id_from_url_when_urn_missing() {
    let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item"><a class="base-card__full-link" href="/jobs/view/1234567/?refId=abc">Job Title</a></li></ul>"#;
    let listings = parse_search_html(html);
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].external_id, "1234567");
    assert_eq!(listings[0].source, "linkedin");
}

#[test]
fn malformed_card_with_no_title_skipped_not_panicked() {
    let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item" data-entity-urn="urn:li:jobPosting:1"></li></ul>"#;
    let listings = parse_search_html(html);
    assert!(listings.is_empty());
}

#[test]
fn missing_company_yields_empty_string_not_panic() {
    let html = r#"<ul class="jobs-search__results-list"><li class="jobs-search-results__list-item" data-entity-urn="urn:li:jobPosting:55"><a class="base-card__full-link" href="/jobs/view/55/">A Role</a></li></ul>"#;
    let listings = parse_search_html(html);
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].title, "A Role");
    assert_eq!(listings[0].company, "");
}
