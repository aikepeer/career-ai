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

## Codex blockers (deferred — revisit for v0.1.2)

- [ ] **Cross-bullet token leak** — `profile_proper_nouns` accepts
  any reword to reuse a token harvested from any bullet. Either
  tighten scope (per-bullet or summary-only) or document the policy.

## Test coverage

- [ ] **Wire `cargo-llvm-cov` into CI** — coverage job, upload to
  Codecov, fail on >1% drop.
- [ ] **Targeted gap-fill** — `tailor::diff::validate` rule
  permutations, `submit` rate-limiter quiet-hours edge cases,
  `sources::company_sync` large-seed timeouts.
- [ ] **Cross-target compile check** — `cargo check --target
  x86_64-pc-windows-gnu` in CI so Windows-incompat changes are
  caught at PR time.

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
- [ ] **`docs/SECURITY.md`** — RUSTSEC-2023-0071 rationale, LinkedIn
  ToS posture, dry-run-by-default invariant, constrained-diff
  invariant.

## Tooling

- [ ] **`cargo-udeps` pass** — requires nightly toolchain (`rustup
  toolchain install nightly`), then `cargo udeps --workspace`.

## Out-of-scope ideas (parking lot)

- BGE embedding-based scorer to replace `JaccardScorer`
- `cargo dist` migration to replace hand-rolled release.yml
- Percentile-based shortlisting with real embedding scorer
- Generic tech-acronym whitelist for tailor guardrail
- Native LinkedIn discovery polish (stealth-v2.js refresh)
