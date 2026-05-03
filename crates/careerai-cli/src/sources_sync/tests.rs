#![allow(clippy::unwrap_used, clippy::expect_used)]

use careerai_sources::company_sync::{AtsVendor, CompanyHit, SyncReport};
use tempfile::TempDir;

use super::merge::{merge_company_list, merge_into_local_yaml};
use super::run::write_atomic;

fn hit(ats: AtsVendor, slug: &str) -> CompanyHit {
    CompanyHit {
        slug: slug.into(),
        ats,
        matched_domains: vec!["ai_ml".into()],
        matched_jobs: 1,
    }
}

#[test]
fn merge_company_list_preserves_manual_additions() {
    let report = SyncReport {
        add: vec![hit(AtsVendor::Greenhouse, "anthropic")],
        keep: vec![],
        remove: vec![],
        probe_failures: vec![],
    };
    let existing = vec!["my-private-co".to_string()];
    let merged = merge_company_list(AtsVendor::Greenhouse, &existing, &report);
    assert_eq!(merged, vec!["anthropic", "my-private-co"]);
}

#[test]
fn merge_company_list_dedupes_case_insensitively() {
    let report = SyncReport {
        add: vec![hit(AtsVendor::Greenhouse, "Anthropic")],
        keep: vec![hit(AtsVendor::Greenhouse, "ANTHROPIC")],
        remove: vec![],
        probe_failures: vec![],
    };
    let existing = vec!["anthropic".to_string()];
    let merged = merge_company_list(AtsVendor::Greenhouse, &existing, &report);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].to_ascii_lowercase(), "anthropic");
}

#[test]
fn merge_company_list_does_not_drop_remove_partition() {
    let report = SyncReport {
        add: vec![],
        keep: vec![],
        remove: vec![(AtsVendor::Greenhouse, "stale-co".into())],
        probe_failures: vec![],
    };
    let existing = vec!["stale-co".to_string(), "active-co".to_string()];
    let merged = merge_company_list(AtsVendor::Greenhouse, &existing, &report);
    assert!(merged.contains(&"stale-co".to_string()));
}

#[test]
fn yaml_round_trip_preserves_other_user_keys() {
    let tmp = TempDir::new().unwrap();
    let cfg_dir = tmp.path().join("config");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    let local = cfg_dir.join("local.yaml");
    let original = r"# user-tuned overrides
match:
  score_threshold: 0.71
notify:
  channels:
    slack:
      webhook_url_env: SLACK_HOOK
sources:
  greenhouse:
    companies:
      - my-private-co
";
    std::fs::write(&local, original).unwrap();

    let report = SyncReport {
        add: vec![hit(AtsVendor::Greenhouse, "anthropic")],
        keep: vec![],
        remove: vec![],
        probe_failures: vec![],
    };
    let merged_yaml = merge_into_local_yaml(&local, &report).unwrap();

    let v: serde_yaml::Value = serde_yaml::from_str(&merged_yaml).unwrap();
    assert_eq!(
        v.get("match")
            .and_then(|m| m.get("score_threshold"))
            .and_then(serde_yaml::Value::as_f64),
        Some(0.71)
    );
    assert_eq!(
        v.get("notify")
            .and_then(|n| n.get("channels"))
            .and_then(|c| c.get("slack"))
            .and_then(|s| s.get("webhook_url_env"))
            .and_then(serde_yaml::Value::as_str),
        Some("SLACK_HOOK")
    );
    let companies: Vec<String> = v
        .get("sources")
        .and_then(|s| s.get("greenhouse"))
        .and_then(|gh| gh.get("companies"))
        .and_then(serde_yaml::Value::as_sequence)
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .collect();
    assert!(companies.contains(&"anthropic".to_string()));
    assert!(companies.contains(&"my-private-co".to_string()));
}

#[test]
fn yaml_round_trip_creates_local_when_missing() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join("local.yaml");
    let report = SyncReport {
        add: vec![hit(AtsVendor::Lever, "lev-co")],
        keep: vec![],
        remove: vec![],
        probe_failures: vec![],
    };
    let merged = merge_into_local_yaml(&local, &report).unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&merged).unwrap();
    let lever_companies: Vec<String> = v
        .get("sources")
        .and_then(|s| s.get("lever"))
        .and_then(|l| l.get("companies"))
        .and_then(serde_yaml::Value::as_sequence)
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str().map(str::to_owned))
        .collect();
    assert_eq!(lever_companies, vec!["lev-co"]);
}

#[test]
fn empty_yaml_root_is_promoted_to_mapping() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join("local.yaml");
    std::fs::write(&local, "").unwrap();
    let report = SyncReport {
        add: vec![hit(AtsVendor::Ashby, "ashby-co")],
        keep: vec![],
        remove: vec![],
        probe_failures: vec![],
    };
    let merged = merge_into_local_yaml(&local, &report).unwrap();
    let v: serde_yaml::Value = serde_yaml::from_str(&merged).unwrap();
    assert!(v
        .get("sources")
        .and_then(|s| s.get("ashby"))
        .and_then(|a| a.get("companies"))
        .is_some());
}

#[test]
fn write_atomic_replaces_existing_file_and_cleans_up_tempfile() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("local.yaml");
    std::fs::write(&dest, "old content").unwrap();

    write_atomic(&dest, "new content").unwrap();
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "new content");

    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok().map(|x| x.file_name().to_string_lossy().to_string()))
        .collect();
    assert_eq!(
        entries,
        vec!["local.yaml".to_string()],
        "expected only local.yaml in tempdir, found: {entries:?}"
    );
}

#[test]
fn write_atomic_repeated_overwrites_succeed() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("local.yaml");
    for content in ["v1", "v2", "v3", "v4"] {
        write_atomic(&dest, content).unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), content);
    }
    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok().map(|x| x.file_name().to_string_lossy().to_string()))
        .collect();
    assert_eq!(entries, vec!["local.yaml".to_string()]);
}

#[test]
fn write_atomic_creates_missing_parent_dir() {
    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("nested").join("local.yaml");
    assert!(!dest.parent().unwrap().exists());

    write_atomic(&dest, "fresh").unwrap();
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "fresh");
}
