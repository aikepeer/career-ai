# careerai-tailor

Constrained-diff resume tailoring + cover letter drafting. Holds
the **load-bearing safety invariant** that prevents the LLM from
inventing experience.

## Boundary

| Owns | Never does |
|---|---|
| `tailor_for_listing(pool, llm, listing_id, profile, cfg, base_dir)` entry point | DB queries |
| `Diff` schema (`Reword`, `Reorder`, `EmphasizeBullets`, `EmphasizeProjects`) | Rendering to DOCX/PDF (`careerai-render` does that) |
| `validate_diff` — 9 rules including the constrained-grammar gate | LLM transport (uses `&dyn Llm` from `careerai-llm`) |
| `forbid_invented_entities[_with]` — proper-noun / number / year / employer guardrails | Cron / scheduling |
| `apply_diff` — produces `ResumeView` from base profile + diff | Argument parsing |
| `CoverLetter` model + length-cap enforcement |  |

## Safety invariant (non-negotiable)

The diff schema is closed. The LLM may only:

* Reword existing bullets (subject to the
  `forbid_invented_entities` allowlist).
* Reorder existing experience entries / projects.
* Emphasize a subset of existing bullets.

It cannot invent new employers, new bullets, new projects, new
years, new numbers, or proper nouns absent from the profile.
Violations surface as `TailorError::InventedContent { path, token,
context }`.

Allowlist scope: profile summary + structured identifier fields
(company, project name, institution, title, location) + skills +
per-bullet original text + `COMMON_ENGLISH_CAPS` whitelist. NOT
experience-bullet bodies (cross-bullet token-leak protection
added 2026-04 in PR #53).

## Tests

```bash
cargo test -p careerai-tailor                   # all
cargo test -p careerai-tailor guardrails::      # just guardrails
```

Tests assert the rejection path (every rule's negative case) AND
the legitimate-input acceptance path. Touch any guardrail rule and
keep these tests strict.

## Module layout

```
src/
  lib.rs              tailor_for_listing entry
  diff/{schema,parse,validate,apply}.rs   constrained diff grammar
  guardrails/         token sets + invented-entity rejection
  prompt.rs           system prompt + Tera template
  cover_letter.rs     cover letter generation
  model.rs            ResumeView / CoverLetter
  error.rs            TailorError variants
```
