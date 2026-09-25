//! User-editable filter rules.
//!
//! Loaded from `config/rules.yaml` (fallback: `config/rules.example.yaml`).
//! Shape mirrors the example file seeded by `careerai init`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{MatchError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilterRules {
    #[serde(default)]
    pub exclude_titles: Vec<String>,
    #[serde(default)]
    pub exclude_keywords_in_jd: Vec<String>,
    #[serde(default)]
    pub require_any_keyword_in_jd: Vec<String>,
}

impl FilterRules {
    /// Load from the resolved config directory, falling back to
    /// `rules.example.yaml`. Returns a default (no-op) rule set if neither
    /// exists.
    pub fn load(root: &Path) -> Result<Self> {
        let config_dir = careerai_core::paths::config_dir_for_root(root);
        let primary = config_dir.join("rules.yaml");
        let fallback = config_dir.join("rules.example.yaml");
        let path = if primary.exists() {
            primary
        } else if fallback.exists() {
            fallback
        } else {
            return Ok(Self::default());
        };
        let text = fs::read_to_string(&path)
            .map_err(|e| MatchError::Config(format!("read {}: {e}", path.display())))?;
        serde_yaml::from_str(&text)
            .map_err(|e| MatchError::Config(format!("parse {}: {e}", path.display())))
            .map(Self::normalize)
    }

    /// Lowercase all keyword/title lists so `classify` can do plain
    /// substring comparisons without allocating per rule per listing.
    fn normalize(mut self) -> Self {
        self.exclude_titles = self
            .exclude_titles
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();
        self.exclude_keywords_in_jd = self
            .exclude_keywords_in_jd
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();
        self.require_any_keyword_in_jd = self
            .require_any_keyword_in_jd
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_yield_default_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let rules = FilterRules::load(tmp.path()).unwrap();
        assert!(rules.exclude_titles.is_empty());
        assert!(rules.require_any_keyword_in_jd.is_empty());
    }

    #[test]
    fn rules_yaml_preferred_over_example() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(
            cfg.join("rules.example.yaml"),
            "exclude_titles:\n  - \"should be overridden\"\n",
        )
        .unwrap();
        fs::write(
            cfg.join("rules.yaml"),
            "exclude_titles:\n  - \"sales\"\n  - \"marketing\"\n",
        )
        .unwrap();
        let rules = FilterRules::load(tmp.path()).unwrap();
        assert_eq!(rules.exclude_titles, vec!["sales", "marketing"]);
    }
}
