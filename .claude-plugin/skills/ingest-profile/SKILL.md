---
name: ingest-profile
description: Walks the user through importing a resume PDF/DOCX and optional LinkedIn data export ZIP into profile/profile.yaml using the LLM extractor, then validates the result. Triggers when the user mentions importing a resume, loading a LinkedIn export, or shares a resume file.
---

# Ingest profile

Imports the user's resume + LinkedIn data into the canonical
`profile/profile.yaml` that drives every downstream stage (filters,
embedding match, tailoring, rendering).

## Trigger conditions

Activate when the user:
- Says "import my resume", "load my LinkedIn", "ingest my profile", or
  similar.
- Pastes or attaches a `.pdf`, `.docx`, or LinkedIn `Basic_LinkedInDataExport_*.zip`.
- Asks how to set up career-ai's profile for the first time.
- Reports `MissingProfile` errors from any other career-ai command.

## Process

1. **Confirm prerequisites.**
   - `pandoc` is needed for downstream rendering, not import — don't block
     on it here, but mention it.
   - For LLM extraction, **either** Claude Code is installed and authed
     (`claude login`) **or** `ANTHROPIC_API_KEY` is set. Run
     `careerai llm probe` to verify; the `backend:` line will say
     `claude-cli`, `api`, or the command will exit non-zero. Max/Pro
     subscribers do NOT need an API key — the CLI subprocess driver
     bills against their session. If neither path is reachable,
     proceed with the regex heuristic and tell the user the LLM path
     is skipped.

2. **Locate inputs.**
   - Ask for the resume path (`.pdf` or `.docx`).
   - Ask for the LinkedIn export path (`.zip`) — optional. Skipping is
     fine; LinkedIn provides extra signals (skills, endorsements,
     positions) but is not required.

3. **Run import.**
   ```
   careerai profile import <resume_path> [<linkedin_zip>]
   careerai profile validate
   ```
   - The CLI auto-enables `--use-llm` when a live backend is reachable
     (claude CLI authed or `ANTHROPIC_API_KEY` set). To force the
     heuristic baseline, pass `--use-llm=false`. To override the
     backend choice, pass `--llm-backend=claude-cli` or
     `--llm-backend=api`.
   - `--force` overwrites an existing `profile/profile.yaml`. Do not pass
     it without explicit user consent.

4. **Surface warnings.**
   - `LinkedInMissingFile` is silently tolerated — only mention it if the
     user asks why a particular CSV's data isn't in the profile.
   - `LinkedInMissingColumn` indicates LinkedIn renamed an export column.
     This is a **schema-drift bug**, not a user issue — tell the user the
     project needs a code-side fix and capture the column name and CSV
     filename in the report.
   - LLM extraction may produce structured warnings (low-confidence
     fields, ambiguous date ranges). Surface each warning with the field
     name and ask the user to confirm or correct.

5. **Confirm.**
   - After `validate` succeeds, print the resolved profile fields
     (`name`, `headline`, count of experiences, count of skills) and ask
     the user to spot-check.

## Safety constraints

- Never edit `profile/profile.yaml` directly. Always go through
  `careerai profile import` so dedup rules (case-sensitive bullets,
  case-insensitive skills, empty-`start`-date preservation) are applied
  consistently.
- Never pass `--force` without the user saying "overwrite" or "replace".

## Failure modes + recovery

- **`LinkedInMissingColumn`** — capture the column + CSV name. Open an
  issue on the project repo; do not paper over by manually editing the
  CSV.
- **LLM extraction returns garbage** — re-run with `--use-llm=false`
  to get the regex heuristic baseline, then diff the two outputs to
  see what the LLM hallucinated.
- **No backend reachable** — `careerai llm probe` will exit non-zero.
  Either install Claude Code (https://claude.ai/download) and run
  `claude login`, or export `ANTHROPIC_API_KEY` for the current shell
  (or add it to the shell rc file). Do not write the key to a
  committed file.
- **PDF text extraction empty** — the PDF is likely image-only. Tell the
  user to either OCR it externally or supply a DOCX.
