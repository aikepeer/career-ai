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
2. **Detect Anthropic API key.** Check `ANTHROPIC_API_KEY` env var, then
   keyring (`secret-tool lookup service careerai key anthropic_api_key` on
   Linux, `security find-generic-password -s careerai -a anthropic_api_key`
   on macOS). If neither is present, prompt the user to either export the env
   var or run `careerai cookies set anthropic` (when that subcommand exists)
   — for now, document the env-var path.
3. **Collect file paths.** If the user passed args, use them. Otherwise ask:
   - "Path to your resume PDF or DOCX?"
   - "Path to your LinkedIn data export ZIP (optional, press Enter to skip)?"
4. **Run scaffold + import.** Execute:
   ```
   careerai init
   careerai profile import --use-llm <resume_path> [<linkedin_zip>]
   careerai profile validate
   ```
   Surface any `LinkedInMissingColumn` or schema-drift warnings prominently —
   those indicate LinkedIn changed its export format and need a code-side fix
   rather than a user fix.
5. **Print next steps.** Suggest:
   - `/career:discover` to pull listings from configured sources
   - `/career:status` to see pipeline counts
   - Edit `config/local.yaml` to tune sources, cadences, and the score
     threshold

## Notes

- The `--use-llm` flag auto-detects: it enables LLM extraction when an
  Anthropic key is reachable, and falls back to the regex heuristic
  otherwise. Pass `--use-llm=false` to force the heuristic.
- This command is idempotent — re-running it will not overwrite an existing
  `profile/profile.yaml` unless the user passes `--force` to the underlying
  CLI.
