//! XDG-aware application paths.
//!
//! Explicit `CAREERAI_ROOT` keeps the legacy project layout for isolated
//! workspaces. Without it, configuration, durable data, cache, and runtime
//! state are separated under the user's XDG directories.

use std::path::{Path, PathBuf};

fn non_empty_path(value: Option<&Path>) -> Option<&Path> {
    value.filter(|path| !path.as_os_str().is_empty())
}

fn xdg_dir(
    home: Option<&Path>,
    override_dir: Option<&str>,
    home_suffix: &str,
    cwd: &Path,
    cwd_suffix: &str,
) -> PathBuf {
    if let Some(base) = override_dir.filter(|value| !value.trim().is_empty()) {
        return PathBuf::from(base).join("career-ai");
    }
    if let Some(home) = non_empty_path(home) {
        return home.join(home_suffix).join("career-ai");
    }
    cwd.join(cwd_suffix)
}

/// Resolve the explicit project root. This pure helper preserves the
/// `CAREERAI_ROOT` override and is used by project-layout tests and callers.
pub fn resolve_root(cwd: &Path, _home: Option<&Path>, env_root: Option<&str>) -> PathBuf {
    env_root
        .filter(|root| !root.trim().is_empty())
        .map_or_else(|| cwd.to_path_buf(), PathBuf::from)
}

/// Resolve the application data root; `CAREERAI_ROOT` preserves the
/// explicit project layout.
pub fn resolve_root_env() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(root) = std::env::var("CAREERAI_ROOT")
        .ok()
        .filter(|root| !root.trim().is_empty())
    {
        return PathBuf::from(root);
    }
    data_dir_env_with_cwd(&cwd)
}
fn is_xdg_data_root(root: &Path) -> bool {
    root == resolve_root_env()
}
/// Resolve the XDG configuration directory.
pub fn config_dir(home: Option<&Path>, xdg_config_home: Option<&str>, cwd: &Path) -> PathBuf {
    xdg_dir(home, xdg_config_home, ".config", cwd, "config")
}

/// Resolve the XDG durable-data directory.
pub fn data_dir(home: Option<&Path>, xdg_data_home: Option<&str>, cwd: &Path) -> PathBuf {
    xdg_dir(home, xdg_data_home, ".local/share", cwd, "data")
}

/// Resolve the XDG cache directory.
pub fn cache_dir(home: Option<&Path>, xdg_cache_home: Option<&str>, cwd: &Path) -> PathBuf {
    xdg_dir(home, xdg_cache_home, ".cache", cwd, "data/cache")
}

/// Resolve the XDG state directory.
pub fn state_dir(home: Option<&Path>, xdg_state_home: Option<&str>, cwd: &Path) -> PathBuf {
    xdg_dir(
        home,
        xdg_state_home,
        ".local/state",
        cwd,
        ".local/state/career-ai",
    )
}

fn data_dir_env_with_cwd(cwd: &Path) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let system_home = home::home_dir();
    let xdg_data_home = std::env::var("XDG_DATA_HOME").ok();
    data_dir(
        home.as_deref().or(system_home.as_deref()),
        xdg_data_home.as_deref(),
        cwd,
    )
}

/// Resolve the XDG configuration directory from the process environment.
pub fn config_dir_env() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let system_home = home::home_dir();
    let xdg_config_home = std::env::var("XDG_CONFIG_HOME").ok();
    config_dir(
        home.as_deref().or(system_home.as_deref()),
        xdg_config_home.as_deref(),
        &cwd,
    )
}

/// Return the log directory for explicit path inputs.
pub fn log_dir(home: Option<&Path>, xdg_state_home: Option<&str>, cwd: &Path) -> PathBuf {
    state_dir(home, xdg_state_home, cwd).join("logs")
}
/// Resolve the XDG cache directory from the process environment.
pub fn cache_dir_env() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let system_home = home::home_dir();
    let xdg_cache_home = std::env::var("XDG_CACHE_HOME").ok();
    cache_dir(
        home.as_deref().or(system_home.as_deref()),
        xdg_cache_home.as_deref(),
        &cwd,
    )
}

