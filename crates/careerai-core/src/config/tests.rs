//! Tests for the layered config loader and per-section validators.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn match_config_must_include_skills_defaults_empty() {
    let yaml = "embedding_model: \"x\"\nscore_threshold: 0.5";
    let cfg: MatchConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(cfg.must_include_skills.is_empty());
}

#[test]
fn match_config_parses_must_include_skills() {
    let yaml = "embedding_model: \"x\"\nscore_threshold: 0.5\nmust_include_skills: [rust, async]";
    let cfg: MatchConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(cfg.must_include_skills, vec!["rust", "async"]);
}

#[test]
fn naukri_validated_clamps_full_coverage_quiet_hours() {
    let cfg = NaukriSubmitConfig {
        quiet_hours_utc: Some((0, 24)),
        ..Default::default()
    };
    assert_eq!(cfg.validated().quiet_hours_utc, Some((19, 1)));
}

#[test]
fn naukri_validated_clamps_out_of_range_quiet_hours() {
    let cfg = NaukriSubmitConfig {
        quiet_hours_utc: Some((25, 30)),
        ..Default::default()
    };
    assert_eq!(cfg.validated().quiet_hours_utc, Some((19, 1)));
}

#[test]
fn naukri_validated_keeps_valid_wrap_window() {
    // 19:00 UTC → 01:00 UTC is a valid wrap (matches the default).
    let cfg = NaukriSubmitConfig {
        quiet_hours_utc: Some((19, 1)),
        ..Default::default()
    };
    assert_eq!(cfg.validated().quiet_hours_utc, Some((19, 1)));
}

#[test]
fn naukri_validated_passes_through_none() {
    let cfg = NaukriSubmitConfig {
        quiet_hours_utc: None,
        ..Default::default()
    };
    assert_eq!(cfg.validated().quiet_hours_utc, None);
}

#[test]
fn embedded_defaults_parse() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = CoreConfig::load(tmp.path()).unwrap();
    assert!(!cfg.user.locations.is_empty());
    assert!(cfg.user.locations.iter().any(|l| l.contains("Remote")));
    assert!(!cfg.domains.is_empty());
    assert!(cfg.matching.score_threshold > 0.0);
}

#[test]
fn local_yaml_overrides_default() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_dir = tmp.path().join("config");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("local.yaml"),
        "match:\n  embedding_model: stub-model\n  score_threshold: 0.42\n",
    )
    .unwrap();

    let cfg = CoreConfig::load(tmp.path()).unwrap();
    assert_eq!(cfg.matching.embedding_model, "stub-model");
    assert!((cfg.matching.score_threshold - 0.42).abs() < f32::EPSILON);
}

// Env-var override is wired via the `config` crate (prefix CAREERAI,
// separator `__`) but not unit-tested here — std::env::set_var is
// unsafe in modern Rust and the workspace forbids unsafe blocks.

#[test]
fn linkedin_interactive_only_defaults_true() {
    let cfg = LinkedinSubmitConfig::default();
    assert!(
        cfg.interactive_only,
        "must default to assist-mode (true) so daemon never auto-clicks"
    );
}

#[test]
fn linkedin_interactive_only_round_trips() {
    let yaml = "interactive_only: false";
    let cfg: LinkedinSubmitConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(!cfg.interactive_only);
}

#[test]
fn naukri_submit_config_defaults_are_safe() {
    let cfg = NaukriSubmitConfig::default();
    assert!(cfg.headless);
    assert!(cfg.max_per_day <= 50, "default cap should be conservative");
    assert!(cfg.min_seconds_between >= 30);
}

#[test]
fn naukri_submit_config_round_trips() {
    let yaml = r"
headless: false
max_per_day: 5
min_seconds_between: 90
jitter_seconds: 10
action_timeout_seconds: 45
";
    let cfg: NaukriSubmitConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(!cfg.headless);
    assert_eq!(cfg.max_per_day, 5);
    assert_eq!(cfg.min_seconds_between, 90);
}
