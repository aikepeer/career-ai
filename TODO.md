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

- [ ] **Targeted gap-fill** — `tailor::diff::validate` rule
  permutations, `submit` rate-limiter quiet-hours edge cases,
  `sources::company_sync` large-seed timeouts.

## Performance

- [ ] **Criterion benchmarks** for hot paths: tailor cache hit/miss,
  JaccardScorer against realistic profile/JD sizes, company_sync
  probe-pool throughput.
- [ ] **Flamegraph** on a real `careerai daemon` tick.
- [ ] **Tera template cache reuse** — verify caching is shared across
  renders.

## Documentation

- [ ] **Top-level README audit** — install options, three integration
  paths (CLI / MCP / plugin).
- [ ] **Per-crate READMEs** for crates missing them.
- [ ] **Architecture diagram** — Mermaid sequence for one full
  pipeline tick.
- [ ] **MCP tool manifests** — input/output schema, error classes.

## Out-of-scope ideas (parking lot)

- BGE embedding-based scorer to replace `JaccardScorer`
- `cargo dist` migration to replace hand-rolled release.yml
- Percentile-based shortlisting with real embedding scorer
- Generic tech-acronym whitelist for tailor guardrail
- Native LinkedIn discovery polish (stealth-v2.js refresh)
