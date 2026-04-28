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
2. **Detect an LLM backend.** Run `careerai llm probe` and capture the
   `backend:` line. Three outcomes:
   - `backend: claude-cli` — the local `claude` CLI is installed and
     authed. No further action needed; the pipeline will reuse the
     user's Claude Code session (Max/Pro or API key).
   - `backend: api` — the CLI isn't reachable but `ANTHROPIC_API_KEY`
     is set; the rig-core API path will be used. This is fine.
   - The probe exits non-zero with `no LLM backend reachable` — tell
     the user to either install Claude Code
     (https://claude.ai/download) and run `claude login`, or export
     `ANTHROPIC_API_KEY` for the current shell, then re-run setup.
   The careerai CLI no longer requires `ANTHROPIC_API_KEY` for
   subscribers; only set it if you don't have Claude Code or want to
   force the API path with `--llm-backend=api`.
3. **Collect file paths.** If the user passed args, use them. Otherwise ask:
   - "Path to your resume PDF or DOCX?"
   - "Path to your LinkedIn data export ZIP (optional, press Enter to skip)?"
4. **Run scaffold + import.** Execute:
   ```
   careerai init
   careerai profile import <resume_path> [<linkedin_zip>]
   careerai profile validate
   ```
   `profile import` now auto-detects whether to use the LLM extractor
   based on what `careerai llm probe` reports — no `--features
   live-llm` rebuild required for Claude Code subscribers. Surface any
   `LinkedInMissingColumn` or schema-drift warnings prominently —
   those indicate LinkedIn changed its export format and need a
   code-side fix rather than a user fix.
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
6. **Configure notifications (optional).** If the user wants
   pings outside the CLI when something needs attention (cookie
   expiring, source unreachable, high-score match), add a
   `notify.channels.<slack|telegram|email|ntfy>` block to
   `config/local.yaml` and verify it via:
   ```
   careerai notify test
   ```
   The command fires a synthetic `SourceUnreachable` event at
   `Severity::Info` through every configured channel and exits
   non-zero if no channels are wired. Skip this step if the
   user only wants in-terminal output. See
   `docs/NOTIFICATIONS.md` for the full channel walkthroughs.
7. **Offer LinkedIn discovery setup (optional).** Ask: "Do you want to
   enable native LinkedIn discovery? (y/N)". If yes:
   - Confirm the user has stored their `li_at` cookie via `careerai
     cookies set linkedin` (the same flow M5 uses for the submitter).
   - Ask for two values:
     - "LinkedIn search keywords?" (free-form, e.g. `AI engineer remote`).
     - "Location filter? (default: Worldwide)" — accept Enter to keep default.
   - Print the YAML snippet for copy-paste into `config/local.yaml`.
     **Do NOT auto-write the file** — operators tune their own configs;
     overwriting a hand-tuned `local.yaml` is a footgun. Format:
     ```yaml
     # Copy into config/local.yaml. Defaults to enabled=false until you
     # acknowledge LinkedIn ToS §8.2 forbids automated access. Per-tick
     # caps: 3 pages × ~25 cards × 2 calls/min, randomized 1.5–3.5s
     # inter-page jitter.
     sources:
       linkedin_browser:
         enabled: true
         keywords: "<KEYWORDS>"
         location: "<LOCATION>"
         filters:
           remote: true
           posted_within_days: 7
           experience_level: ["mid", "senior"]
         max_pages: 3
     ```
     Substitute `<KEYWORDS>` / `<LOCATION>` with the answers; leave the
     filters block as a starter that the user can prune.
   - Note that the daemon will only register this source when the
     binary is built with `--features browser`. The default `cargo
     install --git ... careerai-cli` build is feature-off; rebuild with
     `--features browser` to activate.
   If the user says no, skip this step entirely.
8. **Print next steps.** Suggest:
   - `/career:discover` to pull listings from configured sources
   - `/career:status` to see pipeline counts
   - Edit `config/local.yaml` to tune sources, cadences, and the score
     threshold (including the commented-out `sources.mcp` block — see
     `crates/careerai-core/src/templates/default.yaml` for the worked
     example)

## Notes

- **LLM backend selection.** `careerai llm probe` reports which path
  the CLI will use; override per-invocation with the global flag
  `--llm-backend=auto|claude-cli|api`, or persistently in
  `config/local.yaml` under `llm.backend`.
- This command is idempotent — re-running it will not overwrite an existing
  `profile/profile.yaml` unless the user passes `--force` to the underlying
  CLI.
