//! `pandoc` subprocess plumbing.
//!
//! All invocations go through `tokio::process::Command` with a configurable
//! timeout. On timeout we kill the child and reap it to avoid zombies. On
//! non-zero exit we include the last 2 KB of stderr to keep logs
//! actionable without leaking unbounded amounts of noise.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{info, warn};

use crate::config::RenderConfig;
use crate::error::{RenderError, Result};

const STDERR_TAIL_BYTES: usize = 2 * 1024;

/// Allowlist of PDF engines pandoc accepts. Config-sourced `pdf_engine`
/// values outside this set are rejected — prevents a malicious or
/// mistaken `render.pdf_engine` in `config/local.yaml` from pointing
/// pandoc at an arbitrary attacker-controlled path (via `../../x`) or
/// an unexpected binary.
const ALLOWED_PDF_ENGINES: &[&str] = &[
    "weasyprint",
    "typst",
    "wkhtmltopdf",
    "tectonic",
    "xelatex",
    "lualatex",
    "pdflatex",
    "prince",
    "context",
    "pdfroff",
];

/// Validate `cfg.pdf_engine` against `ALLOWED_PDF_ENGINES`.
fn check_pdf_engine(pdf_engine: &str) -> Result<()> {
    if ALLOWED_PDF_ENGINES.contains(&pdf_engine) {
        Ok(())
    } else {
        Err(RenderError::Pandoc {
            code: 0,
            stderr_tail: format!(
                "pdf_engine '{pdf_engine}' is not in the allowlist; pick one of: {}",
                ALLOWED_PDF_ENGINES.join(", ")
            ),
        })
    }
}

/// Resolve the pandoc binary to use. If `cfg.pandoc_bin` is set and the
/// file exists, use it verbatim. Otherwise probe PATH via `which`.
pub fn resolve_pandoc_bin(cfg: &RenderConfig) -> Result<PathBuf> {
    if let Some(bin) = &cfg.pandoc_bin {
        if bin.exists() {
            return Ok(bin.clone());
        }
        warn!(
            target = "render",
            configured = %bin.display(),
            "configured pandoc_bin does not exist; falling back to PATH"
        );
    }
    which::which("pandoc").map_err(|_| RenderError::PandocMissing)
}

/// Spawn pandoc and wait with a timeout. On timeout, kill + reap the
/// child. On non-zero exit, capture the tail of stderr.
async fn spawn_and_wait(bin: &Path, args: &[String], timeout_seconds: u64) -> Result<()> {
    let start = Instant::now();
    let pretty = format!("{} {}", bin.display(), args.join(" "));
    info!(target = "render", cmd = %pretty, "invoking pandoc");

    let mut child = Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let status_fut = child.wait();

    match timeout(Duration::from_secs(timeout_seconds), status_fut).await {
        Ok(Ok(status)) => {
            let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
            if status.success() {
                info!(target = "render", elapsed_ms, "pandoc ok");
                Ok(())
            } else {
                let stderr_tail = read_stderr_tail(&mut child).await;
                let code = status.code().unwrap_or(-1);
                warn!(
                    target = "render",
                    code,
                    elapsed_ms,
                    stderr_tail = %stderr_tail,
                    "pandoc failed"
                );
                Err(RenderError::Pandoc { code, stderr_tail })
            }
        }
        Ok(Err(e)) => Err(RenderError::Io(e)),
        Err(_elapsed) => {
            warn!(
                target = "render",
                seconds = timeout_seconds,
                "pandoc timed out; killing child"
            );
            let _ = child.start_kill();
            let _ = child.wait().await;
            Err(RenderError::TimedOut {
                seconds: timeout_seconds,
            })
        }
    }
}

async fn read_stderr_tail(child: &mut tokio::process::Child) -> String {
    let Some(mut pipe) = child.stderr.take() else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf).await;
    tail_bytes(&buf, STDERR_TAIL_BYTES)
}

