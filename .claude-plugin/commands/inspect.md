---
description: Inspect an application's row, state history, and rendered artifacts.
argument-hint: "<app-id>"
---

# /career:inspect

Deep-dive on a single application — the row, every state transition logged
in `events`, and the paths to all rendered artifacts.

## Arguments

- `<app-id>` (required): UUID of the application.

## What it does

Calls the `careerai_inspect` MCP tool with `application_id: <app-id>`.

## Output

- The full `applications` row (listing reference, current state, source,
  scores, timestamps).
- The `events` audit trail — every state transition with timestamp and
  payload.
- Artifact paths: tailored Markdown, DOCX, PDF, cover letter, dry-run
  screenshot if any, submission response if submitted.
- LLM call log: prompt cache hit/miss, token cost, model used.

## Use cases

- Debugging why an application is stuck in a state.
- Reviewing what was actually submitted before responding to a recruiter.
- Investigating a `failed` state to decide whether to retry or skip.
