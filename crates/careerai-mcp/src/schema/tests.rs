#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn discover_args_default_when_empty() {
    let args: DiscoverArgs = serde_json::from_str("{}").unwrap();
    assert!(args.sources.is_empty());
}

#[test]
fn discover_args_accepts_sources_array() {
    let args: DiscoverArgs =
        serde_json::from_str(r#"{"sources":["greenhouse","lever"]}"#).unwrap();
    assert_eq!(args.sources, vec!["greenhouse", "lever"]);
}

#[test]
fn shortlist_args_default_limit_and_min_score() {
    let args: ShortlistArgs = serde_json::from_str("{}").unwrap();
    assert!(args.limit.is_none());
    assert!(args.min_score.is_none());
}

#[test]
fn shortlist_args_accepts_partial() {
    let args: ShortlistArgs = serde_json::from_str(r#"{"limit":10}"#).unwrap();
    assert_eq!(args.limit, Some(10));
    assert!(args.min_score.is_none());
}

#[test]
fn apply_args_dry_run_defaults_true() {
    let args: ApplyArgs = serde_json::from_str(r#"{"application_id":"abc"}"#).unwrap();
    assert_eq!(args.application_id, "abc");
    assert!(args.dry_run, "dry_run must default to true (safety)");
    assert!(args.confirm.is_none());
}

#[test]
fn apply_args_carries_confirm_token() {
    let args: ApplyArgs = serde_json::from_str(
        r#"{"application_id":"abc","dry_run":false,"confirm":"I_UNDERSTAND_TOS_RISK"}"#,
    )
    .unwrap();
    assert!(!args.dry_run);
    assert_eq!(args.confirm.as_deref(), Some("I_UNDERSTAND_TOS_RISK"));
}

#[test]
fn digest_args_required() {
    let err = serde_json::from_str::<DigestArgs>("{}");
    assert!(err.is_err(), "since must be required");
    let ok: DigestArgs = serde_json::from_str(r#"{"since":"1d"}"#).unwrap();
    assert_eq!(ok.since, "1d");
}

#[test]
fn tailor_and_render_and_inspect_args_require_id() {
    assert!(serde_json::from_str::<TailorArgs>("{}").is_err());
    assert!(serde_json::from_str::<RenderArgs>("{}").is_err());
    assert!(serde_json::from_str::<InspectArgs>("{}").is_err());
}

#[test]
fn discover_result_serializes_snake_case() {
    let r = DiscoverResult {
        fetched: 12,
        new_rows: 3,
        duplicates: 9,
        errors: 0,
    };
    let v: serde_json::Value = serde_json::to_value(&r).unwrap();
    assert_eq!(v["new_rows"], 3);
    assert_eq!(v["fetched"], 12);
}

#[test]
fn apply_result_omits_none_fields() {
    let r = ApplyResult {
        application_id: "id".into(),
        source: "greenhouse".into(),
        outcome: "Skipped".into(),
        would_submit: None,
        note: Some("source disabled".into()),
    };
    let s = serde_json::to_string(&r).unwrap();
    assert!(
        !s.contains("would_submit"),
        "would_submit should be skipped: {s}"
    );
    assert!(s.contains("note"));
}

#[test]
fn compact_listing_excludes_bulky_fields() {
    let c = CompactListing {
        listing_id: "L-1".into(),
        title: "Staff ML Engineer".into(),
        company: "ACME".into(),
        url: "https://example.test/j/1".into(),
        source: "greenhouse".into(),
        score: Some(0.87),
        location: None,
    };
    let s = serde_json::to_string(&c).unwrap();
    assert!(
        !s.contains("description"),
        "description must not appear in compact resource body: {s}"
    );
    assert!(
        !s.contains("raw_json"),
        "raw_json must not appear in compact resource body: {s}"
    );
    assert!(
        !s.contains("location"),
        "None location must be skipped: {s}"
    );
    for key in ["listing_id", "title", "company", "url", "source", "score"] {
        assert!(s.contains(key), "expected key {key} in {s}");
    }
}
