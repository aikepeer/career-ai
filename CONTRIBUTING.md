# Contributing to career-ai

Thanks for your interest. This is a single-user-focused project, but
contributions that add adapters, harden safety gates, or improve docs are
welcome.

## Ground rules (before you open a PR)

- **No direct commits to `main`.** Branch off and open a PR.
- **Branch names:** `feat/`, `fix/`, `chore/`, `docs/`, `issues/<n>-<slug>`.
- **Commits:**
  - 50/72 rule (≤50 char subject, ≤72 char body wrap).
  - Generic subject — no "Claude" / "AI" mentions.
  - SSH-signed (`commit.gpgsign = true`, `gpg.format = ssh`). The
    project doesn't ship a pre-push hook; signing is enforced via the
    maintainer's local git config and reviewed at merge time.
  - Sign-off required: `git commit -s` (DCO). There is no automated
    DCO CI check yet — sign-offs are validated at review.
- **CI must be green** before review: `rustfmt`, `clippy -D warnings`,
  tests, `cargo audit`. The `clippy` gate is strict — fix warnings, do
  not `#[allow]` them away unless you can justify it in a comment.

## Token discipline (mandatory for AI-assisted contributions)

If you're using Claude Code or similar AI tooling, the project's
[CLAUDE.md](./CLAUDE.md) defines a non-negotiable code-reading + shell-
output protocol (Serena symbol-level tools instead of full-file Read,
context-mode for unbounded shell output). Subagent prompts must propagate
the snippet from CLAUDE.md verbatim. PRs that show full-file dumps in
their commit history or use `grep -r` for symbol queries will be sent
back.

## Test layers

| Layer | Where | When to use |
|---|---|---|
| Unit | inside `crates/<crate>/src/` | Pure functions: parsing, filters, rank math, diff apply. |
| Integration | `crates/<crate>/tests/*_it.rs` | Real SQLite + `wiremock` HTTP + `MockLLM`. State-machine end-to-end. |
| Snapshot | `insta` + the in-tree fixtures | Render artifacts, filter outputs, prompt bodies. |
| Browser | `chromiumoxide` + `tiny-http` against captured HTML | LinkedIn / Indeed selector regression. CI never hits real sites. |

`cargo nextest run --workspace` is the preferred runner if you have it
installed (faster, better output); the project does not ship a
`nextest.toml` so default settings apply. `cargo test --workspace`
works equivalently without `nextest`.

`INSTA_UPDATE=always cargo test` accepts intentional snapshot changes —
review the diff before accepting.

### LLM backend matrix

The LLM gateway has two real backends; a non-trivial change to
`careerai-llm` must be exercised against both feature combinations:

| Build | What it pulls in | Use |
|---|---|---|
| `cargo build` (default features) | `live-llm-cli` only — `claude` CLI subprocess driver | What Claude Code subscribers ship with. Smaller binary. |
| `cargo build --features live-llm-api` | `live-llm-cli` + `live-llm-api` (rig-core + reqwest) | API path. Required for hosts without Claude Code. |
| `cargo build --features live-llm` | umbrella alias: both | Back-compat. |

Tests gated on a real `claude` binary live behind a future
`live-claude-cli-real` feature so CI doesn't shell out. Stub-binary
unit tests in `crates/careerai-llm/src/claude_cli.rs` exercise the
full driver through a tempdir-installed shell script.

## Adding a new job source

1. Create `crates/careerai-sources/src/<source>.rs`.
2. Implement the `Source` trait from `crates/careerai-sources/src/base.rs`
   (see existing `greenhouse.rs`, `lever.rs`, `naukri.rs`, `remoteok.rs`,
   `remotive.rs`).
3. Register the new variant in the source factory.
4. Add a config entry to `config/default.yaml` (commented-out, default
   `enabled: false` if the source has any ToS gray area).
5. Add at least one snapshot test against a real-shape JSON fixture.

## Adding a new submitter

1. Create `crates/careerai-submit/src/<submitter>.rs`.
2. Implement the `Submitter` trait. **Honor `auto_submit: false` in
   dry-run.** Submitters that issue a network write under dry-run
   will be reverted on review.
3. Acquire a `governor` permit before any network call.
4. Integration test must assert `would_submit` events on dry-run AND
   refuse live submission without the per-source `submit_enabled` gate.

## LLM safety invariants

The constrained-diff validator in
`crates/careerai-tailor/src/diff.rs` is strict by design. Resume edits
can only reorder or rewrite EXISTING bullets; new bullets, fabricated
employers, or invented dates are rejected at parse time. When touching
this module, keep the validator strict and add tests for any new diff
op.

The `careerai-mcp::apply` tool refuses live submissions without
`confirm: "I_UNDERSTAND_TOS_RISK"`. Keep that gate intact.

## Security checks

`semgrep scan --config auto` is part of the baseline workflow; the
project will track findings in `.claude/.semgrep-baseline.json` once a
clean baseline is captured. Re-run `semgrep scan --config auto` and
refresh the baseline after:
- ≥20 changed files in one PR
- Dependency manifest changes (`Cargo.toml`, lockfile bumps)
- Anything touching auth / crypto / SQL / deserialization

## Release process

`cargo-dist` is configured at `[workspace.metadata.dist]` in the root
`Cargo.toml`. Releases are tagged by the project owner; the GitHub
Actions workflow auto-publishes prebuilt binaries to the Releases page.
Don't bump `version` in a feature PR.
