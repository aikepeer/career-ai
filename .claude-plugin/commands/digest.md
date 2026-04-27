---
description: Print a daily summary of pipeline activity over a configurable window.
argument-hint: "[--since 24h|7d|2w|<hours>]"
---

# /career:digest

Calls `careerai_digest` over a user-supplied window and prints the result.

## Arguments

- `--since` (optional, default `24h`): time window. Accepts `24h`, `7d`,
  `2w`, or a bare integer (interpreted as hours).

## What it does

Calls the `careerai_digest` MCP tool with `since: <window>`.

## Output

- Counts by state transition during the window.
- Per-source breakdown (discovered, shortlisted, submitted, responded).
- Last successful cron tick per source.
- Cookie expiry warnings (any session cookie expiring within 7 days).

## Notes

- This is the same data `/career:status` shows for `24h`. Use `/career:digest`
  when you want a different window without changing config.
- Wraps the same `careerai digest --since <window>` CLI subcommand —
  output format matches.
