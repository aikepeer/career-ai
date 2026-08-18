//! `careerai profile {import,show,validate}` — profile ingestion +
//! validation. The LLM-extraction adapter and backend resolution
//! live in `super::profile_llm`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ProfileCommand;

/// Default location for the canonical profile file: `./profile/profile.yaml`.
pub(crate) fn profile_yaml_path() -> Result<PathBuf> {
    Ok(std::env::current_dir()?
        .join("profile")
        .join("profile.yaml"))
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
    if paths.is_empty() {
        anyhow::bail!("profile import: at least one source file is required");
    }
    let out = profile_yaml_path()?;
    if out.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite",
            out.display(),
        );
    }
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();

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

fn show() -> Result<()> {
    let out = profile_yaml_path()?;
    let text = std::fs::read_to_string(&out).with_context(|| format!("read {}", out.display()))?;
    println!("{text}");
    Ok(())
}

fn validate() -> Result<()> {
    let out = profile_yaml_path()?;
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
}
