# career-ai TODO

Backlog distilled from the v0.1.1-mcp post-release inventory pass
(2026-04-29). Each item is a concrete unit of work with a target output;
none of them block the v0.1.1-mcp release that is currently published.

Order is rough priority — pick freely.

## File-size splits — 30 files exceed the 300-LOC cap

Project CLAUDE.md mandates that source files stay under 300 LOC; today
30 files violate that. Each split is its own PR. No behavior change,
TDD-light (move tests with the symbols they cover), `cargo check +
clippy + fmt` clean before merge.

### Tier 1 — over 1000 LOC (do these first)

- [ ] **`crates/careerai-cli/src/main.rs`** (1499 LOC) → split into
  per-command modules under `src/commands/`:
  - `commands/profile.rs` — `run_profile`, `profile_import`,
    `profile_show`, `profile_validate`, `profile_yaml_path`,
    `detect_stale_skills_schema`, `anthropic_key_reachable`,
    `llm_backend_maybe_available`, `run_profile_import_with_llm`,
    `strip_provider_prefix`, plus the inline `profile_llm_adapter` mod
    (~600 LOC, biggest cohesive group)
  - `commands/llm.rs` — `run_llm_probe`, `probe_forced_resolve`
  - `commands/mcp.rs` — `run_mcp_probe`
  - `commands/notify.rs` — `run_notify_test`
  - `commands/discover.rs` — `run_discover`
  - `commands/match_.rs` — `run_match`
  - `commands/shortlist.rs` — `run_shortlist_show`
  - `commands/apply.rs` — `run_apply`, `print_apply_line`,
    `run_applied`, `map_apply_error_to_exit_code`
  - `commands/inspect.rs` — `run_inspect`
  - `commands/tailor.rs` — `map_tailor_error_to_exit_code`
  - `commands/render.rs` — `map_render_error_to_exit_code`
  - **Stay in `main.rs`**: `Cli`/`Command` clap structs, `main`,
    `init_tracing`, `load_cfg`, dispatcher, the inline `mod tests` arg-
    parser tests (test the parser, not handlers).

- [ ] **`crates/careerai-pipeline/src/lib.rs`** (1051 LOC) → split by
  pipeline stage: `discover.rs`, `match_.rs` (or rename), `tailor.rs`,
  `render.rs`, `apply.rs`. Keep `lib.rs` as thin re-exports + the
  shared `DiscoveryReport` / `MatchReport` types.

- [ ] **`crates/careerai-llm/src/backend.rs`** (1017 LOC) → likely
  split: `resolution.rs` (BackendChoice + Auto resolver), `probe.rs`
  (auth probe, `probe_forced_resolve` helpers), keep
  `Backend` enum + `complete()` dispatch in `backend.rs`.

### Tier 2 — 500–1000 LOC

- [ ] **`crates/careerai-db/src/queries.rs`** (972 LOC) → split by
  entity: `queries/listings.rs`, `queries/applications.rs`,
  `queries/artifacts.rs`, `queries/payloads.rs`, `queries/events.rs`.
- [ ] **`crates/careerai-core/src/config.rs`** (914 LOC) → split by
  section: `config/{user,domains,match_,rates,submit,llm,scheduler,
  sources,render,notify}.rs`. Top-level `config.rs` re-exports.
- [ ] **`crates/careerai-tailor/src/diff.rs`** (894 LOC) → `diff/
  schema.rs` (DiffDoc + ops), `diff/parse.rs`, `diff/validate.rs`,
  `diff/apply.rs`.
- [ ] **`crates/careerai-tailor/src/guardrails.rs`** (825 LOC) →
  `guardrails/{tokens.rs,common_caps.rs,validator.rs}`.
- [ ] **`crates/careerai-llm/src/claude_cli.rs`** (976 LOC) →
  `claude_cli/{driver.rs,error.rs,binary_locator.rs}`.
- [ ] **`crates/careerai-sources/src/indeed_rss.rs`** (795 LOC).
- [ ] **`crates/careerai-sources/src/company_sync.rs`** (777 LOC) →
  `company_sync/{seed.rs,probe.rs,partition.rs}`.