/// Resolve the XDG state directory from the process environment.
pub fn state_dir_env() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let system_home = home::home_dir();
    let xdg_state_home = std::env::var("XDG_STATE_HOME").ok();
    state_dir(
        home.as_deref().or(system_home.as_deref()),
        xdg_state_home.as_deref(),
        &cwd,
    )
}

/// Select config files for a resolved root. The default XDG data root
/// loads config from `XDG_CONFIG_HOME/career-ai`.
pub fn config_dir_for_root(root: &Path) -> PathBuf {
    let explicit_root = std::env::var("CAREERAI_ROOT")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    if explicit_root || root.join("config").is_dir() || !is_xdg_data_root(root) {
        root.join("config")
    } else {
        config_dir_env()
    }
}

/// Resolve the SQLite database path for either a project root or XDG data root.
pub fn database_path(root: &Path) -> PathBuf {
    let explicit_root = std::env::var("CAREERAI_ROOT")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    if explicit_root || root.join("config").is_dir() || root.join("data").is_dir() {
        root.join("data").join("careerai.sqlite")
    } else {
        root.join("careerai.sqlite")
    }
}

/// Resolve the cache location for either a project root or XDG data root.
pub fn cache_dir_for_root(root: &Path) -> PathBuf {
    let explicit_root = std::env::var("CAREERAI_ROOT")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    if explicit_root
        || root.join("config").is_dir()
        || root.join("data").is_dir()
        || !is_xdg_data_root(root)
    {
        root.join("data").join("cache")
    } else {
        cache_dir_env()
    }
}
/// Return the canonical path to `profile/profile.yaml` for a given data root.
pub fn profile_path(root: &Path) -> PathBuf {
    root.join("profile").join("profile.yaml")
}

/// Return the canonical path to `profile/profile.draft.yaml` for a given root.
pub fn profile_draft_path(root: &Path) -> PathBuf {
    root.join("profile").join("profile.draft.yaml")
}

/// Return the canonical path to `profile/` directory for a given root.
pub fn profile_dir(root: &Path) -> PathBuf {
    root.join("profile")
}

/// Return the log directory without ever falling back to project-local state.
fn log_dir_with_home_fallback(
    environment_home: Option<&Path>,
    system_home: Option<&Path>,
    xdg_state_home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(base) = xdg_state_home.filter(|value| !value.trim().is_empty()) {
        return Some(PathBuf::from(base).join("career-ai").join("logs"));
    }
    environment_home
        .filter(|path| !path.as_os_str().is_empty())
        .or(system_home)
        .map(|home| {
            home.join(".local")
                .join("state")
                .join("career-ai")
                .join("logs")
        })
}

