//! `careerai init` — scaffold XDG config/data files, or a legacy project
//! layout when `CAREERAI_ROOT` is set.
//!
//! Writes only files that don't already exist unless `force` is set.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tracing::info;

const DEFAULT_YAML: &str = include_str!("templates/default.yaml");
const RULES_EXAMPLE_YAML: &str = include_str!("templates/rules.example.yaml");
const ENV_EXAMPLE: &str = include_str!("templates/.env.example");
const PROFILE_EXAMPLE_YAML: &str = include_str!("templates/profile.example.yaml");

pub fn scaffold(root: &Path, force: bool) -> Result<()> {
    scaffold_at(&root.join("config"), root, force)?;
    write_if_absent(&root.join(".env.example"), ENV_EXAMPLE, force)?;
    Ok(())
}

/// Scaffold normal runtime files in their XDG locations.
pub fn scaffold_env(force: bool) -> Result<()> {
    let data_root = crate::paths::resolve_root_env();
    let explicit_root = std::env::var("CAREERAI_ROOT").is_ok_and(|value| !value.trim().is_empty());
    if explicit_root {
        return scaffold(&data_root, force);
    }

    let config_root = crate::paths::config_dir_for_root(&data_root);
    scaffold_at(&config_root, &data_root, force)?;
    write_if_absent(&config_root.join(".env.example"), ENV_EXAMPLE, force)?;
    Ok(())
}

fn scaffold_at(config_root: &Path, data_root: &Path, force: bool) -> Result<()> {
    ensure_dir(config_root)?;
    ensure_dir(&data_root.join("profile"))?;
    ensure_dir(&data_root.join("artifacts"))?;

    write_if_absent(&config_root.join("default.yaml"), DEFAULT_YAML, force)?;
    write_if_absent(
        &config_root.join("rules.example.yaml"),
        RULES_EXAMPLE_YAML,
        force,
    )?;
    write_if_absent(
        &data_root.join("profile/profile.example.yaml"),
        PROFILE_EXAMPLE_YAML,
        force,
    )?;

    info!(
        config_root = %config_root.display(),
        data_root = %data_root.display(),
        "init: scaffold complete"
    );
    Ok(())
}

fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create dir {}", path.display()))
}

fn write_if_absent(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        info!(path = %path.display(), "init: skipping, file exists");
        return Ok(());
    }
    fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    info!(path = %path.display(), "init: wrote");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scaffold_creates_expected_layout() {
        let tmp = tempfile::tempdir().unwrap();
        scaffold(tmp.path(), false).unwrap();

        assert!(tmp.path().join("config/default.yaml").is_file());
        assert!(tmp.path().join("config/rules.example.yaml").is_file());
        assert!(tmp.path().join("profile/profile.example.yaml").is_file());
        assert!(tmp.path().join(".env.example").is_file());
        assert!(tmp.path().join("artifacts").is_dir());
        assert!(!tmp.path().join("logs").exists());
    }

    #[test]
    fn xdg_scaffold_separates_config_and_data_roots() {
        let config = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();

        scaffold_at(config.path(), data.path(), false).unwrap();

        assert!(config.path().join("default.yaml").is_file());
        assert!(config.path().join("rules.example.yaml").is_file());
        assert!(data.path().join("profile/profile.example.yaml").is_file());
        assert!(data.path().join("artifacts").is_dir());
        assert!(!data.path().join("config").exists());
    }

    #[test]
    fn scaffold_preserves_existing_files_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("config")).unwrap();
        fs::write(tmp.path().join("config/default.yaml"), "custom: true\n").unwrap();

        scaffold(tmp.path(), false).unwrap();

        let contents = fs::read_to_string(tmp.path().join("config/default.yaml")).unwrap();
        assert_eq!(contents, "custom: true\n");
    }

    #[test]
    fn scaffold_overwrites_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("config")).unwrap();
        fs::write(tmp.path().join("config/default.yaml"), "custom: true\n").unwrap();

        scaffold(tmp.path(), true).unwrap();

        let contents = fs::read_to_string(tmp.path().join("config/default.yaml")).unwrap();
        assert_ne!(contents, "custom: true\n");
        assert!(contents.contains("user:"));
    }
}