- [ ] **`crates/careerai-sources/src/mcp_jobs.rs`** (705 LOC).
- [ ] **`crates/careerai-mcp/src/server.rs`** (693 LOC) → `server/
  {tools.rs,resources.rs,handlers.rs}`.
- [ ] **`crates/careerai-scheduler/src/lib.rs`** (686 LOC) — mostly
  cohesive; `scheduler/{cron.rs,shutdown.rs,error.rs}` worth a look.
- [ ] **`crates/careerai-submit/src/rate_limiter.rs`** (598 LOC).
- [ ] **`crates/careerai-submit/src/linkedin.rs`** (565 LOC).
- [ ] **`crates/careerai-llm/src/rig.rs`** (542 LOC).
- [ ] **`crates/careerai-sources/src/naukri.rs`** (539 LOC).
- [ ] **`crates/careerai-notify/src/lib.rs`** (504 LOC) — already has
  `channels::{slack,telegram,email,ntfy}` submodules; split top-level
  into `lib.rs` + `event.rs` + `dispatcher.rs`.

### Tier 3 — 300–500 LOC

`careerai-profile/src/llm_extract.rs` (472), `cli/src/sources_sync.rs`
(445), `profile/src/merge.rs` (432), `profile/src/heuristic.rs` (390),
`mcp/src/schema.rs` (371), `submit/src/credentials.rs` (363),
`sources/src/linkedin_browser_parser.rs` (357),
`sources/src/linkedin_browser.rs` (356), `submit/src/ats_http.rs`
(354), `match/src/filters.rs` (349), `submit/src/lib.rs` (343),
`cli/src/digest.rs` (330).

## Codex blockers from v0.1.1-mcp review (deferred)

Three findings flagged but deemed non-blocking for v0.1.1-mcp; revisit
for v0.1.2:

- [ ] **Cross-bullet token leak** — `profile_proper_nouns` accepts
  any reword to reuse a token harvested from any bullet of the
  profile. Debatable policy call (resume-tailoring practice typically
  allows profile-wide vocabulary reuse) but worth deciding
  explicitly. If we tighten, scope `profile_proper_nouns` per-bullet
  or to summary tokens only.