/// Resolve the log directory from the real process environment.
///
/// Logging remains on stderr when neither XDG nor a user home directory can
/// be resolved; it must not create project-local state.
pub fn log_dir_env() -> Option<PathBuf> {
    let environment_home = std::env::var_os("HOME").map(PathBuf::from);
    let system_home = home::home_dir();
    let xdg_state_home = std::env::var("XDG_STATE_HOME").ok();
    log_dir_with_home_fallback(
        environment_home.as_deref(),
        system_home.as_deref(),
        xdg_state_home.as_deref(),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn project_layout(home: &Path) -> PathBuf {
        let project = home.join("projects").join("career-ai");
        std::fs::create_dir_all(project.join("config")).unwrap();
        project
    }

    #[test]
    fn env_root_wins_over_everything() {
        let home = tempfile::tempdir().unwrap();
        project_layout(home.path());
        let cwd = home.path().to_path_buf();
        let env = "/some/explicit/root";
        assert_eq!(
            resolve_root(&cwd, Some(home.path()), Some(env)),
            PathBuf::from(env)
        );
    }

    #[test]
    fn empty_env_root_falls_through_to_cwd() {
        let home = tempfile::tempdir().unwrap();
        project_layout(home.path());
        let cwd = home.path().to_path_buf();
        assert_eq!(resolve_root(&cwd, Some(home.path()), Some("")), cwd);
        assert_eq!(resolve_root(&cwd, Some(home.path()), Some("   ")), cwd);
    }

    #[test]
    fn cwd_in_home_is_used_without_machine_specific_fallback() {
        let home = tempfile::tempdir().unwrap();
        project_layout(home.path());
        let cwd = home.path().to_path_buf();
        assert_eq!(resolve_root(&cwd, Some(home.path()), None), cwd);
    }

    #[test]
    fn cwd_outside_home_is_used_verbatim() {
        let home = tempfile::tempdir().unwrap();
        // The project layout exists, but CWD is elsewhere — CWD wins.
        project_layout(home.path());
        let cwd = tempfile::tempdir().unwrap().path().to_path_buf();
        assert_eq!(resolve_root(&cwd, Some(home.path()), None), cwd);
    }

    #[test]
    fn missing_home_falls_back_to_cwd() {
        let cwd = tempfile::tempdir().unwrap().path().to_path_buf();
        assert_eq!(resolve_root(&cwd, None, None), cwd);
    }

    #[test]
    fn state_paths_use_xdg_state_home_or_home_without_project_data() {
        let home = Path::new("/home/example");
        let cwd = Path::new("/workspace/career-ai");
        assert_eq!(
            state_dir(Some(home), None, cwd),
            PathBuf::from("/home/example/.local/state/career-ai")
        );
        assert_eq!(
            log_dir(Some(home), None, cwd),
            PathBuf::from("/home/example/.local/state/career-ai/logs")
        );
        assert_eq!(
            log_dir(Some(home), Some("/var/lib/user-state"), cwd),
            PathBuf::from("/var/lib/user-state/career-ai/logs")
        );
        assert!(!state_dir(Some(home), None, cwd).starts_with(home.join("data")));
    }

    #[test]
    fn xdg_dirs_are_separated_by_storage_kind() {
        let home = Path::new("/home/example");
        let cwd = Path::new("/workspace/career-ai");
        assert_eq!(
            config_dir(Some(home), None, cwd),
            PathBuf::from("/home/example/.config/career-ai")
        );
        assert_eq!(
            data_dir(Some(home), None, cwd),
            PathBuf::from("/home/example/.local/share/career-ai")
        );
        assert_eq!(
            cache_dir(Some(home), None, cwd),
            PathBuf::from("/home/example/.cache/career-ai")
        );
        assert_eq!(
            state_dir(Some(home), None, cwd),
            PathBuf::from("/home/example/.local/state/career-ai")
        );
    }

    #[test]
    fn xdg_overrides_take_precedence_over_home() {
        let home = Path::new("/home/example");
        let cwd = Path::new("/workspace/career-ai");
        assert_eq!(
            config_dir(Some(home), Some("/run/user/1000/config"), cwd),
            PathBuf::from("/run/user/1000/config/career-ai")
        );
        assert_eq!(
            data_dir(Some(home), Some("/mnt/data"), cwd),
            PathBuf::from("/mnt/data/career-ai")
        );
        assert_eq!(
            cache_dir(Some(home), Some("/mnt/cache"), cwd),
            PathBuf::from("/mnt/cache/career-ai")
        );
    }

    #[test]
    fn database_path_supports_project_and_xdg_data_roots() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir(project.path().join("data")).unwrap();
        assert_eq!(
            database_path(project.path()),
            project.path().join("data/careerai.sqlite")
        );

        let data_root = Path::new("/home/example/.local/share/career-ai");
        assert_eq!(database_path(data_root), data_root.join("careerai.sqlite"));
    }

    #[test]
    fn state_paths_use_system_home_when_environment_home_is_missing() {
        let system_home = Path::new("/home/example");
        assert_eq!(
            log_dir_with_home_fallback(None, Some(system_home), None),
            Some(PathBuf::from("/home/example/.local/state/career-ai/logs"))
        );
        assert_eq!(log_dir_with_home_fallback(None, None, None), None);
    }

    #[test]
    fn profile_helpers_derive_canonical_paths() {
        let root = Path::new("/tmp/test-project");
        assert_eq!(
            profile_path(root),
            PathBuf::from("/tmp/test-project/profile/profile.yaml")
        );
        assert_eq!(
            profile_draft_path(root),
            PathBuf::from("/tmp/test-project/profile/profile.draft.yaml")
        );
        assert_eq!(
            profile_dir(root),
            PathBuf::from("/tmp/test-project/profile")
        );
    }
}
