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
/// 2. `cwd` when it is NOT the user's home dir (dev runs, tests, and any
///    checkout the operator actually works inside).
/// 3. When `cwd == home` (runit services and bare shells start in
///    `$HOME`), the project workspace `~/projects/career-ai` if it has a
///    `config/` directory — this is the deployed layout for this machine.
/// 4. Fallback: `cwd` unchanged.
pub fn resolve_root(cwd: &Path, home: Option<&Path>, env_root: Option<&str>) -> PathBuf {
    if let Some(root) = env_root {
        if !root.trim().is_empty() {
            return PathBuf::from(root);
        }
    }
    if let Some(home) = home.filter(|h| !h.as_os_str().is_empty() && cwd == *h) {
        let project = home.join("projects").join("career-ai");
        if project.join("config").is_dir() {
            return project;
        }
    }
    cwd.to_path_buf()
}

/// [`resolve_root`] with the real environment: `CAREERAI_ROOT` env var,
/// `$HOME`, and the process working directory.
pub fn resolve_root_env() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let env_root = std::env::var("CAREERAI_ROOT").ok();
    resolve_root(&cwd, home.as_deref(), env_root.as_deref())
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
    fn empty_env_root_falls_through() {
        let home = tempfile::tempdir().unwrap();
        let project = project_layout(home.path());
        let cwd = home.path().to_path_buf();
        assert_eq!(resolve_root(&cwd, Some(home.path()), Some("")), project);
        assert_eq!(resolve_root(&cwd, Some(home.path()), Some("   ")), project);
    }

    #[test]
    fn cwd_in_home_with_project_dir_uses_project() {
        let home = tempfile::tempdir().unwrap();
        let project = project_layout(home.path());
        let cwd = home.path().to_path_buf();
        assert_eq!(resolve_root(&cwd, Some(home.path()), None), project);
    }

    #[test]
    fn cwd_in_home_without_project_dir_keeps_cwd() {
        let home = tempfile::tempdir().unwrap();
        let cwd = home.path().to_path_buf();
        assert_eq!(resolve_root(&cwd, Some(home.path()), None), cwd);
    }

    #[test]
    fn project_dir_without_config_marker_is_ignored() {
        let home = tempfile::tempdir().unwrap();
        // `projects/career-ai` exists but has no `config/` dir — must not
        // be treated as the app root.
        std::fs::create_dir_all(home.path().join("projects").join("career-ai")).unwrap();
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
