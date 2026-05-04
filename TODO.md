# career-ai TODO

Backlog refreshed 2026-05-04. Each item is a concrete unit of work;
none block the current release.

## Completed (since last refresh)

- [x] All file-size splits — every Rust source file under the 375 LOC
  cap (350 + 25 buffer, per `~/.claude/CLAUDE.md`).
- [x] `deny.toml` — cargo-deny config with advisory ignore, license
  allowlist, source restrictions.
- [x] Windows atomic rename (`sources_sync/run.rs::write_atomic`) —
  already uses `tempfile::NamedTempFile::persist()`.
- [x] `notify_threshold` — already wired in
  `careerai-pipeline/src/match_.rs` via `fire_high_score_if_above`.
- [x] Semgrep baseline — `.claude/.semgrep-baseline.json` (9 false
  positives; all `tainted-path` in local CLI paths).
- [x] `docs/SECURITY.md` — design invariants, accepted risks, reporting
- [x] Codecov upload — `codecov-action@v5` in CI; needs token in
  secrets + status-check gate in Codecov UI.
- [x] Cross-bullet token leak — already fixed (`summary_proper_nouns`
  excludes bullet bodies in `guardrails/tokens.rs`).
- [x] Cross-target Windows check — CI already has `windows-check` job.
- [x] `cargo-udeps` pass — removed 2 unused dev-deps (docx-rs,
  tempfile).

## Test coverage

- [x] **Targeted gap-fill** — partly done. `tailor::diff::validate`:
  6 tests added (rules 2, 3, 8, 9 — summary guardrails, cover letter
  word cap, MoveBefore parse, missing entry/bullet). `company_sync`:
  probe timeout test with injectable `timeout` param. `rate-limiter`
  quiet-hours: deferred — requires clock abstraction to test safely.

## Performance

- [x] **Criterion benchmarks** — JaccardScorer (65µs–1.8ms across 3
  scales, `benches/jaccard.rs`), LLM cache (miss 23µs, hit 33µs, put
  155µs, `benches/cache.rs`). company_sync probe-pool skipped
  (requires wiremock servers, better suited as integration test).
- [ ] **Flamegraph** on a real `careerai daemon` tick — blocked on
  `sudo apt-get install -y linux-perf` (needed by `cargo flamegraph`).
- [x] **Tera template cache reuse** — `OnceLock<Result<Tera, String>>`
  in `templates.rs:91` caches a single process-wide instance; both
  `render_resume` and `render_cover_letter` share it.

## Documentation

- [x] **Top-level README audit** — stale PR references removed;
  install options and integration paths current.
- [x] **Per-crate READMEs** — all 15 crates have READMEs (42–181 LOC
  each).
- [x] **Architecture diagram** — Mermaid sequence diagram in
  `docs/ARCHITECTURE.md` covering full pipeline tick.
- [x] **MCP tool manifests** — 8 tools documented in
  `docs/MCP_TOOLS.md` (351 lines) with input/output schemas and
  error semantics.

## Out-of-scope ideas (parking lot)

- BGE embedding-based scorer to replace `JaccardScorer`
- `cargo dist` migration to replace hand-rolled release.yml
- Percentile-based shortlisting with real embedding scorer
- Generic tech-acronym whitelist for tailor guardrail
- Native LinkedIn discovery polish (stealth-v2.js refresh)
