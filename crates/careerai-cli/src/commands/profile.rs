//! `careerai profile {import,show,validate}` — profile ingestion +
//! validation. The LLM-extraction adapter and backend resolution
//! live in `super::profile_llm`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ProfileCommand;

/// Default location for the canonical profile file, resolved against the
/// app root (`CAREERAI_ROOT` → CWD → home fallback): `<root>/profile/profile.yaml`.
pub(crate) fn profile_yaml_path() -> PathBuf {
    careerai_core::paths::resolve_root_env()
        .join("profile")
        .join("profile.yaml")
}

pub fn run(
    command: ProfileCommand,
    backend_override: Option<careerai_core::config::BackendChoice>,
) -> Result<()> {
    match command {
        ProfileCommand::Import {
            paths,
            force,
            use_llm,
        } => import(&paths, force, use_llm, backend_override),
        ProfileCommand::Show => show(),
        ProfileCommand::Validate => validate(),
    }
}

fn import(
    paths: &[PathBuf],
    force: bool,
    use_llm: Option<bool>,
    backend_override: Option<careerai_core::config::BackendChoice>,
) -> Result<()> {
    let sources: Vec<PathBuf> = if paths.is_empty() {
        let home = match std::env::var_os("HOME") {
            Some(h) if !h.is_empty() => PathBuf::from(h),
            _ => anyhow::bail!(
                "profile import: no source files given and $HOME is unset; pass source files explicitly"
            ),
        };
        let defaults = default_resume_sources(&home)?;
        let picked = defaults
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect::<Vec<_>>()
            .join(", ");
        println!("no source files given; importing: {picked}");
        defaults
    } else {
        paths.to_vec()
    };
    let out = profile_yaml_path();
    if out.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite",
            out.display(),
        );
    }
    let refs: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();

    // Auto-enable LLM extraction when ANY live backend is compiled in
    // and one is plausibly available — `claude` binary on PATH (auth
    // NOT verified at this stage) or an Anthropic API key reachable.
    // The actual reachability check (including `claude` auth) happens
    // inside `super::profile_llm::run_with_llm`; if it fails we fall
    // back to the heuristic parser and print a hint to `claude login`
    // or export `ANTHROPIC_API_KEY`.
    let live_compiled = cfg!(any(feature = "live-llm-cli", feature = "live-llm-api"));
    let want_llm = match use_llm {
        Some(v) => v,
        None => {
            live_compiled && super::profile_llm::backend_maybe_available(backend_override.as_ref())
        }
    };

    let profile = if want_llm {
        // Fall back to the heuristic parser when LLM resolution fails
        // mid-run. When `--use-llm` was passed explicitly, surface the
        // original error rather than silently downgrading.
        match super::profile_llm::run_with_llm(&refs, backend_override) {
            Ok(p) => p,
            Err(e) if use_llm == Some(true) => {
                return Err(e);
            }
            Err(e) => {
                tracing::warn!(
                    target = "profile",
                    error = %format_args!("{e:#}"),
                    "LLM extraction failed; falling back to heuristic parser"
                );
                eprintln!(
                    "warning: LLM extraction failed ({e}); falling back to heuristic parser.\n\
                     hint: Run `careerai llm probe` to verify your LLM backend, or export DEEPSEEK_API_KEY / ANTHROPIC_API_KEY."
                );
                careerai_profile::import_paths(&refs).context("parsing profile sources")?
            }
        }
    } else {
        if use_llm.is_none() && !live_compiled {
            // Built without any live backend; nothing the user can do
            // at runtime to improve this.
        } else if use_llm.is_none() {
            eprintln!(
                "warning: no LLM backend reachable (claude CLI not authed and no \
                 ANTHROPIC_API_KEY); falling back to heuristic parser. Run \
                 `claude login` or set ANTHROPIC_API_KEY for better results."
            );
        }
        careerai_profile::import_paths(&refs).context("parsing profile sources")?
    };

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let yaml = profile.to_yaml().context("serialize profile")?;
    std::fs::write(&out, &yaml).with_context(|| format!("write {}", out.display()))?;
    tracing::info!(out = %out.display(), bytes = yaml.len(), "profile imported");
    println!("wrote {}", out.display());
    Ok(())
}

/// Default resume sources for `profile import` when no explicit paths
/// are given: scan `$HOME/Documents/personal/` for resume PDF/DOCX
/// files. The canonical `Resume-Kamal-Pandey.pdf` sorts first, the rest
/// alphabetically. Errors name the scanned dir so the user knows where
/// to drop files.
fn default_resume_sources(home: &Path) -> Result<Vec<PathBuf>> {
    let dir = home.join("Documents").join("personal");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => anyhow::bail!(
            "profile import: no source files given; default resume dir {} is unreadable: {e}",
            dir.display(),
        ),
    };
    let mut resumes: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let is_resume = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("Resume"));
            let is_supported = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "pdf" || e == "docx");
            is_resume && is_supported
        })
        .collect();
    resumes.sort_by_key(|p| {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        (name != "Resume-Kamal-Pandey.pdf", name)
    });
    if resumes.is_empty() {
        anyhow::bail!(
            "profile import: no source files given; no Resume*.pdf/docx found in {}",
            dir.display(),
        );
    }
    Ok(resumes)
}

