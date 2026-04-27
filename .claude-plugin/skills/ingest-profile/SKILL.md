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
   - For LLM extraction, an Anthropic key must be reachable via the
     `ANTHROPIC_API_KEY` environment variable. (Keyring-backed storage
     for the Anthropic key is a future capability and is not wired
     today; the keyring is currently used for cookie-based sources like
     LinkedIn `li_at` only.) Check; if absent, proceed with the regex
     heuristic and tell the user the LLM path is skipped.

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
   - The current `careerai profile import` CLI on `main` only supports
     `--force` (overwrite an existing `profile/profile.yaml`). Use the
     regex heuristic path — that is the only import path on `main`
     today.
   - **Future / Phase 2 (PR #16, `feat/profile-llm-extract`):** an
     LLM-backed extractor is in review. Once that PR merges, builds
     produced with `cargo install --features live-llm` will accept
     `--use-llm` to route PDF/DOCX text through the LLM extractor (auto
     -detected when an Anthropic key is reachable; pass `--use-llm=false`
     to force the heuristic). Until then, do not suggest the flag —
     it does not exist on the current CLI.
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
- **LLM extraction returns garbage** *(Phase 2 only — PR #16)* — once
  `--use-llm` ships, re-run with `--use-llm=false` to get the regex
  heuristic baseline, then diff the two outputs to see what the LLM
  hallucinated. On `main` today only the heuristic runs, so this case
  does not apply.
- **Anthropic key unreachable** — guide the user to export
  `ANTHROPIC_API_KEY` for the current shell (or add it to their shell
  rc file). Do not write the key to a file. Keyring storage for the
  Anthropic key is not wired today.
- **PDF text extraction empty** — the PDF is likely image-only. Tell the
  user to either OCR it externally or supply a DOCX.
