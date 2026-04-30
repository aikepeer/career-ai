# careerai-sources

Discovery adapters — one per job board / ATS — behind a single
`Source` trait. Pluggable: adding a new board means implementing
the trait, never special-casing in the pipeline.

## Boundary

| Owns | Never does |
|---|---|
| `Source` trait (`name`, `fetch_listings`) | Submission (`careerai-submit` does that) |
| `GreenhouseSource`, `LeverSource`, `AshbySource`, `RemotiveSource`, `RemoteOkSource`, `NaukriSource`, `IndeedRssSource`, `McpJobsSource`, `LinkedinBrowserSource` | Tailoring |
| `RawListing` + filter normalization | LLM calls |
| `company_sync` (auto-discover companies from seed) | DB writes (returns `RawListing`s for the pipeline to persist) |
| MCP server probe (`probe_mcp_source`) | Cron scheduling |

## Adapters

| Source | Type | Notes |
|---|---|---|
| `GreenhouseSource` | HTTP JSON | Public API per company slug |
| `LeverSource` | HTTP JSON | Public API per company slug |
| `AshbySource` | HTTP JSON | Public API per company slug |
| `RemotiveSource` | HTTP JSON | Aggregator, single feed |
| `RemoteOkSource` | HTTP JSON | Aggregator, single feed |
| `NaukriSource` | HTTP HTML scrape | India-specific |
| `IndeedRssSource` | RSS | Per query |
| `McpJobsSource` | stdio MCP | Wraps any MCP server exposing `search_jobs` |
| `LinkedinBrowserSource` | Chromium DevTools (feature `browser`) | ToS-violating; default off |

## Adding a source

1. Add `crate::your_source` module implementing `Source`.
2. Register the constructor in `careerai_pipeline::build_sources(cfg)`.
3. Add a config struct under `cfg.sources.your_source`.
4. Add a `wiremock` (or `tiny-http`) integration test against
   captured response shapes.

## Features

| Feature | Effect |
|---|---|
| `browser` | Build `LinkedinBrowserSource` (uses `chromiumoxide` + `stealth-v2.js`) |

## company_sync

Auto-discovers companies whose currently-open jobs match the
operator's `domains:` keywords. Walks the embedded
`seed_companies.yaml` and probes each slug. Soft-fails timeouts /
5xx so a flaky network doesn't nominate working companies for
removal. Output diff merged into `config/local.yaml` via
`careerai sources sync --apply`.

## Tests

```bash
cargo test -p careerai-sources
```

Integration tests use `wiremock` for HTTP and a `tiny-http`
fixture server for browser source HTML. CI never hits real
endpoints.
