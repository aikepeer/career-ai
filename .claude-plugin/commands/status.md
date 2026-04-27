---
description: Show pipeline overview — counts by state, per-source breakdown, last cron tick.
---

# /career:status

Prints a snapshot of the local pipeline so you can see where every listing
and application sits without trawling the SQLite DB.

## What it does

Calls the `careerai_profile_status` MCP tool to confirm a profile is loaded,
then `careerai_digest` with `since: 24h` to summarize recent activity.

## Output

Sections:
1. **Profile** — loaded yes/no, last validated timestamp, schema version.
2. **Pipeline counts** — listings in each state (`discovered`,
   `filtered_out`, `shortlisted`, `tailored`, `rendered`, `prepared`,
   `submitted`, `responded`, `skipped`, `failed`).
3. **Per-source** — listings + applications grouped by source.
4. **Last cron tick** — timestamp of the most recent daemon run.
5. **Cookie expiry warnings** — any session cookies (LinkedIn, Naukri)
   expiring within 7 days, decoded from their JWT `exp` claims.

## Notes

- For a longer window, use `/career:digest --since 7d`.
- For a single application's full row + state history, use
  `/career:inspect <app-id>`.
