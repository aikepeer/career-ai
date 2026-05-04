# Changelog

All notable changes to career-ai. The format roughly follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); pre-1.0
versions don't promise semver yet.

## [0.1.2] — 2026-05-04

Second end-to-end release. Consolidates two major feature streams (dashboard
+ service management; ATS company-sync + Ashby + Claude CLI backend + MCP
sources adapter), months of file-size refactoring (30+ files split into
per-concern submodules, every Rust source file under 375 LOC), and a
comprehensive CI/docs/test-coverage/performance hardening pass.

### Added

- **Dashboard** (`careerai status serve`) — read-only HTTP dashboard at
  `http://127.0.0.1:8787`. KPI strip, kanban funnel, "next steps" punch
  list, auto-refresh via meta-refresh. Loopback-only by default.
- **Service management** (`careerai service install/status/uninstall`) —
  systemd user unit with `Restart=on-failure`, `MemoryMax=2G`,
  `CPUQuota=80%`. Linux-only.
- `careerai-dashboard` crate (axum + tera).
- **`careerai sources sync`** — probes ~80 curated ATS slugs (Greenhouse /
  Lever / Ashby) and scores each against configured `domains[].keywords_any`.
  Three-way diff (`+ add` / `keep` / `- consider removing`); `--apply` writes
  merged lists into `config/local.yaml` preserving user keys. 8-way
  concurrency, 10s per-company timeout.
- **Ashby adapter** (`careerai-sources::AshbySource`) — public posting API
  (`/posting-api/job-board/<slug>`). Wired into `SourcesConfig`.
- **Claude CLI LLM backend** (`careerai-llm::ClaudeCliLlm`) — subprocesses
  the local `claude` CLI. Default for Claude Code subscribers; no API key
  required. `BackendChoice` enum (auto | claude-cli | api), `--llm-backend`
  flag, `careerai llm probe` subcommand.
- **MCP sources adapter** (`careerai-sources::mcp_jobs`) — consumes community
  MCP servers as discovery sources. `careerai mcp probe` reachability check.
- **Criterion benchmarks** — `JaccardScorer` (65µs–1.8ms across 3 scales) in
  `careerai-match/benches/jaccard.rs`; LLM cache miss/hit/put (23–155µs) in
  `careerai-llm/benches/cache.rs`.
- **Flamegraph** — profiled daemon tick against ~85 companies; careerai
  user-space 45%, tokio runtime 55%, mostly I/O-bound.
- **`docs/SECURITY.md`** — design invariants, accepted risks, reporting.
- **`docs/ARCHITECTURE.md`** — Mermaid sequence diagram covering full
  pipeline tick.
- **`docs/MCP_TOOLS.md`** — 8 tools documented with input/output schemas.
- **Per-crate READMEs** — all 15 crates have READMEs (42–181 LOC each).
- **Semgrep baseline** — `.claude/.semgrep-baseline.json` (9 false positives).

### Changed

- **File-size cap enforcement** — 30+ Rust files split into per-concern
  submodules across 8 PRs (#73–#80): `claude_cli/driver.rs`,
  `claude_cli/tests.rs`, `company_sync/tests.rs`, `tailor/diff.rs`,
  `tailor/diff/tests.rs`, `llm_extract.rs`, `merge.rs`, `heuristic.rs`,
  `rate_limiter.rs`, `linkedin.rs`, and more. Every Rust source file now
  under the 375 LOC cap.
- **Probe timeout injectable** — `probe_one()` takes `timeout: Duration`
  parameter (was hardcoded `PROBE_TIMEOUT`). Enables integration tests to
  assert timeout behavior with `wiremock::set_delay()`.
- **Tera cache process-wide** — `OnceLock<Result<Tera, String>>` caches a
  single instance; both `render_resume` and `render_cover_letter` share it.
- **README audit** — stale PR references removed; install options and
  integration paths current.

### CI / Tooling

- **Codecov** — `codecov-action@v5` uploads coverage in CI; token in
  secrets + status-check gate in Codecov UI.
- **Windows cross-compile check** — CI has `windows-check` job (previously
  gated behind `if: false`).
- **`cargo-deny`** — `deny.toml` with advisory ignore, license allowlist,
  source restrictions.
- **`cargo-udeps`** — pass; removed 2 unused dev-deps (`docx-rs`, `tempfile`).

### Fixed

- **argv PII leak** (PR #21) — system prompt + profile block moved from
  `--append-system-prompt <text>` to a 0o600 tempfile
  (`--append-system-prompt-file <path>`). Regression test in
  `claude_cli/tests.rs`.
- **Cross-bullet token leak** — `summary_proper_nouns` now excludes bullet
  bodies in `guardrails/tokens.rs`.
- **Windows atomic rename** — confirmed `tempfile::NamedTempFile::persist()`
  handles atomic rename on Windows.
- **`notify_threshold`** — confirmed wired in `match_.rs` via
  `fire_high_score_if_above`.
- **Missing feature gate** — `live-llm` feature umbrella alias restored.
- **`cargo-deny` wildcards** — license allowlist tightened from wildcard to
  explicit SPDX identifiers.
- **Tailor diff validation** — 6 new gap-fill tests (rules 2, 3, 8, 9:
  summary guardrails, cover-letter word cap, MoveBefore parse, missing
  entry/bullet).

### Security

- `ClaudeCliLlm` no longer leaks profile content to argv (see Fixed above).
- `docs/SECURITY.md` published with design invariants and accepted risks.

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
  Match quality is therefore deliberately naive in this release —
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
