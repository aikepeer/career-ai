//! Low-level atomic file helpers shared by the profile-handler submodules.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Return the fixed `.bak` sibling path used by `backup_file`.
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(|| OsString::from("file"), std::ffi::OsStr::to_os_string);
    name.push(".bak");
    path.with_file_name(name)
}

/// Copy `path` to `<path>.bak` if it exists, so a destructive write always
/// keeps the last-known-good version. Uses a fixed name (single rolling
/// backup) rather than a timestamp to avoid unbounded `.bak` accumulation.
pub(crate) fn backup_file(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let bak = backup_path(path);
    std::fs::copy(path, &bak).map_err(|e| format!("backup {}: {e}", path.display()))?;
    #[cfg(unix)]
    fix_sudo_ownership(&bak);
    Ok(())
}

/// Build a unique sibling temp path for `path` so concurrent writers never
/// collide on the same fixed `.tmp` name. Uses pid + a process-local counter;
/// both are reset on restart, which is fine because the file is renamed into
/// place (and stale temps are ignored by readers).
pub(crate) fn unique_tmp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .map_or_else(|| OsString::from("tmp"), std::ffi::OsStr::to_os_string);
    name.push(format!(".{}.{}.tmp", std::process::id(), n));
    path.with_file_name(name)
}

#[cfg(unix)]
#[allow(clippy::similar_names)]
pub(crate) fn fix_sudo_ownership(path: &Path) {
    use std::os::unix::fs::MetadataExt;
    // 1. Check SUDO_UID / SUDO_GID
    if let (Ok(uid_s), Ok(gid_s)) = (std::env::var("SUDO_UID"), std::env::var("SUDO_GID")) {
        if let (Ok(uid), Ok(gid)) = (uid_s.parse::<u32>(), gid_s.parse::<u32>()) {
            let _ = std::process::Command::new("chown")
                .arg(format!("{uid}:{gid}"))
                .arg(path)
                .status();
            return;
        }
    }
    // 2. If running as root without SUDO_UID, inherit ownership from parent directory
    if let Some(parent) = path.parent() {
        if let Ok(meta) = std::fs::metadata(parent) {
            let owner_uid = meta.uid();
            let owner_gid = meta.gid();
            if owner_uid != 0 {
                let _ = std::process::Command::new("chown")
                    .arg(format!("{owner_uid}:{owner_gid}"))
                    .arg(path)
                    .status();
            }
        }
    }
}

/// Atomically replace `path` with `contents` (write temp file + rename). The
/// temp file lives in the same directory so the rename never crosses a
/// filesystem boundary.
pub(crate) fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        #[cfg(unix)]
        fix_sudo_ownership(parent);
    }
    let tmp = unique_tmp_path(path);
    std::fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    fix_sudo_ownership(&tmp);
    std::fs::rename(&tmp, path).map_err(|e| format!("commit {}: {e}", path.display()))?;
    #[cfg(unix)]
    fix_sudo_ownership(path);
    Ok(())
}

/// Small helper so YAML key lookups read like the surrounding code.
pub(crate) fn value_str(s: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(s.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn backup_path_appends_bak_suffix() {
        let bak = backup_path(Path::new("profile/profile.yaml"));
        assert_eq!(bak.file_name().unwrap(), "profile.yaml.bak");
        assert_eq!(bak.parent().unwrap(), Path::new("profile"));
    }

    #[test]
    fn backup_file_creates_dot_bak_and_preserves_original() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("profile.yaml");
        std::fs::write(&path, "live: true\n").unwrap();

        backup_file(&path).unwrap();

        let bak = tmp.path().join("profile.yaml.bak");
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), "live: true\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "live: true\n");
    }

    #[test]
    fn backup_file_missing_source_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nope.yaml");
        assert!(backup_file(&path).is_ok());
        assert!(!path.exists());
        assert!(!tmp.path().join("nope.yaml.bak").exists());
    }
}
