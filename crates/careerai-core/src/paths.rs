//! App-root resolution: where `config/`, `data/`, and `profile/` live.
//!
//! The CLI, daemon, and dashboard all resolve their root once at startup
//! and join `config/local.yaml`, `data/careerai.sqlite`, and
//! `profile/profile.yaml` onto it. Every entry point uses
//! [`resolve_root_env`] so a `careerai` invocation behaves the same
//! regardless of which binary is driving it.

use std::path::{Path, PathBuf};

/// Resolve the app root from explicit inputs. Pure function — no env or
/// filesystem reads beyond the checks below, so it is fully unit-testable.
///
/// Resolution order:
/// 1. `env_root` (the `CAREERAI_ROOT` override) when non-empty — wins.
/// 2. The process working directory.
///
/// The working directory is intentionally used verbatim. Data created by the
/// application must stay in the directory where the command is run rather
/// than being redirected into a machine-specific home-directory checkout.
pub fn resolve_root(cwd: &Path, _home: Option<&Path>, env_root: Option<&str>) -> PathBuf {
    if let Some(root) = env_root {
        if !root.trim().is_empty() {
            return PathBuf::from(root);
        }
    }
    cwd.to_path_buf()
}

/// [`resolve_root`] with the real environment: `CAREERAI_ROOT` and the
/// process working directory.
pub fn resolve_root_env() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let env_root = std::env::var("CAREERAI_ROOT").ok();
    resolve_root(&cwd, None, env_root.as_deref())
}

/// Return the canonical path to `profile/profile.yaml` for a given root directory.
pub fn profile_path(root: &Path) -> PathBuf {
    root.join("profile").join("profile.yaml")
}

/// Return the canonical path to `profile/profile.draft.yaml` for a given root directory.
pub fn profile_draft_path(root: &Path) -> PathBuf {
    root.join("profile").join("profile.draft.yaml")
}

/// Return the canonical path to `profile/` directory for a given root directory.
pub fn profile_dir(root: &Path) -> PathBuf {
    root.join("profile")
}

/// Return the user state directory used for logs and runtime metadata.
///
/// `XDG_STATE_HOME` is honored when set; otherwise state is stored under
/// `$HOME/.local/state/career-ai`. If no home directory is available, the
/// current directory is used as the final fallback.
pub fn state_dir(home: Option<&Path>, xdg_state_home: Option<&str>, cwd: &Path) -> PathBuf {
    if let Some(base) = xdg_state_home.filter(|value| !value.trim().is_empty()) {
        return PathBuf::from(base).join("career-ai");
    }
    if let Some(home) = home.filter(|path| !path.as_os_str().is_empty()) {
        return home.join(".local").join("state").join("career-ai");
    }
    cwd.join(".local").join("state").join("career-ai")
}

/// Return the directory for application logs and metadata.
pub fn log_dir(home: Option<&Path>, xdg_state_home: Option<&str>, cwd: &Path) -> PathBuf {
    state_dir(home, xdg_state_home, cwd).join("logs")
}

/// Resolve the log directory from the real process environment.
pub fn log_dir_env() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg_state_home = std::env::var("XDG_STATE_HOME").ok();
    log_dir(home.as_deref(), xdg_state_home.as_deref(), &cwd)
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
    fn state_paths_fall_back_to_cwd_without_home() {
        let cwd = Path::new("/workspace/career-ai");
        assert_eq!(
            log_dir(None, None, cwd),
            PathBuf::from("/workspace/career-ai/.local/state/career-ai/logs")
        );
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
