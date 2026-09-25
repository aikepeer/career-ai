use std::path::Path;

use anyhow::Context;
use careerai_sources::company_sync::{AtsVendor, SyncReport};

pub(crate) fn merge_into_local_yaml(
    local_path: &Path,
    report: &SyncReport,
) -> anyhow::Result<String> {
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

fn merge_into_value(mut root: serde_yaml::Value, report: &SyncReport) -> serde_yaml::Value {
    if !root.is_mapping() {
        root = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    }
    let Some(root_map) = root.as_mapping_mut() else {
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
        let yaml_seq: Vec<serde_yaml::Value> =
            merged.into_iter().map(serde_yaml::Value::String).collect();
        ats_map.insert(companies_key, serde_yaml::Value::Sequence(yaml_seq));
    }
    root
}

pub(crate) fn merge_company_list(
    ats: AtsVendor,
    existing: &[String],
    report: &SyncReport,
) -> Vec<String> {
    let mut out: Vec<String> = existing.to_vec();
    for hit in report.add.iter().chain(report.keep.iter()) {
        if hit.ats != ats {
            continue;
        }
        if !out.iter().any(|s| s.eq_ignore_ascii_case(&hit.slug)) {
            out.push(hit.slug.clone());
        }
    }
    // Sort case-insensitively *before* dedup so case-variant duplicates are
    // adjacent and removed. A byte-value sort leaves e.g. "ANTHROPIC" and
    // "anthropic" non-adjacent, so both would survive `dedup_by`.
    out.sort_by_key(|s| s.to_ascii_lowercase());
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    out
}
