//! `careerai service install/status/uninstall` — systemd user service
//! management. Linux-only; on other platforms the subcommands exit with
//! a clear "not supported" message.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

const UNIT_NAME: &str = "careerai";
const UNIT_FILENAME: &str = "careerai.service";

const UNIT_TEMPLATE: &str = r"[Unit]
Description=career-ai pipeline daemon
Documentation=https://github.com/justdoGIT/career-ai
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={CAREERAI_BIN} daemon
WorkingDirectory={CAREERAI_CWD}
Restart=on-failure
RestartSec=10s
# Optional env file (no error if missing); use this to keep secrets out
# of the unit file. Add lines like ANTHROPIC_API_KEY=... or CAREERAI_ROOT=...
EnvironmentFile=-%h/.config/careerai/env
# Resource caps so a runaway daemon does not eat the laptop.
MemoryMax=2G
CPUQuota=80%

[Install]
WantedBy=default.target
";

/// Render the unit-file body with the resolved binary + cwd. Pure;
/// public so integration tests can assert on the output without
/// touching `~/.config/systemd`.
pub fn render_unit(bin: &std::path::Path, cwd: &std::path::Path) -> String {
    UNIT_TEMPLATE
        .replace("{CAREERAI_BIN}", &bin.display().to_string())
        .replace("{CAREERAI_CWD}", &cwd.display().to_string())
}

pub fn run_install(force: bool) -> Result<()> {
    require_linux("install")?;
    let unit_path = unit_path()?;
    let bin = current_bin()?;
    // Pin the daemon's WorkingDirectory to the app root (same resolution
    // the CLI uses). Without this, `systemd --user` starts the service
    // in the manager's default cwd and config/data resolution would read
    // the wrong tree.
    let cwd = careerai_core::paths::resolve_root_env();
    let body = render_unit(&bin, &cwd);

    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }

    let exists = unit_path.exists();
    if exists && !force {
        let current = std::fs::read_to_string(&unit_path)
            .with_context(|| format!("read {}", unit_path.display()))?;
        if current == body {
            println!(
                "service install: already installed at {} (no changes)",
                unit_path.display()
            );
        } else {
            bail!(
                "service install: {} already exists with different content; \
                 re-run with --force to overwrite",
                unit_path.display()
            );
        }
    } else {
        write_atomic(&unit_path, &body)
            .with_context(|| format!("write {}", unit_path.display()))?;
        println!("service install: wrote {}", unit_path.display());
    }

    systemctl_user(&["daemon-reload"]).context("systemctl --user daemon-reload failed")?;

    println!();
    print_post_install(&unit_path);

    if prompt_yes_default("Run `loginctl enable-linger $USER` so the daemon survives logout?")? {
        match enable_linger() {
            Ok(()) => println!("linger: enabled"),
            Err(e) => {
                eprintln!("linger: failed to enable ({e:#})");
                eprintln!(
                    "linger: run manually if needed: \
                     loginctl enable-linger $USER"
                );
            }
        }
    } else {
        println!(
            "linger: skipped — run `loginctl enable-linger $USER` later \
             if you want survive-logout behavior."
        );
    }

    Ok(())
}

pub fn run_status() -> Result<()> {
    require_linux("status")?;
    // Pass through to systemctl so operators see the canonical output.
    let status = Command::new("systemctl")
        .args(["--user", "status", UNIT_NAME])
        .status()
        .context("invoke systemctl")?;
    // status is non-zero when the service is inactive; that's not an error
    // for our wrapper — surface the exit code if it's a real failure
    // (no systemctl found etc.). The inactive case is benign.
    if !status.success() {
        // Codes 3 (inactive) and 4 (no-such-unit) are valid status outcomes,
        // not crashes. Treat any other failure as an error.
        match status.code() {
            Some(3 | 4) => {}
            Some(c) => bail!("systemctl --user status exited with code {c}"),
            None => bail!("systemctl --user status was killed by a signal"),
        }
    }
    Ok(())
}

pub fn run_uninstall() -> Result<()> {
    require_linux("uninstall")?;
    let unit_path = unit_path()?;
    // disable + stop in one shot; ignore "not loaded" errors silently.
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", UNIT_NAME])
        .status();
    if unit_path.exists() {
        std::fs::remove_file(&unit_path)
            .with_context(|| format!("remove {}", unit_path.display()))?;
        println!("service uninstall: removed {}", unit_path.display());
    } else {
        println!("service uninstall: no unit file at {}", unit_path.display());
    }
    systemctl_user(&["daemon-reload"]).ok();
    println!(
        "note: linger setting (loginctl) is left untouched. \
         Run `loginctl disable-linger $USER` manually if you want to \
         clear it."
    );
    Ok(())
}

