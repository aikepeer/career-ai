# Changelog

All notable changes to career-ai. The format roughly follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); pre-1.0
versions don't promise semver yet.

## [0.1.1-mcp] — 2026-04-29

First end-to-end-verified release. Replaces the never-published
`v0.1.0-mcp` tag (which pointed at a commit that pre-dated 9 critical
bug fixes). The full pipeline — discover → match → tailor → render →
apply (dry-run) → daemon — runs cleanly against a populated user
profile and live job listings.

### Fixed

- `careerai-mcp --version` / `--help` no longer hang. The binary now
  uses a `clap::Parser` so flags exit cleanly instead of launching the
  stdio MCP server. (#31)
- `careerai sources sync` no longer marks probe-timed-out slugs for
  removal. Soft-failed slugs (timeout / 5xx / parse error) are
  surfaced under `probe failures` and explicitly skipped in the
  remove-suggestion partition. Previously a flaky network would
  silently nominate `anthropic` and `stripe` for deletion. (#32)
- `careerai discover --source greenhouse,lever,ashby` now splits on
  commas via clap's `value_delimiter`. The repeated form
  `--source greenhouse --source lever` keeps working. Previously the
  comma string was treated as one filter and silently no-op'd. (#33)
- `match.score_threshold` and `match.notify_threshold` defaults
  re-calibrated for the shipping `JaccardScorer` (token Jaccard
  produces scores in ~[0, 0.05]; the previous 0.62 / 0.85 thresholds
  were calibrated for a future BGE-cosine scorer that doesn't ship).
  Defaults are now 0.035 / 0.05. The `embedding_model` field stays in
  the yaml as a forward-compat marker; calibration comment block
  warns to bump together with any embedding-scorer swap. (#34)
- `tailor` prompt template now teaches the LLM the
  `projects[<i>].bullets[<j>]` path shape alongside experience paths.
  The validator (`diff::validate` rule 1) requires every profile
  bullet — experience AND projects — to be covered; the prompt only
  documenting experience caused every tailor cycle on a profile with
  any project bullets to fail with `bullet not covered`. (#35)
- CLI error logging now prints the full anyhow chain
  (`{e:#}` via `format_args!`) instead of the outermost context
  label only. Five `tracing::error!` callsites + one `tracing::warn!`
  switched. Surfaces root causes like
  `tailor failed: tailor_for_listing: schema::parse_and_validate:
  invented proper noun: <token>` instead of just
  `error=tailor_for_listing`. (#36)
- Tailor guardrail now accepts proper nouns from the **original
  bullet** (the carve-out the function's docstring promised but only
  honored for numbers). Reuse of project / system codenames like
  `Symbot6` from a source bullet no longer triggers
  "invented proper noun". (#37)
- `COMMON_ENGLISH_CAPS` extended with ~50 standard resume-action
  verbs (`Architected`, `Engineered`, `Optimized`, `Migrated`, ...)
  so sentence-start verbs picked from the wider resume thesaurus
  clear the proper-noun guardrail. (#38)
- Tailor guardrail now scrapes proper-noun tokens from the **full
  flattened profile prose** (experience + project bullets, summary)
  in addition to the structured employer / project-name / skill
  fields. Terms like `Linux`, `Wayland`, `X11`, `ROS`, `Docker` —
  legitimately the user's own because they typed them into bullets —
  no longer false-reject as "invented". (#39)
- `COMMON_ENGLISH_CAPS` further extended with common
  determiners / pronouns and ~25 temporal / qualifier adverbs
  (`Currently`, `Previously`, `Successfully`, `However`,
  `Specifically`, `Initially`, `Eventually`, ...). Resume bullets
  and summaries that open with these no longer false-reject. (#40)

### Notes

- The `embedding_model: BAAI/bge-small-en-v1.5` config field is
  reserved for a future scorer; the v1 scorer is plain `JaccardScorer`.
  Match quality is therefore deliberately naïve in this release —
  shortlisting is correct on the calibrated threshold but ranking
  semantics are token-overlap, not semantic similarity. Tracking the
  embedding-scorer swap separately.
- Tech acronyms (`API`, `JSON`, `GPU`, ...) still trigger the
  proper-noun guardrail unless the profile lists them as skills.
  Tracking a curated tech-acronym whitelist (or sentence-boundary
  detection) as a follow-up if it surfaces in real use.
- `llm.timeout_seconds` in `default.yaml` raised to 300 (from 120) to
  accommodate the live `claude` CLI subprocess on cold start with
  ~8K-token completions.

### Removed

- The unpublished `v0.1.0-mcp` tag (deleted local + remote). It
  pointed at the merge commit of #30, predating every bug fix above.
  Two stuck release-workflow runs that targeted that commit were
  cancelled.

## [Unreleased]

### Added

- `careerai sources sync` (`feat/sources-sync`) — auto-discovers
  Greenhouse / Lever / Ashby companies whose currently-open jobs
  match the user's `domains:` keywords. Probes a curated seed list
  of ~80 known-public ATS slugs (embedded in
  `crates/careerai-sources/src/templates/seed_companies.yaml`,
  verified 2026-04-28), scores each board against
  `cfg.domains[].keywords_any` (case-insensitive substring across
  title + description), and prints a three-way diff
  (`+ add` / `keep` / `- consider removing`). Default is preview;
  `--apply` writes the merged lists into `config/local.yaml` while
  preserving every other user key (round-trip via `serde_yaml::Value`).
  Manual slugs the user added by hand are never silently dropped.
  Concurrency: up to 8 in-flight probes, 10s per-company timeout,
  HTTP errors soft-fail and surface as `probe_failures`. CLI-only in
  this release; daemon integration is deferred to a later PR.
- New `careerai-sources::AshbySource` adapter for the Ashby public
  posting API (`/posting-api/job-board/<slug>`). Same shape as
  `GreenhouseSource` / `LeverSource`. Wired into `SourcesConfig` as
  `sources.ashby.companies` and `pipeline::build_sources`.

### Security

- **`careerai-llm::ClaudeCliLlm` no longer leaks profile content to
  argv** (PR #21 review). The system prompt + profile block (rendered
  YAML, includes PII) used to ride on `--append-system-prompt <text>`,
  which made the data visible to any local user via
  `/proc/<pid>/cmdline` or `ps -ef`. The driver now writes the
  combined prompt to a 0o600-mode tempfile under
  `target/.careerai-prompts/` (project-local, `.gitignore`'d) and
  passes `--append-system-prompt-file <path>`. argv carries only flags
  + model id. Unit test
  `claude_cli::tests::stub_binary_does_not_leak_profile_to_argv`
  asserts the absence regression.

### Changed

- `feat/claude-cli-backend` — new `careerai-llm::ClaudeCliLlm` driver
  that subprocesses the local `claude` CLI (`--print --output-format
  json`). Default for Claude Code subscribers — no `ANTHROPIC_API_KEY`
  required. Adds `careerai-llm::Backend` resolver and `BackendChoice`
  enum (auto | claude-cli | api), `--llm-backend` global CLI flag,
  `careerai llm probe` subcommand. The `live-llm` feature is split
  into `live-llm-cli` (default) and `live-llm-api`; the umbrella
  `live-llm` alias still toggles both for back-compat. Anthropic
  prompt caching is not exposed via the CLI surface, so
  `cache_profile=true` is silently ignored on the CLI backend
  (logged once).
- `careerai llm probe` honors the global `--llm-backend` flag
  (previously silently ignored). `Backend::probe` also honors
  `CAREERAI_SKIP_CLI_PROBE=1` for parity with `Backend::resolve` and
  `build_cli`.
- `locate_claude_binary` now validates the resolved path is a regular
  executable file. A stale `CAREERAI_CLAUDE_BIN` pointing at a missing
  or non-exec path surfaces as `ClaudeCliError::BinaryUnusable {
  path, reason }` rather than a confusing spawn ENOENT later.
- `careerai profile import` falls back to the heuristic parser when
  LLM resolution fails mid-run (claude session expired, network drop,
  etc.). The user gets a usable profile and a clear log line pointing
  at `claude login` / `ANTHROPIC_API_KEY`. Explicit `--use-llm=true`
  preserves the previous fail-fast behavior.
- `ClaudeCliLlm::complete` no longer maintains a second on-disk cache
  layer with a `claude-cli:` key prefix. The on-disk response cache
  lives in the consumer wrapper (e.g.
  `careerai-tailor::tailor_for_listing`), keyed by the
  provider-agnostic `compose_key(...)` shared with the API backend.
  Two cache layers had been writing duplicate files to the same dir
  with different filenames.
- `feat/mcp-sources-adapter` (PR #19, in review) —
  `careerai-sources::mcp_jobs` adapter consumes any community MCP
  server as a discovery source. New CLI: `careerai mcp probe`
  reachability check. `kind: mcp` source type in config (default
  `enabled: false`).
- `docs/plugin-release` (PR #20) — README + CONTRIBUTING + CHANGELOG
  refreshed; `.mcp.json` migrated to `uvx --from git+...` form
  (community LinkedIn MCPs aren't on PyPI under the canonical names),
  with both community entries `disabled: true` until upstream
  packaging is sorted; `cargo-dist` configured for prebuilt binaries;
  plugin manifest gains `categories` + extended `keywords`.

## [0.1.0-mcp] — 2026-04-27

First version that's installable as a Claude Code plugin.

### Added
- **Plugin shell** (#17, `feat/claude-plugin`) — `.claude-plugin/`
  with 7 slash commands (`/career:setup|discover|tailor|apply|status|digest|inspect`),
  3 skills (`ingest-profile`, `tailor-resume`, `dry-run-apply`),
  3 subagents (`job-hunter`, `resume-tailor`, `application-reviewer`),
  and `.mcp.json` registering `careerai` (local) plus community LinkedIn
  servers.
- **MCP server** (#18, `feat/mcp-server`) — new `careerai-mcp` crate
  using `rmcp` 1.5; 8 tools (`careerai_profile_status`, `_discover`,
  `_shortlist`, `_tailor`, `_render`, `_apply`, `_inspect`, `_digest`),
  resource templates for `careerai://shortlist/{date}` and
  `careerai://artifacts/{application_id}`, hard `dry_run=true` default
  with `confirm: "I_UNDERSTAND_TOS_RISK"` gate for live submission.
  Async I/O throughout (`tokio::fs` for resource reads).
- **LLM-backed profile importer** (#16, `feat/profile-llm-extract`) —
  replaces the heuristic regex parser for free-form PDF resumes;
  constrained JSON-schema prompt → `Profile` with retry-on-validation;
  LinkedIn-first merge order so structured data wins on conflict.
  Gated behind `--features live-llm`.

### Fixed
- Heuristic parser misclassifying experience entries on real PDF text
  (titles like `"Jan 2025"`, education entries that were just years).
- Stale `skills:` schema (legacy `Vec<String>` form) detected at
  `profile validate` with actionable migration message.

## [0.1.0-w3] — 2026-04-27 (sprint week 3)

- `feat(cli): careerai digest daily pipeline summary` (#15) — `careerai
  digest --since 24h` reports counts by state, per-source breakdown,
  last cron tick, and LinkedIn `li_at` cookie-expiry warnings (48h
  window via JWT decode).
- `feat(credentials): cookie_expiry decodes li_at JWT exp claim`.

## [0.1.0-w2] — 2026-04-27 (sprint week 2)

- `feat(submit): drafted state + careerai review walk-through` (#13).
- `fix(submit): copilot follow-up review` (#14).

## [0.1.0-w1] — 2026-04-25 (sprint week 1)

- `feat(filters): must-include keyword filter` (#10) — adds the
  `must_include` filter to `careerai-match` so domain-specific roles
  pass the score threshold even when the JD wording underweights core
  keywords.
- `chore(claude-md): token-discipline rules` (#8) — Serena symbol-level
  + context-mode mandate; subagent propagation snippet.
- `chore(spec): job-search-sprint-spec` (#9).

## [0.1.0-m6] — 2026-04-25

- **M6: scheduler daemon** (#7, `feat/m6-daemon`) — `tokio-cron-
  scheduler` with per-source cadences; graceful shutdown; observability.

## [0.1.0-m5] — 2026-04-25

- **M5a: LinkedIn submitter** (#6, `feat/m5-browser-submit`) —
  `chromiumoxide`-driven Easy Apply, governor token-bucket rate
  limiter (RAII permit), keyring credential store, stealth script
  pinned by SHA, day-aware permits.

## [0.1.0-m4] — 2026-04-25

- **M4: Naukri + Submit trait** (#5, `feat/m4-submit-naukri`) — adds
  Naukri discovery, the `Submitter` trait, ATS HTTP submitters, and
  `careerai apply / applied / inspect` CLI surface.

## [0.1.0-m3] — 2026-04-25

- **M3: tailor + render** (#4, `feat/m3-tailor-render`) — constrained-
  diff resume edits, cover-letter drafting, Tera → Markdown → pandoc
  DOCX/PDF rendering pipeline.

## [0.1.0-m2] — 2026-04-24

- **M2: discovery + matching** (#2, `feat/m2-discovery-match`) — DB
  schema + four ATS adapters (Greenhouse, Lever, Remotive, RemoteOK),
  Jaccard scorer, `careerai discover|match|shortlist` CLI.
- Docs sync (#3, `docs/post-m1-sync`).

## [0.1.0-m1] — 2026-04-23

- **M1: profile pipeline** (#1, `feat/m1-profile-pipeline`) — PDF /
  DOCX / LinkedIn-zip ingestion → canonical `profile/profile.yaml`;
  date normalization; bullet + skill dedup.
- **M0: workspace scaffold** — 13 Rust crates, edition 2021,
  MSRV 1.78. Python stub deleted.
