//! Targeted YAML merges into `config/local.yaml` (LLM settings + generated
//! profile config). Every write is atomic and preserves unrelated keys.

use std::path::Path;

use super::util::{atomic_write, backup_file, value_str};

/// Merge the profile-derived sections of a generated config into
/// `config/local.yaml` without clobbering everything else.
///
/// Only `sources.keywords`, `sources.locations`, `filter`, and
/// `scheduler.cadence` are taken from the generated document. Keys the
/// generator doesn't know about — `llm.api_key`, `submit.*`, `notify.*`,
/// `user.*`, `domains`, and per-source company lists — are preserved.
/// The previous full-file overwrite silently deleted those.
/// Compute the merged `config/local.yaml` content produced by
/// `generated` without writing anything. Used by the preview endpoint so
/// the operator can see the before/after before applying.
pub(crate) fn merge_generated_config_doc(
    cfg_path: &Path,
    generated: &str,
) -> Result<String, String> {
    let mut doc: serde_yaml::Value = if cfg_path.exists() {
        let raw = std::fs::read_to_string(cfg_path)
            .map_err(|e| format!("read {}: {e}", cfg_path.display()))?;
        serde_yaml::from_str(&raw).map_err(|e| format!("parse {}: {e}", cfg_path.display()))?
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };
    let generated: serde_yaml::Value =
        serde_yaml::from_str(generated).map_err(|e| format!("parse generated config: {e}"))?;

    let root = doc
        .as_mapping_mut()
        .ok_or_else(|| "config root must be a YAML mapping".to_string())?;
    let Some(gen) = generated.as_mapping() else {
        return Err("generated config must be a YAML mapping".to_string());
    };

    // sources.keywords + sources.locations — merge per-key so an existing
    // `sources.greenhouse.companies` list survives.
    if let Some(gen_sources) = gen
        .get(value_str("sources"))
        .and_then(serde_yaml::Value::as_mapping)
    {
        let sources = root
            .entry(value_str("sources"))
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        let sm = sources
            .as_mapping_mut()
            .ok_or_else(|| "`sources` must be a YAML mapping".to_string())?;
        for key in ["keywords", "locations"] {
            if let Some(v) = gen_sources.get(value_str(key)) {
                sm.insert(value_str(key), v.clone());
            }
        }
    }

    if let Some(filter) = gen.get(value_str("filter")) {
        root.insert(value_str("filter"), filter.clone());
    }

    // scheduler.cadence — merge only the cadence map.
    if let Some(gen_cadence) = gen
        .get(value_str("scheduler"))
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|s| s.get(value_str("cadence")))
    {
        let scheduler = root
            .entry(value_str("scheduler"))
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        let sm = scheduler
            .as_mapping_mut()
            .ok_or_else(|| "`scheduler` must be a YAML mapping".to_string())?;
        sm.insert(value_str("cadence"), gen_cadence.clone());
    }

    serde_yaml::to_string(&doc).map_err(|e| format!("serialize config: {e}"))
}

/// Merge `generated` into `config/local.yaml` and write it atomically,
/// backing up the previous file first. See [`merge_generated_config_doc`].
pub(crate) fn merge_generated_config(cfg_path: &Path, generated: &str) -> Result<(), String> {
    backup_file(cfg_path)?;
    let yaml = merge_generated_config_doc(cfg_path, generated)?;
    atomic_write(cfg_path, &yaml)
}

