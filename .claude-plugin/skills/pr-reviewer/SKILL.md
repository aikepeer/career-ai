---
name: pr-reviewer
description: Automated PR review for the career-ai Rust workspace — covers safety invariants, pipeline correctness, submit gates, and code quality. Used by the DeepSeek Claude Code agent workflow.
version: 1.0.0
---

# PR Review — career-ai Rust Workspace

## Output Format (STRICT, one line per finding)

FINDING|path|line|severity|description

- **P1**: blocks merge (bug, security, safety-invariant violation, crash)
- **P2**: should fix (code quality, maintainability, drift risk)
- Descriptions under 120 chars.
- 80%+ confidence only.
- If nothing found: output exactly `No issues found`

---

## Rust Checks

### Safety invariants (P1)

- `unwrap()` / `expect()` on external input, file I/O, network, env vars, or DB results
- `unsafe` block without a safety comment explaining the invariants it upholds
- Missing `?` on `Result` in functions that return `Result` (silent error discard)
- `panic!` / `unreachable!` reachable from non-test code paths
- `todo!()` left in production code (not behind `#[cfg(test)]`)

### career-ai pipeline invariants (P1 — NON-NEGOTIABLE)

These map to the safety boundaries documented in `CLAUDE.md`:

- **Tailor diff grammar**: changes to `crates/careerai-tailor/src/diff/validate.rs` that weaken the nine-rule validator. The diff grammar must reject new experience entries, titles, dates, employers, or skills not in the master profile. Any relaxation of entity guardrails is a P1.
- **Submit dry-run gate**: changes to `crates/careerai-submit/` that could cause a network write when `auto_submit` is false. Submitters must check the gate before any POST/PUT. Bypassing the gate is P1.
- **Per-source submit gate**: submitter code that does not honor `submit_enabled` per-source config flag. All submitters must check this before issuing a network call. P1.
- **Rate limiting**: new submitter that does not acquire a `governor` permit before network calls. Rate limiting is enforced at the boundary, not inside the submitter. Missing permit acquisition is P1.
- **Secrets in logs**: new `tracing` event or `println!` that could leak credentials. The redaction filter in `careerai-submit` must cover all secret shapes. New log sites near credential handling are P1.

### Code quality (P2)

- `.clone()` on large structs inside hot loops or pipeline stages
- `#[tokio::test]` that does not clean up temp files / DB
- New crate dependency without a documented reason in `Cargo.toml` comments
- `sqlx::query!` / `query_as!` without running `cargo sqlx prepare` — CI will fail
- `println!` outside `careerai-cli` (use `tracing` instead)
- Missing `#[cfg(test)]` gate on test-only public API (`pub(crate)` is fine)

### Workspace convention drift (P2)

- Logic (beyond dispatch) in `careerai-cli` that belongs in `careerai-core` or a domain crate
- HTTP, browser, or DB queries directly in `careerai-core` (should be in domain crates)
- Special-casing a specific source/submitter in `careerai-core` or `careerai-pipeline`

---

## Bash / Shell Checks

- P1: Script missing `set -euo pipefail`
- P1: Unquoted variable in `rm`, `chmod`, `chown`, or destructive path
- P1: `curl | bash` or `wget | sh` without checksum verification
- P1: Hardcoded credentials, tokens, or API keys
- P2: `[ $VAR = "val" ]` instead of `[[ "$VAR" = "val" ]]`
- P2: Command substitution used unquoted: `$VAR` from `$(cmd)`

## YAML / Config Checks

- P1: Secret, token, or password value (not reference) in plain YAML
- P1: `chmod 777` or world-writable path
- P1: `sudo` without specific command — bare `sudo bash` or `sudo su`
- P2: Missing `no_log: true` on tasks handling secrets (Ansible)
- P2: Hardcoded thresholds/cadences that belong in `config/default.yaml`

## SQL / Migration Checks

- P1: Destructive migration without a rollback plan (`DROP TABLE`, `DROP COLUMN`)
- P1: Migration that would break existing data (type change without cast)
- P2: Migration not idempotent — running twice errors out
- P2: Missing `DOWN` section in migration file

## GitHub Actions / CI Checks

- P1: `secrets.XXX` used in a context that could be exposed to forked PRs
- P1: `actions/checkout` without `persist-credentials: false` on PR-targeted workflows
- P2: Hardcoded runner label (`ubuntu-22.04`) that will break on deprecation
- P2: Missing `timeout-minutes` on job

## Skip Entirely

- Style, formatting, naming conventions
- Comment grammar or spelling
- Line length, import ordering
- Whitespace changes
- Test coverage percentage (unless zero coverage on new safety code)
