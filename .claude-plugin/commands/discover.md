---
description: Pull new listings from configured sources into the local pipeline.
argument-hint: "[--sources greenhouse,lever,ashby,...]"
---

# /career:discover

Pulls new job listings from the sources configured in `config/default.yaml`
(or `config/local.yaml`) into the local SQLite pipeline. Each new listing
enters state `discovered` and is then walked through filters and embedding
match.

## What it does

Calls the `careerai_discover` MCP tool from the local careerai server.

Arguments:
- `--sources` (optional): comma-separated subset of sources to run, e.g.
  `greenhouse,lever`. Without this flag, every enabled source in config
  runs.

Each source has its own rate-limit token bucket, daily cap, and quiet-hours
window enforced at the boundary. Discovery never writes to listings
already in a terminal state (`submitted`, `responded`, `failed`).

## Output

Prints a per-source summary:
- New listings added
- Listings deduplicated (already in DB)
- Errors per source (e.g. ATS API 429, parse failure)

For a deeper view of what came in, follow up with `/career:status` or query
the pipeline directly via `careerai shortlist show`.

## Notes

- Discovery is read-only against external services — it never submits
  applications. Submission goes through `/career:apply` with explicit
  consent.
- New listings are matched + filtered by the daemon on the next cron tick;
  to force matching now, run `careerai match` from the shell after this
  command completes.
