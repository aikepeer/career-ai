//! `careerai sources sync` — probes the seeded ATS list for companies
//! whose currently-open jobs match the user's `domains:` keywords and
//! either previews the diff or merges it into `config/local.yaml`.
//!
//! Default behavior is preview. `--apply` is required to mutate the
//! file, and the merge preserves every OTHER user key (we round-trip
//! through `serde_yaml::Mapping`, only touching `sources.<ats>.companies`).

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use careerai_core::config::CoreConfig;
use careerai_sources::company_sync::{
    load_embedded_seed, sync as run_sync, AtsVendor, CompanyHit, SyncReport,
};

/// Print + optionally apply the sync.
///
/// `apply == false` is the default — prints a preview only.
/// `apply == true` writes `config/local.yaml` (creates it if missing)
/// with the new lists merged in. Existing keys outside
/// `sources.<ats>.companies` are preserved verbatim.
pub async fn run(cwd: &Path, apply: bool) -> Result<()> {
    let cfg = CoreConfig::load(cwd).context("load config")?;
    let seed = load_embedded_seed().context("parse embedded seed_companies.yaml")?;
    let report = run_sync(&cfg, &seed)
        .await
        .map_err(|e| anyhow!("sync failed: {e}"))?;

    print_report(&report, apply);

    if !apply {
        return Ok(());
    }
    let local_path = cwd.join("config").join("local.yaml");
    let merged = merge_into_local_yaml(&local_path, &report)?;
    std::fs::write(&local_path, merged)
        .with_context(|| format!("write {}", local_path.display()))?;
    println!();
    println!("wrote {}", local_path.display());
    Ok(())
}

fn print_report(report: &SyncReport, apply: bool) {
    let header = if apply {
        "sources sync (applying):"
    } else {
        "sources sync (preview — pass --apply to write):"
    };
    println!("{header}");

    for ats in [AtsVendor::Greenhouse, AtsVendor::Lever, AtsVendor::Ashby] {
        let adds: Vec<&CompanyHit> = report.add.iter().filter(|h| h.ats == ats).collect();
        let keeps: Vec<&CompanyHit> = report.keep.iter().filter(|h| h.ats == ats).collect();
        let removes: Vec<&String> = report
            .remove
            .iter()
            .filter_map(|(a, s)| if *a == ats { Some(s) } else { None })
            .collect();
        if adds.is_empty() && keeps.is_empty() && removes.is_empty() {
            continue;
        }
        println!("  {}:", ats.as_str());
        for hit in &adds {
            println!(
                "    + {}  ({} matches: {})",
                hit.slug,
                hit.matched_jobs,
                hit.matched_domains.join(" + "),
            );
        }
        for hit in &keeps {
            println!(
                "    keep {}  ({} matches: {})",
                hit.slug,
                hit.matched_jobs,
                hit.matched_domains.join(" + "),
            );
        }
        for slug in removes {
            println!("    - {slug}    (no current matches; consider removing)");
        }
    }

    if !report.probe_failures.is_empty() {
        println!();
        println!("  probe failures (soft-failed, sync continued):");
        for (slug, reason) in &report.probe_failures {
            println!("    ! {slug}: {reason}");
        }
    }

    if !apply {
        println!();
        println!("  next: rerun with --apply to write changes to config/local.yaml");
    }
}

/// Merge the sync diff into `config/local.yaml` and return the new YAML.
///
/// Behavior:
/// - If the file exists, load it as a `serde_yaml::Mapping`. Otherwise
///   start with an empty mapping (we'll create the file fresh).
/// - For each ATS, build the union of `keep + add` slugs PLUS any slug
///   the user already had under `sources.<ats>.companies` that is NOT
///   in the seed list (manual additions are preserved).
/// - Slugs in the `remove` partition are left in the file — `sources
///   sync` only suggests removals; users hand-prune.
/// - Round-trip is `serde_yaml::Value`-based so unrelated keys
///   (`match.score_threshold`, `notify.channels`, etc.) survive.
pub(crate) fn merge_into_local_yaml(local_path: &Path, report: &SyncReport) -> Result<String> {
    let existing = if local_path.exists() {
        let raw = std::fs::read_to_string(local_path)
            .with_context(|| format!("read {}", local_path.display()))?;
        if raw.trim().is_empty() {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        } else {
            serde_yaml::from_str::<serde_yaml::Value>(&raw)
                .with_context(|| format!("parse {}", local_path.display()))?
        }
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };
    let merged = merge_into_value(existing, report);
    serde_yaml::to_string(&merged).context("serialize merged yaml")
}

