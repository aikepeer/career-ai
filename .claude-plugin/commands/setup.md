---
description: First-run wizard — checks prerequisites and imports your resume + LinkedIn export.
argument-hint: "[resume.pdf] [linkedin-export.zip]"
---

# /career:setup

Walks the user through career-ai's first-run setup. This command shells out to
the local `careerai` CLI for file imports because the user must supply real
filesystem paths to their resume and LinkedIn data export.

## What it does

1. **Detect `pandoc`.** Run `pandoc --version`. If missing, print the
   distro-specific install command (Debian/Ubuntu: `sudo apt install pandoc`,
   macOS: `brew install pandoc`, Arch: `sudo pacman -S pandoc`) and stop.
2. **Detect Anthropic API key.** Check the `ANTHROPIC_API_KEY` env var.
   The current CLI reads the key from this env var only — there is no
   `careerai cookies set anthropic` subcommand (the `careerai cookies`
   command only handles LinkedIn / Naukri session cookies, see
   `/career:apply`'s failure-modes section). If `ANTHROPIC_API_KEY` is
   unset, tell the user to export it for the current shell (or add it to
   their shell rc file) and stop. Keyring-backed storage for the
   Anthropic key is a future capability and is not wired today.
3. **Collect file paths.** If the user passed args, use them. Otherwise ask:
   - "Path to your resume PDF or DOCX?"
   - "Path to your LinkedIn data export ZIP (optional, press Enter to skip)?"
4. **Run scaffold + import.** Execute:
   ```
   careerai init
   careerai profile import <resume_path> [<linkedin_zip>]
   careerai profile validate
   ```
   The current CLI on `main` only supports `--force` on
   `careerai profile import`; the regex heuristic is the only import
   path. Surface any `LinkedInMissingColumn` or schema-drift warnings
   prominently — those indicate LinkedIn changed its export format and
   need a code-side fix rather than a user fix. (See Notes for the
   future LLM-extractor flag.)
5. **Probe MCP-source servers (optional).** If the user has configured
   any `kind: mcp` entries under `sources.mcp` in `config/default.yaml`
   or `config/local.yaml`, run `careerai mcp probe` and surface the
   per-source reachability lines verbatim. Format examples:
   ```
   linkedin-jobs-mcp: reachable (4 tools, search_jobs available)
   mcp-linkedin: unreachable -- spawn mcp server `uvx`: No such file or directory
     hint: install/run `uvx mcp-linkedin` and retry
   ```
   The probe is read-only — `careerai mcp probe` calls `initialize` +
   `tools/list` only, never `tools/call`. **Do NOT auto-enable any
   source**; ToS exposure varies per server and the user must flip
   `enabled: true` themselves. If the user has no `kind: mcp` entries
   configured yet, skip this step silently.
6. **Print next steps.** Suggest:
   - `/career:discover` to pull listings from configured sources
   - `/career:status` to see pipeline counts
   - Edit `config/local.yaml` to tune sources, cadences, and the score
     threshold (including the commented-out `sources.mcp` block — see
     `crates/careerai-core/src/templates/default.yaml` for the worked
     example)

## Notes

- **Future: LLM extractor (PR #16, `feat/profile-llm-extract`).** An
  LLM-backed PDF/DOCX extractor is in review. Until that PR merges,
  the regex heuristic is the only import path on `main` and the
  `careerai profile import` CLI does not accept a `--use-llm` flag.
  Once PR #16 merges, builds produced with `cargo install --features
  live-llm` will accept `--use-llm` (auto-detected when an Anthropic
  key is reachable; pass `--use-llm=false` to force the heuristic). Do
  not suggest the flag before the PR lands — the current CLI rejects
  it.
- This command is idempotent — re-running it will not overwrite an existing
  `profile/profile.yaml` unless the user passes `--force` to the underlying
  CLI.
