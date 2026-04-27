---
description: Tailor the master resume to a shortlisted listing and render to DOCX + PDF.
argument-hint: "<listing-id>"
---

# /career:tailor

Tailors the user's master resume + drafts a cover letter for a single
shortlisted listing, then renders both to DOCX and PDF via pandoc.

## What it does

1. Calls the `careerai_tailor` MCP tool with the supplied `<listing-id>`
   (UUID). The tool produces a constrained JSON diff that can only:
   - reorder existing bullets,
   - rewrite existing bullets,
   - reorder existing skills.
   It cannot invent new experience, titles, dates, or employers — this is
   enforced by the schema validator in `crates/careerai-tailor/src/diff.rs` and is
   non-negotiable.
2. Calls the `careerai_render` MCP tool to produce DOCX + PDF artifacts via
   pandoc.

## Arguments

- `<listing-id>` (required): UUID of a shortlisted listing. Get one from
  `/career:status` or `careerai shortlist show`.

## Output

Prints the application ID (UUID) and the absolute paths of:
- the rendered DOCX
- the rendered PDF
- the cover-letter Markdown source

## Failure modes

- If your profile has not been set up yet, run `/career:setup` first.
- If the supplied listing ID is not found in the DB, or the listing is not
  in state `shortlisted` (already submitted, filtered out, etc.),
  re-check with `/career:status`. Missing listings surface as
  `DbError::NotFound`.
- If tailoring fails because the constrained-diff validator rejects the
  model output (`TailorError::Schema` or `TailorError::InventedContent`),
  re-run the command; persistent failures usually mean the JD is too
  sparse.
- If pandoc is not installed or not available on `PATH`, install it (see
  `/career:setup` step 1).

## Notes

After this command succeeds, the application is in state `rendered` and
ready for `/career:apply <app-id>` (dry-run by default).