/// Merge the sync diff into a parsed YAML root and return the
/// updated tree. Always succeeds — non-mapping inputs are silently
/// promoted, so the caller never has to handle a structural error
/// here. (Round-trip failures surface in [`merge_into_local_yaml`]
/// from the `to_string` step.)
fn merge_into_value(mut root: serde_yaml::Value, report: &SyncReport) -> serde_yaml::Value {
    // Promote scalar/null roots to a mapping so we can index into it.
    if !root.is_mapping() {
        root = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    let Some(root_map) = root.as_mapping_mut() else {
        // Unreachable: `is_mapping` was just normalized to true.
        return root;
    };

    let sources_key = serde_yaml::Value::String("sources".into());
    let sources_entry = root_map
        .entry(sources_key)
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    if !sources_entry.is_mapping() {
        *sources_entry = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    let Some(sources) = sources_entry.as_mapping_mut() else {
        return root;
    };

    for ats in [AtsVendor::Greenhouse, AtsVendor::Lever, AtsVendor::Ashby] {
        // Pull the existing companies list (if any); we'll merge the
        // sync output into it, preserving manual additions.
        let ats_key = serde_yaml::Value::String(ats.as_str().into());
        let ats_entry = sources
            .entry(ats_key)
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        if !ats_entry.is_mapping() {
            *ats_entry = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        let Some(ats_map) = ats_entry.as_mapping_mut() else {
            continue;
        };
        let companies_key = serde_yaml::Value::String("companies".into());
        let existing_list: Vec<String> = ats_map
            .get(&companies_key)
            .and_then(serde_yaml::Value::as_sequence)
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();

        let merged = merge_company_list(ats, &existing_list, report);
        let yaml_seq: Vec<serde_yaml::Value> = merged
            .into_iter()
            .map(serde_yaml::Value::String)
            .collect();
        ats_map.insert(companies_key, serde_yaml::Value::Sequence(yaml_seq));
    }
    root
}

/// Build the merged company list for one ATS.
///
/// - Start with `existing` (preserves manually-added slugs).
/// - Union in every slug from `report.keep` and `report.add` for this
///   ATS.
/// - Keep alphabetical order so the diff stays stable across runs.
/// - Slugs in `report.remove` are LEFT alone (only suggested for
///   removal; the user prunes by hand).
fn merge_company_list(ats: AtsVendor, existing: &[String], report: &SyncReport) -> Vec<String> {
    let mut out: Vec<String> = existing.to_vec();
    for hit in report.add.iter().chain(report.keep.iter()) {
        if hit.ats != ats {
            continue;
        }
        if !out.iter().any(|s| s.eq_ignore_ascii_case(&hit.slug)) {
            out.push(hit.slug.clone());
        }
    }
    out.sort();
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use careerai_sources::company_sync::{AtsVendor, CompanyHit};
    use tempfile::TempDir;

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
        // Only one `anthropic` survives; case carries from the first
        // occurrence (the existing entry).
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].to_ascii_lowercase(), "anthropic");
    }

    #[test]
    fn merge_company_list_does_not_drop_remove_partition() {
        // remove just suggests; merge_company_list keeps the slug.
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

        // Re-parse and check the unrelated keys survived.
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
        // Manual `my-private-co` survives, and `anthropic` got added.
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
        // file does not exist; merge should still succeed
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
}
