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
   If the build was produced with `cargo install --features live-llm`
   (or `cargo build --features live-llm`), pass `--use-llm` to route the
   PDF/DOCX text through the LLM extractor; in non-`live-llm` builds the
   regex heuristic always runs and `--use-llm` is unsupported. Surface
   any `LinkedInMissingColumn` or schema-drift warnings prominently —
   those indicate LinkedIn changed its export format and need a
   code-side fix rather than a user fix.
5. **Print next steps.** Suggest:
   - `/career:discover` to pull listings from configured sources
   - `/career:status` to see pipeline counts
   - Edit `config/local.yaml` to tune sources, cadences, and the score
     threshold

## Notes

- The `--use-llm` flag is only available in builds compiled with the
  `live-llm` cargo feature. In those builds it auto-detects: enabled
  when an Anthropic key is reachable, and falls back to the regex
  heuristic otherwise. Pass `--use-llm=false` to force the heuristic.
  In non-`live-llm` builds the heuristic is always used and the flag is
  rejected.
- This command is idempotent — re-running it will not overwrite an existing
  `profile/profile.yaml` unless the user passes `--force` to the underlying
  CLI.