fn show() -> Result<()> {
    let out = profile_yaml_path();
    let text = std::fs::read_to_string(&out).with_context(|| format!("read {}", out.display()))?;
    println!("{text}");
    Ok(())
}

fn validate() -> Result<()> {
    let out = profile_yaml_path();
    let text = std::fs::read_to_string(&out).with_context(|| format!("read {}", out.display()))?;

    // Detect the stale-schema signature (`skills:` followed by a flat
    // sequence) before serde gets a chance to bury the error in a generic
    // "invalid type" message. The 0.x line wrote skills as `Vec<String>`;
    // the current schema is the structured `Skills { languages, ... }`.
    if detect_stale_skills_schema(&text) {
        anyhow::bail!(
            "Detected stale schema (skills as a flat list). Old binary wrote this file.\n\
             Re-run: careerai profile import --force <your sources>"
        );
    }

    let profile = careerai_profile::Profile::from_yaml(&text).context("parse profile yaml")?;
    profile.check().context("profile validation failed")?;
    println!("profile ok: {}", out.display());
    Ok(())
}

/// Stale-schema sniffer for `profile validate`. The 0.x binary wrote
/// `skills:` as a flat YAML sequence (`- Rust\n- Python`). The current
/// schema serializes it as a nested mapping with `languages`,
/// `frameworks`, `tools`. We detect the legacy shape via a quick string
/// scan rather than pulling in a YAML parser — false-positive cost is
/// just a misleading hint, which is fine.
fn detect_stale_skills_schema(text: &str) -> bool {
    let mut in_skills = false;
    for line in text.lines() {
        if !in_skills {
            if line.trim_start() == "skills:" && !line.starts_with(' ') {
                in_skills = true;
            }
            continue;
        }
        // Ignore blanks and pure comments inside the block.
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let leading = line.len() - trimmed.len();
        if leading == 0 {
            // Left the skills block without seeing nested keys.
            return false;
        }
        // First non-empty child of `skills:`. Stale shape:  `- Rust`.
        return trimmed.starts_with("- ");
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn detect_stale_skills_flags_legacy_flat_list() {
        let yaml = "personal:\n  name: Alice\nskills:\n  - Rust\n  - Python\n";
        assert!(detect_stale_skills_schema(yaml));
    }

    #[test]
    fn detect_stale_skills_passes_current_mapping_shape() {
        let yaml = "personal:\n  name: Alice\nskills:\n  languages:\n    - Rust\n";
        assert!(!detect_stale_skills_schema(yaml));
    }

    #[test]
    fn detect_stale_skills_handles_missing_skills_block() {
        let yaml = "personal:\n  name: Alice\nsummary: hi\n";
        assert!(!detect_stale_skills_schema(yaml));
    }

    #[test]
    fn detect_stale_skills_ignores_blank_lines_and_comments() {
        let yaml = "personal:\n  name: Alice\nskills:\n\n  # a comment\n  languages:\n    - Rust\n";
        assert!(!detect_stale_skills_schema(yaml));
    }

    fn personal_dir(home: &Path) -> PathBuf {
        let dir = home.join("Documents").join("personal");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"resume").unwrap();
    }

    #[test]
    fn default_resume_sources_picks_canonical_resume_first() {
        let home = tempfile::tempdir().unwrap();
        let dir = personal_dir(home.path());
        touch(&dir, "Resume-Kamal-Pandey-Honeywell.pdf");
        touch(&dir, "Resume-Kamal-Pandey.pdf");
        touch(&dir, "electricity-bill.pdf");
        let sources = default_resume_sources(home.path()).unwrap();
        let names: Vec<String> = sources
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(
            names,
            vec![
                "Resume-Kamal-Pandey.pdf",
                "Resume-Kamal-Pandey-Honeywell.pdf"
            ]
        );
    }

    #[test]
    fn default_resume_sources_falls_back_to_other_resume_files() {
        let home = tempfile::tempdir().unwrap();
        let dir = personal_dir(home.path());
        touch(&dir, "Resume-Kamal-Pandey-Honeywell.pdf");
        touch(&dir, "DHBVN.pdf");
        let sources = default_resume_sources(home.path()).unwrap();
        let names: Vec<String> = sources
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, vec!["Resume-Kamal-Pandey-Honeywell.pdf"]);
    }

    #[test]
    fn default_resume_sources_accepts_docx_resumes() {
        let home = tempfile::tempdir().unwrap();
        let dir = personal_dir(home.path());
        touch(&dir, "Resume-Kamal-Pandey.docx");
        let sources = default_resume_sources(home.path()).unwrap();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].ends_with("Resume-Kamal-Pandey.docx"));
    }

    #[test]
    fn default_resume_sources_errors_when_dir_missing() {
        let home = tempfile::tempdir().unwrap();
        let err = default_resume_sources(home.path()).unwrap_err();
        assert!(format!("{err:#}").contains("Documents/personal"));
    }

    #[test]
    fn default_resume_sources_errors_when_no_resume_found() {
        let home = tempfile::tempdir().unwrap();
        let dir = personal_dir(home.path());
        touch(&dir, "DHBVN.pdf");
        touch(&dir, "electricity-bill.pdf");
        let err = default_resume_sources(home.path()).unwrap_err();
        assert!(format!("{err:#}").contains("Documents/personal"));
    }
}
