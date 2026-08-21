# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

M0 (workspace scaffold) and M1 (profile ingestion) are merged on `main`. M2 (discovery + matching) is in review on `feat/m2-discovery-match` (PR #2). Remaining milestones M3–M7 (LLM tailoring + render, dry-run submitters, browser auto-apply, daemon, follow-ups) are unstarted. Scope, tech stack, and milestones are summarized below; refer to this file before making non-trivial changes.

The Python stub (`main.py`, `pyproject.toml`, `uv.lock`, `.python-version`, `.venv/`) was deleted in M0. Do not add Python code or dependencies — the direction is Rust end-to-end.

The profile pipeline (M1) ingests `.pdf` / `.docx` / LinkedIn data-export `.zip` into `profile/profile.yaml`. Errors distinguish a missing file (`LinkedInMissingFile`) from a renamed-column schema drift (`LinkedInMissingColumn`); only `LinkedInMissingFile` is silently tolerated when stitching optional CSVs. Bullet dedup is case-sensitive (acronyms preserved); skill dedup is case-insensitive. Experience entries with empty `start` dates are never deduped. `dates::two_digit_month` validates the month is in `1..=12`; out-of-range inputs pass through verbatim rather than producing invalid `YYYY-00` / `YYYY-13` strings.

## What this project is

A local, single-user pipeline that discovers jobs (ATS APIs + feeds + LinkedIn + Indeed), matches them to the user's profile, tailors a resume + cover letter per JD via LLM, and auto-applies with a dry-run safety gate. Target niche: **AI/ML + LLM apps** and **embedded platforms / robotics**, remote-first with Delhi-NCR fallback. Runs as a local daemon with `tokio-cron-scheduler` plus a `clap` CLI for one-shots.

## Commands

Common commands:

```bash
# Build + check
cargo build                                    # dev build all workspace crates
cargo build --release --workspace              # release profile — always build alongside dev commands
cargo check --workspace                        # fast type-check, no codegen

# Tests
cargo test --workspace                         # all tests
cargo test -p careerai-match                   # single crate
cargo test -p careerai-match -- filters::      # single module
cargo test --test pipeline_it                  # single integration test file
cargo nextest run --workspace                  # faster, better output (preferred in CI)
INSTA_UPDATE=always cargo test                 # accept new insta snapshots

# Lint + format + security
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo audit                                    # CVE scan of dependencies
cargo deny check                               # license + supply-chain policy
semgrep scan --config auto                     # per global CLAUDE.md

# DB
sqlx migrate run                               # apply migrations to DATABASE_URL
sqlx migrate add <name>                        # create new migration

# Run the CLI
cargo run -p careerai-cli -- --help
cargo run -p careerai-cli -- init
cargo run -p careerai-cli -- discover --source greenhouse
cargo run -p careerai-cli -- daemon
```

**External runtime dependency:** `pandoc` must be on PATH for resume rendering (DOCX + PDF). Install via distro package manager.

## Architecture (big picture)

The pipeline is a linear state machine persisted in SQLite. One `Listing` moves through these states, one row per transition appended to `events` for audit:

```
discovered → filtered_out | shortlisted → tailored → rendered → prepared → submitted | skipped | failed → responded
```

Two entry points drive the same stages:
- **Daemon** (`careerai daemon`) — `tokio-cron-scheduler` runs each source on its own cadence (see `config/default.yaml`), walks new listings through the pipeline.
- **CLI subcommands** (`discover`, `match`, `tailor`, `apply`, `inspect`) — same code paths, manual invocation.

**Crate boundaries are deliberate — respect them:**

| Crate | Owns | Never does |
|---|---|---|
| `careerai-core` | Pipeline orchestration, state transitions, config loading | HTTP, browser, DB queries directly |
| `careerai-db` | `sqlx` models + queries + migrations | Business logic |
| `careerai-sources` | Discovery adapters (one per job board) behind a `Source` trait | Submission, rendering |
| `careerai-match` | Filters, fastembed embeddings, cosine rank | LLM calls |
| `careerai-llm` | `rig`-based provider gateway, prompt building, response cache | Profile/listing schemas (imports them) |
| `careerai-tailor` | Constrained-diff resume edits, cover letter drafting | Rendering to DOCX/PDF |
| `careerai-render` | Tera → Markdown → `pandoc` subprocess | Network |
| `careerai-submit` | Submitters per source behind a `Submitter` trait (HTTP, browser, email) + dry-run wrapper | Discovery, matching |
| `careerai-scheduler` | Daemon, cron wiring, graceful shutdown | Submission logic |
| `careerai-cli` | `clap` parsing, dispatch into other crates, human-readable output | Any logic beyond dispatch |

All adapters plug in through two traits — `Source` (discovery) and `Submitter` (apply). Adding a new job board means implementing `Source`; adding a new apply channel means implementing `Submitter`. Do not special-case individual sources/submitters in `careerai-core`.

**LLM safety invariant (non-negotiable):** The resume tailoring step emits a **constrained JSON diff** that can only reorder or rewrite existing bullets from the profile. It cannot invent new experience, titles, dates, or employers. Schema validation in `careerai-tailor/src/diff.rs` rejects anything outside that grammar. When touching this module, keep the validator strict and add tests for any new diff op.

**Submit safety invariant:** `auto_submit` defaults to `false`. In dry-run mode, submitters must never issue a network write — they screenshot + log a `would_submit` event instead. Per-source `submit_enabled` gates must be honored even with `--auto-submit`. Integration tests assert both of these via `wiremock` expectations and log capture; don't bypass them.

**Rate limiting:** All outbound submissions go through `governor` token-buckets defined per source in config. Quiet hours and daily caps are enforced at the boundary, not in the submitter. New submitters must acquire a permit before any network call.

## Configuration

All tunables (domains, locations, LLM models, rate caps, cron cadences, score threshold) live in `config/default.yaml` and can be overridden via `config/local.yaml` or env vars (via the `config` crate's layered loader). Never hardcode thresholds or cadences inside crates — read them from `CoreConfig` which is loaded once at startup.

## Credentials

OS keychain via the `keyring` crate is the primary store (LinkedIn cookies, Anthropic/OpenAI keys, SMTP creds). `.env` is the fallback for CI-like environments. Secrets must never hit logs — `tracing` has a redaction filter; there is a regex-based test in `careerai-submit` asserting captured events contain no known-secret shapes.

## TDD is the default for this project (MANDATORY)

The Iron Law: **NO PRODUCTION CODE WITHOUT A FAILING TEST FIRST.** This applies
to every behavior change in this repo — features, bug fixes, refactors that
touch behavior. The full protocol lives in the `superpowers:test-driven-development`
skill; load it via the `Skill` tool at the start of any non-trivial work.

Project-specific application:

- **RED**: write the failing test as the first commit-worthy artifact. For
  binaries, that often means an integration test in `crates/<crate>/tests/<name>_it.rs`
  using `env!("CARGO_BIN_EXE_<bin-name>")` to spawn the actual binary. For
  pure functions, use a `#[cfg(test)]` module next to the code.
- **Verify RED** by running `cargo test -p <crate> --test <name_it>` (or
  the targeted unit test path) and pasting the failure into the conversation.
  If the test passes immediately or fails for the wrong reason, fix it
  before writing implementation.
- **GREEN**: minimal change to make the test pass. Don't add unrelated
  cleanup, don't add fields "for the future", don't refactor adjacent code.
- **Verify GREEN** with the same targeted command. Then run the affected
  crate's full suite (`cargo test -p <crate>`) before pushing — clippy +
  fmt + the broader workspace can wait for CI per `ci-trust-when-green`.
- **REFACTOR** under the green test if useful; keep the test green.

Bug fixes always start with a regression test that fails on the buggy
code and passes on the fix. This is non-negotiable in this repo because
the LLM tailor + submit modules have safety invariants that depend on
the test suite catching drift (`careerai-tailor/src/diff.rs` constrained
grammar; `careerai-submit` dry-run + per-source gate).

Exceptions (still ask first): throwaway prototypes, scaffolding that
will be deleted in the same PR, generated code, configuration-only
changes. "Just one line" / "trivially safe" / "I'll add the test after"
are not exceptions — they are red flags.

## Testing layers

Match the layer to what you're testing:

1. **Unit** — pure functions (parsing, filters, rank math, diff apply). No I/O. Fast.
2. **Integration** (`tests/*_it.rs`) — real SQLite (tempfile), `wiremock` for HTTP, `MockLLM` stub for `rig`. Validates state-machine transitions end-to-end.
3. **Golden / snapshot** — `insta` for filter outputs; render-artifact tests extract text from produced DOCX/PDF via `pdf-extract` + `docx-rs` and diff against snapshots.
4. **Browser** — `chromiumoxide` driven against captured LinkedIn/Indeed HTML served by a local `tiny-http` fixture server. CI never hits real LinkedIn/Indeed. The dashboard has live-browser E2E tests (`crates/careerai-dashboard/tests/browser_it.rs`) that boot the server in-process and drive a real headless Chromium (click through the config tab, preview the generated config, mobile-overflow check). Install the pinned browser with `scripts/fetch-chromium.sh` (or set `CAREERAI_CHROMIUM`); tests skip when no browser is found.

`INSTA_UPDATE=always cargo test` is how you accept intentional snapshot changes — always review the diff before accepting.

## Conventions

- Edition 2021, MSRV 1.78. Pin via `rust-toolchain.toml`.
- Errors: `thiserror` in library crates, `anyhow`/`color-eyre` only in `careerai-cli`.
- Async: `tokio` everywhere. No `async-std` or `smol` mixing.
- Logging: structured via `tracing`; no `println!` outside the CLI crate's user-facing output.
- SQL: `sqlx` with compile-time-checked queries (`query!`/`query_as!`). Set `DATABASE_URL` and run `cargo sqlx prepare` before commits that change queries.
- Workspace-level lints in `.cargo/config.toml`; clippy must be clean with `-D warnings`.
- Build the release profile with every change: run `cargo build --release --workspace` alongside `cargo build`, `cargo test`, and `cargo clippy`. Release-only `cfg` branches, `debug_assertions`, and overflow-check differences must not ship unverified.

## Global rules inherited from `~/.claude/CLAUDE.md`

- Commits: 50/72 rule, generic subject, no "Claude"/"AI", always `git commit -s`, SSH-signed, hook blocks unsigned push.
- No direct commits to `main`. Branch names: `feat/`, `fix/`, `chore/`, `docs/`, `issues/<n>-<slug>`. Check `git branch -r | grep -i <topic>` / `gh pr list --search <topic>` before branching.
- Post-merge: delete the local branch.
- Semgrep baseline: `.claude/.semgrep-baseline.json` — create on first entry, re-scan after ≥20 changed files or dep bumps or auth/crypto/SQL/deserialization changes.

## Token discipline (MANDATORY — Serena + context-mode)

The full rule lives in `~/.claude/CLAUDE.md` ("Code Reading + Editing" + "Shell output" + "Subagent propagation" sections). It applies in this project without exception. Project-specific tightening:

**Before reading any Rust source file, decide:**
1. Known function/struct/method/trait/impl → `mcp__plugin_serena_serena__find_symbol(name_path, relative_path, include_body=true)`. NEVER `Read` the file.
2. Surveying a 500+ LOC file (e.g., `careerai-pipeline/src/lib.rs` is 600+ LOC, `careerai-submit/src/rate_limiter.rs` is 500+) → `get_symbols_overview` first, then drill in. NEVER `Read` the whole file.
3. "Who calls `discover_one`/`apply_all`/`Scheduler::from_config`/etc." → `find_referencing_symbols`. NEVER `grep -rn`.
4. File ≤ 200 LOC (e.g., `linkedin_selectors.rs`, `error.rs` modules) → `Read` with `offset`+`limit` is fine.

**Before running any cargo command, decide:**
1. `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo audit`, `cargo deny check` → `mcp__plugin_context-mode_context-mode__ctx_execute(language="shell", code="...", intent="test failures")`. The full output is indexed; you get a summary or matched sections.
2. `cargo test -p single-crate --test single_test` (typically <50 lines output) → direct `Bash` with `tail -20` is fine.
3. `git diff origin/main` (large diffs > 500 LOC) → `ctx_execute` with intent. Never let raw diff hunks flood context.

**Forbidden in this project:**
- `Read` of `careerai-pipeline/src/lib.rs`, `careerai-submit/src/rate_limiter.rs`, `careerai-submit/src/linkedin.rs`, `careerai-scheduler/src/lib.rs`, `careerai-cli/src/pipeline.rs` (now relocated), `careerai-core/src/config.rs` without `offset`+`limit` AND a specific symbol target. These are all 400+ LOC; use Serena.
- `grep -r` to find call sites of any pipeline entry point, scheduler hook, or submit trait method.
- Direct `Bash` for `cargo test --workspace` — use `ctx_execute`.

**Subagent prompts in this project MUST include:**

```
## Token discipline (inherited from career-ai CLAUDE.md — non-negotiable)
- Use mcp__plugin_serena_serena__find_symbol / get_symbols_overview / find_referencing_symbols / search_for_pattern for code reading. NEVER full-file Read for known-symbol queries.
- Use mcp__plugin_serena_serena__replace_symbol_body / insert_after_symbol for symbol-scoped edits.
- Use mcp__plugin_context-mode_context-mode__ctx_execute for cargo/build/test/find/grep commands. Direct Bash only for bounded one-liners.
- Pre-load these tool schemas via ToolSearch at the start of your task. Cost ~2 KB; saves >10x on the first 5 reads.
- Files in this project that are FORBIDDEN to Read in full (use Serena symbol-level tools): careerai-pipeline/src/lib.rs, careerai-submit/src/rate_limiter.rs, careerai-submit/src/linkedin.rs, careerai-scheduler/src/lib.rs, careerai-core/src/config.rs.
```

If a subagent's tool-call report shows full-file Reads of forbidden files, raw `grep -r` for symbol queries, or unbounded cargo dumps in its context — its work is suspect. Re-run with a tightened prompt, don't paper over.

## Gotchas

- The `.remember/logs/` directory is required by a hookify PostToolUse hook; do not delete it.
- LinkedIn + Indeed auto-apply violates their ToS — this is a documented, accepted trade-off with mitigations (dry-run default, rate caps, stealth, kill-switches). Keep the safety gates intact when touching `careerai-submit`.
- Anthropic prompt caching is used on the master-profile block to cut token cost — don't inline the profile into per-call prompts without caching.
- `chromiumoxide` stealth relies on a checked-in `stealth-v2.js` pinned by SHA. When Chrome CDP changes break it, update the script and its hash in the same commit as the selector fixes.