/// Merge LLM settings into `config/local.yaml`, preserving every other key
/// and any hand edits. This is a targeted YAML merge rather than a full
/// `CoreConfig` re-serialize so unknown/forward keys survive.
pub fn update_llm_settings_in_config(
    cfg_path: &Path,
    backend: &str,
    provider: Option<&str>,
    model: Option<&str>,
    api_base_url: Option<&str>,
    api_key: Option<&str>,
    timeout_seconds: Option<u64>,
) -> Result<(), String> {
    let mut doc: serde_yaml::Value = if cfg_path.exists() {
        let raw = std::fs::read_to_string(cfg_path)
            .map_err(|e| format!("read {}: {e}", cfg_path.display()))?;
        serde_yaml::from_str(&raw).map_err(|e| format!("parse {}: {e}", cfg_path.display()))?
    } else {
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };

    let root = doc
        .as_mapping_mut()
        .ok_or_else(|| "config root must be a YAML mapping".to_string())?;
    let llm = root
        .entry(serde_yaml::Value::String("llm".to_string()))
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    let llm_map = llm
        .as_mapping_mut()
        .ok_or_else(|| "`llm` must be a YAML mapping".to_string())?;

    let set = |map: &mut serde_yaml::Mapping, key: &str, value: serde_yaml::Value| {
        map.insert(serde_yaml::Value::String(key.to_string()), value);
    };

    if !backend.trim().is_empty() {
        set(
            llm_map,
            "backend",
            serde_yaml::Value::String(backend.trim().to_string()),
        );
    }
    if let Some(v) = provider.filter(|s| !s.trim().is_empty()) {
        set(
            llm_map,
            "provider",
            serde_yaml::Value::String(v.trim().to_string()),
        );
    }
    if let Some(v) = model.filter(|s| !s.trim().is_empty()) {
        set(
            llm_map,
            "model",
            serde_yaml::Value::String(v.trim().to_string()),
        );
    }
    if let Some(v) = api_base_url.filter(|s| !s.trim().is_empty()) {
        set(
            llm_map,
            "api_base_url",
            serde_yaml::Value::String(v.trim().to_string()),
        );
    }
    if let Some(v) = api_key.filter(|s| !s.trim().is_empty()) {
        set(
            llm_map,
            "api_key",
            serde_yaml::Value::String(v.trim().to_string()),
        );
    }
    if let Some(t) = timeout_seconds.filter(|t| *t > 0) {
        let value =
            serde_yaml::to_value(t).map_err(|e| format!("serialize timeout_seconds: {e}"))?;
        set(llm_map, "timeout_seconds", value);
    }

    let yaml = serde_yaml::to_string(&doc).map_err(|e| format!("serialize config: {e}"))?;
    // Atomic replace so a crash mid-write never leaves a truncated config.
    atomic_write(cfg_path, &yaml)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn update_llm_settings_preserves_unrelated_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "user:\n  locations: [Remote]\n").unwrap();

        update_llm_settings_in_config(
            &cfg,
            "api",
            Some("deepseek"),
            Some("deepseek-chat"),
            Some("https://api.deepseek.com/v1"),
            Some("sk-secret"),
            Some(120),
        )
        .unwrap();

        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("user:"), "unrelated `user` key lost: {text}");
        assert!(text.contains("Remote"), "unrelated location lost: {text}");
        assert!(text.contains("backend: api"), "backend missing: {text}");
        assert!(
            text.contains("provider: deepseek"),
            "provider missing: {text}"
        );
        assert!(
            text.contains("api_key: sk-secret"),
            "api_key missing: {text}"
        );
        assert!(
            text.contains("timeout_seconds: 120"),
            "timeout missing: {text}"
        );
    }

    #[test]
    fn update_llm_settings_creates_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");

        update_llm_settings_in_config(
            &cfg,
            "auto",
            None,
            Some("claude-3-5-sonnet"),
            None,
            None,
            None,
        )
        .unwrap();

        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("backend: auto"), "backend missing: {text}");
        assert!(
            text.contains("model: claude-3-5-sonnet"),
            "model missing: {text}"
        );
        assert!(
            !text.contains("api_key"),
            "api_key should be absent: {text}"
        );
    }

    #[test]
    fn merge_generated_config_preserves_unrelated_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(
            &cfg,
            "llm:\n  api_key: sk-secret\nsubmit:\n  auto_submit: false\n",
        )
        .unwrap();

        let generated = "sources:\n  keywords: [rust]\n  locations: [Remote]\nfilter:\n  remote_preference: remote_first\nscheduler:\n  cadence:\n    greenhouse: \"0 0 */1 * * *\"\n";
        merge_generated_config(&cfg, generated).unwrap();

        let text = std::fs::read_to_string(&cfg).unwrap();
        assert!(text.contains("api_key: sk-secret"), "llm key lost: {text}");
        assert!(text.contains("auto_submit: false"), "submit lost: {text}");
        assert!(text.contains("rust"), "keyword missing: {text}");
        assert!(text.contains("remote_first"), "filter missing: {text}");
    }

    #[test]
    fn merge_generated_config_preserves_source_company_lists() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(
            &cfg,
            "sources:\n  greenhouse:\n    companies: [anthropic]\n",
        )
        .unwrap();

        let generated = "sources:\n  keywords: [rust]\n  locations: [Remote]\n";
        merge_generated_config(&cfg, generated).unwrap();

        let doc: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(
            doc["sources"]["greenhouse"]["companies"][0].as_str(),
            Some("anthropic")
        );
        assert_eq!(doc["sources"]["keywords"][0].as_str(), Some("rust"));
    }

    #[test]
    fn merge_generated_config_backs_up_previous_file() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config").join("local.yaml");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, "llm:\n  api_key: sk-secret\n").unwrap();

        merge_generated_config(&cfg, "sources:\n  keywords: [rust]\n").unwrap();

        let bak = tmp.path().join("config").join("local.yaml.bak");
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            "llm:\n  api_key: sk-secret\n"
        );
        assert!(std::fs::read_to_string(&cfg).unwrap().contains("rust"));
    }
}