- [ ] **`std::fs::rename` not portably atomic on Windows** in
  `crates/careerai-cli/src/sources_sync.rs::write_atomic`. On Windows
  `rename` fails when dest exists; needs `MOVEFILE_REPLACE_EXISTING`
  semantics. Affects only Windows operators running
  `careerai sources sync --apply`. Use `tempfile::persist` (or the
  `fs2` crate's `rename_atomic`) for cross-platform behavior.

- [ ] **`notify_threshold` is dead config**. The pipeline scores
  listings and transitions states but never constructs/fires
  `NotifyEvent::HighScoreMatch`, even though the event is wired in
  `careerai-notify`. Add a hook in `careerai-pipeline::match_one`
  (or wherever scoring lands) that emits the event when score >=
  `cfg.matching.notify_threshold`. Either fix the wiring or document
  the field as reserved.

## Test coverage

- [ ] **Wire `cargo-llvm-cov` into CI**. Add a `coverage` job that
  runs `cargo llvm-cov --workspace --lcov --output-path lcov.info`,
  uploads to Codecov (or stores as artifact), and fails on a coverage
  drop > 1%. Establish baseline numbers per crate.
- [ ] **Targeted gap-fill** after baseline lands — focus on
  `careerai-tailor::diff::validate` (rules 1-9, currently has unit
  coverage but not exhaustive permutations), `careerai-submit`
  rate-limiter quiet-hours edge cases, `careerai-sources::company_sync`
  large-seed timeouts.
- [ ] **Cross-target compile check on PRs**. Add a `cargo check
  --target x86_64-pc-windows-gnu` job to ci.yml so Windows-incompat
  changes (PR #44 was a release-time surprise) get caught at PR time.
  Linux runners can do this with `mingw-w64-toolchain` — same crate
  cache, sub-minute incremental.

## Performance

- [ ] **Criterion benchmarks** for the hot paths so regressions
  surface in PRs:
  - `careerai-tailor`: cache hit vs miss latency for `tailor_for_listing`
  - `careerai-match`: `JaccardScorer::score` against a 500-token
    profile / 300-token JD (the realistic shape)
  - `careerai-sources::company_sync`: probe-pool throughput at
    `PROBE_CONCURRENCY=8` against a wiremock farm
- [ ] **Flamegraph** on a real `careerai daemon` tick once Criterion
  fingers a hot spot. (`cargo flamegraph -p careerai-cli -- daemon`)
- [ ] **Match-scorer Tera template hot path** is in the perf-debt
  list per CLAUDE.md ("P0 perf backlog (Tera, pandoc)"). Verify
  whether template caching is reused across renders.

## Documentation

- [ ] **Top-level `README.md` audit** — lead with v0.1.1-mcp install
  options (release tarball / `cargo install --git --tag`); document
  the three integration paths (CLI / MCP / plugin) with one-paragraph
  per path; link to `docs/NOTIFICATIONS.md` and the per-crate READMEs.
- [ ] **Per-crate `README.md`** for the 13 crates that don't have
  one. Crate-level boundary statement + key types + entry-point
  function. Generated by `cargo doc --no-deps` is fine for the API
  surface; the README is for the *narrative* of why this crate
  exists.
- [ ] **Architecture diagram** — current `CLAUDE.md` table is good
  but a Mermaid sequence diagram for one full pipeline tick (discover
  → match → tailor → render → apply) would help new contributors.
- [ ] **MCP tool manifests** — document each of the 8
  `careerai_*` tools with input schema, output schema, error
  classes. Pull from `crates/careerai-mcp/src/schema.rs`.
- [ ] **`docs/SECURITY.md`** — document the rsa Marvin advisory
  (`RUSTSEC-2023-0071`) ignore + rationale (career-ai uses SQLite
  only; sqlx-mysql is a build-dep of sqlx-macros and never runs).
  Also document the LinkedIn ToS posture + the dry-run-by-default
  invariant + the constrained-diff invariant.

## Tooling

- [ ] Install + run `cargo-udeps` once on a nightly toolchain to
  identify any genuinely-unused workspace deps (the workspace clean
  state suggests there are few or none, but worth a one-time pass).
- [ ] Add `cargo deny` config (`deny.toml`) — license + supply-chain
  policy. The CI workflow already references `cargo deny check` per
  CLAUDE.md but no config exists yet.
- [ ] Run **semgrep** baseline scan per global CLAUDE.md rule
  ("on entering a new project, check for `.claude/.semgrep-
  baseline.json`"). Save findings to that file with timestamp +
  commit SHA.

## Out-of-scope ideas (parking lot)

These were noted during the v0.1.1-mcp work but are speculative —
don't start without scoping further:

- BGE embedding-based scorer to replace `JaccardScorer`. The config's
  `embedding_model: BAAI/bge-small-en-v1.5` field anticipates this
  swap.
- Cross-platform `cargo dist` migration to replace the hand-rolled
  release.yml — gives `dist-manifest.json`, `installer.sh`, brew/
  scoop manifests.
- Match-quality follow-up — Codex flagged that the current threshold
  `0.035` shortlists weakly-matching listings (12 results out of
  8853 in the live test, max score `0.039`). With a real embedding
  scorer the percentile-based shortlisting policy makes more sense.
- Generic tech-acronym whitelist for the tailor guardrail (`API`,
  `JSON`, `GPU`, ...). Currently they fail-reject unless the user's
  profile lists them as skills. Either curate the list or build
  sentence-boundary detection so sentence-start capitalization
  doesn't flag them as proper nouns.
- Native LinkedIn discovery polish — the `linkedin_browser` source
  exists but is disabled by default per ToS. If we ever want to ship
  it on, a stealth-v2.js refresh + a smoke test on real li_at
  cookies are prerequisites.