fn require_linux(action: &str) -> Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        bail!(
            "service {action}: systemd user services are Linux-only. \
             On macOS use launchd; on Windows use Task Scheduler. \
             See docs/SERVICE.md for manual setup instructions."
        )
    }
}

fn unit_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs_home().map(|h| h.join(".config")))
        .ok_or_else(|| anyhow!("cannot resolve config home (HOME unset?)"))?;
    Ok(base.join("systemd/user").join(UNIT_FILENAME))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn current_bin() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("resolve current_exe")?;
    // Resolve symlinks so the unit file points at the actual binary, not a
    // ~/.cargo/bin shim that could be replaced under us.
    let canon = std::fs::canonicalize(&exe).unwrap_or(exe);
    Ok(canon)
}

fn systemctl_user(args: &[&str]) -> Result<()> {
    let mut full = vec!["--user"];
    full.extend_from_slice(args);
    let status = Command::new("systemctl")
        .args(&full)
        .status()
        .context("invoke systemctl")?;
    if !status.success() {
        bail!(
            "systemctl --user {} exited with status {status}",
            args.join(" ")
        );
    }
    Ok(())
}

fn enable_linger() -> Result<()> {
    let user = std::env::var("USER").unwrap_or_default();
    if user.is_empty() {
        bail!("USER env var not set");
    }
    let status = Command::new("loginctl")
        .args(["enable-linger", &user])
        .status()
        .context("invoke loginctl")?;
    if !status.success() {
        bail!("loginctl enable-linger exited with status {status}");
    }
    Ok(())
}

fn write_atomic(path: &std::path::Path, body: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path has no parent: {}", path.display()))?;
    let tmp = tempfile::NamedTempFile::new_in(parent).context("tempfile in unit dir")?;
    {
        let mut f = tmp.as_file();
        f.write_all(body.as_bytes())?;
        f.flush()?;
    }
    tmp.persist(path)
        .map_err(|e| anyhow!("persist tempfile: {e}"))?;
    Ok(())
}

fn prompt_yes_default(question: &str) -> Result<bool> {
    print!("{question} [Y/n] ");
    io::stdout().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf).context("read stdin")?;
    let trimmed = buf.trim().to_lowercase();
    Ok(matches!(trimmed.as_str(), "" | "y" | "yes"))
}

fn print_post_install(unit_path: &std::path::Path) {
    println!("Installed: {}", unit_path.display());
    println!();
    println!("Enable + start with:");
    println!("    systemctl --user enable --now {UNIT_NAME}");
    println!();
    println!("Check status:");
    println!("    careerai service status");
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn render_unit_substitutes_bin_and_cwd() {
        let body = render_unit(
            Path::new("/opt/careerai/bin/careerai"),
            Path::new("/home/kk/career-ai-data"),
        );
        assert!(
            body.contains("ExecStart=/opt/careerai/bin/careerai daemon"),
            "missing ExecStart: {body}"
        );
        assert!(
            body.contains("WorkingDirectory=/home/kk/career-ai-data"),
            "missing WorkingDirectory: {body}"
        );
        assert!(
            body.contains("Restart=on-failure"),
            "missing Restart: {body}"
        );
        assert!(body.contains("MemoryMax=2G"), "missing MemoryMax: {body}");
        assert!(body.contains("CPUQuota=80%"), "missing CPUQuota: {body}");
        assert!(
            body.contains("EnvironmentFile=-%h/.config/careerai/env"),
            "missing EnvironmentFile: {body}"
        );
        assert!(
            body.contains("[Install]\nWantedBy=default.target"),
            "missing Install section: {body}"
        );
        // No leftover placeholders.
        assert!(
            !body.contains("{CAREERAI_BIN}") && !body.contains("{CAREERAI_CWD}"),
            "unsubstituted placeholder: {body}"
        );
    }

    #[test]
    fn unit_path_uses_xdg_config_home_when_set() {
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/test-xdg");
        let path = unit_path().expect("resolve");
        assert_eq!(
            path,
            Path::new("/tmp/test-xdg/systemd/user/careerai.service")
        );
        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    #[test]
    fn write_atomic_round_trip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let target = tmp.path().join("careerai.service");
        let body = "[Unit]\nDescription=test\n";
        write_atomic(&target, body).expect("write_atomic");
        let read_back = std::fs::read_to_string(&target).expect("read");
        assert_eq!(read_back, body);
        // Re-writing must not error and must replace cleanly.
        write_atomic(&target, "[Unit]\nDescription=updated\n").expect("rewrite");
        let updated = std::fs::read_to_string(&target).expect("read updated");
        assert_eq!(updated, "[Unit]\nDescription=updated\n");
    }
}