fn tail_bytes(bytes: &[u8], max: usize) -> String {
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

pub async fn md_to_docx(md_path: &Path, out_path: &Path, cfg: &RenderConfig) -> Result<()> {
    let bin = resolve_pandoc_bin(cfg)?;
    md_to_docx_with_bin(&bin, md_path, out_path, cfg).await
}

pub async fn md_to_pdf(md_path: &Path, out_path: &Path, cfg: &RenderConfig) -> Result<()> {
    check_pdf_engine(&cfg.pdf_engine)?;
    let bin = resolve_pandoc_bin(cfg)?;
    md_to_pdf_with_bin(&bin, md_path, out_path, cfg).await
}

/// `md_to_docx` variant that skips the `which::which` probe — the caller
/// is expected to have resolved the pandoc binary once via
/// `resolve_pandoc_bin` and pass the same path to each subprocess. Used
/// by `render_application` to amortize the PATH scan across three calls
/// that run in parallel.
pub async fn md_to_docx_with_bin(
    bin: &Path,
    md_path: &Path,
    out_path: &Path,
    cfg: &RenderConfig,
) -> Result<()> {
    let args = vec![
        md_path.to_string_lossy().into_owned(),
        "-o".to_string(),
        out_path.to_string_lossy().into_owned(),
    ];
    spawn_and_wait(bin, &args, cfg.timeout_seconds).await
}

/// `md_to_pdf` variant that skips the `which::which` probe. Still
/// enforces the `pdf_engine` allowlist — the security check belongs on
/// every entry path.
pub async fn md_to_pdf_with_bin(
    bin: &Path,
    md_path: &Path,
    out_path: &Path,
    cfg: &RenderConfig,
) -> Result<()> {
    check_pdf_engine(&cfg.pdf_engine)?;
    let mut args = vec![
        md_path.to_string_lossy().into_owned(),
        "-M".to_string(),
        "title=Resume".to_string(),
        format!("--pdf-engine={}", cfg.pdf_engine),
    ];
    if cfg.pdf_engine == "typst" {
        args.push("-V".to_string());
        args.push("mainfont=Liberation Sans".to_string());
    }
    args.push("-o".to_string());
    args.push(out_path.to_string_lossy().into_owned());
    spawn_and_wait(bin, &args, cfg.timeout_seconds).await
}

/// Convert an HTML document directly to PDF via `weasyprint` (fast-path)
/// or via `pandoc` with configured engine. If the engine is `weasyprint`
/// but the binary is not installed, returns an explicit error rather than
/// falling back to a Markdown→PDF pipeline that would fail confusingly.
pub async fn html_to_pdf(html_path: &Path, out_path: &Path, cfg: &RenderConfig) -> Result<()> {
    check_pdf_engine(&cfg.pdf_engine)?;
    if cfg.pdf_engine == "weasyprint" {
        if let Ok(weasy_bin) = which::which("weasyprint") {
            let args = vec![
                html_path.to_string_lossy().into_owned(),
                out_path.to_string_lossy().into_owned(),
            ];
            return spawn_and_wait(&weasy_bin, &args, cfg.timeout_seconds).await;
        }
        // weasyprint configured but not installed — fail explicitly.
        return Err(RenderError::PandocMissing);
    }
    // For non-weasyprint engines, pandoc can consume HTML directly.
    let bin = resolve_pandoc_bin(cfg)?;
    md_to_pdf_with_bin(&bin, html_path, out_path, cfg).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uses_configured_bin_when_it_exists() {
        // /usr/bin/true exists on typical Linux hosts. If it doesn't, bail
        // gracefully — this test is just asserting the override path.
        let candidate = PathBuf::from("/usr/bin/true");
        if !candidate.exists() {
            return;
        }
        let cfg = RenderConfig {
            pandoc_bin: Some(candidate.clone()),
            ..RenderConfig::default()
        };
        let got = resolve_pandoc_bin(&cfg).unwrap();
        assert_eq!(got, candidate);
    }

    #[test]
    fn resolve_errors_when_pandoc_absent_and_no_override() {
        // Only meaningful if pandoc is actually absent from PATH.
        if which::which("pandoc").is_ok() {
            return;
        }
        let cfg = RenderConfig {
            pandoc_bin: None,
            ..RenderConfig::default()
        };
        let err = resolve_pandoc_bin(&cfg).unwrap_err();
        assert!(matches!(err, RenderError::PandocMissing));
    }

    #[test]
    fn tail_bytes_truncates_head() {
        let v: Vec<u8> = (b'a'..=b'z').collect();
        let s = tail_bytes(&v, 3);
        assert_eq!(s, "xyz");
    }

    #[test]
    fn tail_bytes_returns_all_when_smaller_than_max() {
        let v = b"hi".to_vec();
        assert_eq!(tail_bytes(&v, 100), "hi");
    }
}
