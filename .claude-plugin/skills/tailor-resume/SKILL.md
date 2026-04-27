---
name: tailor-resume
description: Tailors the master resume + drafts a cover letter for a specific shortlisted listing, then renders DOCX + PDF via pandoc. Triggers when the user mentions a listing ID, asks to tailor for a job, or wants to customize their resume for a specific posting.
---

# Tailor resume

Drives the LLM tailoring + rendering stage of the pipeline for a single
shortlisted listing.

## Trigger conditions

Activate when the user:
- Says "tailor my resume for `<listing-id>`" or "tailor for this job".
- Pastes a job description and asks for a tailored resume.
- Has a listing in `shortlisted` state and wants to move it forward.

## Process

1. **Verify the listing.** Resolve the listing ID. If the user pasted a JD
   without an ID, suggest running `/career:discover` first or ingesting
   the JD via the daemon's manual-add path (when implemented).

2. **Tailor.** Call `careerai_tailor` MCP tool with `listing_id: <id>`.
   The tool:
   - Loads the master profile (cached as an Anthropic prompt-cache block —
     the cache must hit for cost efficiency).
   - Sends the JD + bullet-rewrite prompt.
   - Receives a constrained JSON diff and validates it against the strict
     grammar in `crates/careerai-tailor/src/diff.rs`.
   - Applies the diff to produce a tailored profile snapshot for this
     application.

3. **Render.** Call `careerai_render` MCP tool with the application ID
   returned from step 2. Produces:
   - tailored resume DOCX
   - tailored resume PDF
   - cover-letter Markdown source

4. **Surface artifacts.** Print absolute paths to all three. Suggest the
   user open them and review before applying.

## The constrained-diff invariant (non-negotiable)

The tailoring step **cannot fabricate**. Specifically, the diff grammar
permits only:
- reordering existing bullets
- rewriting existing bullets (must reference an existing bullet ID)
- reordering existing skills

It rejects:
- new experience entries
- new titles, dates, or employers
- new skills not in the master profile

If the LLM ever emits anything outside this grammar, the validator
rejects the whole diff and the call fails with `TailorError::Schema` (or
`TailorError::InventedContent` when the model proposes a bullet that
isn't grounded in the master profile). Re-running usually fixes
transient model glitches; persistent failures mean the JD is too sparse
to tailor against.

When discussing tailored output with the user, never describe a bullet
that isn't grounded in the master profile — that would defeat the whole
safety story.

## Safety constraints

- Do not bypass the diff validator. If a user asks to "just make up some
  more experience", refuse and explain the invariant.
- Do not inline the master profile into per-call prompts without the
  Anthropic prompt-cache block — that breaks the cost story.

## Failure modes + recovery

- **`TailorError::Schema` / `TailorError::InventedContent`** — diff
  failed validation. Re-run once. If it fails twice, capture the JD and
  skip with a note.
- **Pandoc missing** — install pandoc and re-run only the render step
  via `careerai_render`.
- **Profile missing (`TailorError::Profile`)** — run the
  `ingest-profile` skill first.
- **Listing not found (`DbError::NotFound`) or wrong state** — listing is
  not in `shortlisted` state. Inspect with `/career:inspect` to see why.
