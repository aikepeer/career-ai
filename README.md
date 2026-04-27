# career-ai

Automated job discovery, resume tailoring, and auto-apply for a single user —
runs as a local daemon and as a Claude Code plugin.

**Status (2026-04-27):** M0–M6 shipped. The pipeline state machine, four
ATS adapters (Greenhouse, Lever, Remotive, RemoteOK), LinkedIn submitter,
LLM-backed resume importer, MCP server, and Claude Code plugin shell are
all merged on `main`. See [`CHANGELOG.md`](./CHANGELOG.md) for milestone
detail.

## What it does

Finds jobs across ATS APIs (Greenhouse, Lever, Remotive, RemoteOK, plus any
MCP-exposed source — see Phase 4), matches them against your profile,
tailors a resume + cover letter per job description via Claude, and submits
applications with a hard dry-run gate enabled by default. Niche focus:
AI/ML + LLM apps and embedded platforms / robotics. Remote-first with
Delhi-NCR fallback.

## Install — Claude Code plugin (recommended)

```bash
# 1. Add the plugin
claude plugins install github.com/justdoGIT/career-ai

# 2. Install the local MCP server binary that the plugin's slash commands call
cargo install --git https://github.com/justdoGIT/career-ai careerai-mcp careerai-cli

# 3. Inside Claude Code
/career:setup        # walks pandoc check + key prompts + profile import
/career:discover     # pulls fresh listings
/career:status       # shortlist + pipeline overview
/career:tailor <id>  # constrained-diff resume + cover-letter for one listing
/career:apply <id>   # DRY-RUN by default; --live needs an explicit confirm
```

The plugin also registers two community LinkedIn MCPs in `.mcp.json`:
`linkedin-jobs` (RapidAPI, ToS-clean — default) and `linkedin-browser`
(scraper, opt-in, violates LinkedIn ToS §8.2).

## Install — CLI / daemon

```bash
cargo install --git https://github.com/justdoGIT/career-ai careerai-cli

careerai init
careerai profile import resume.pdf LinkedIn-Export.zip   # +--use-llm with --features live-llm
careerai discover --source greenhouse,lever
careerai match
careerai shortlist show --limit 20
careerai tailor <listing-id>
careerai render <application-id>
careerai apply <application-id>                          # dry-run
careerai daemon                                          # long-running scheduler
careerai digest --since 24h
```

For LLM features:

```bash
cargo install --git https://github.com/justdoGIT/career-ai \
    careerai-cli --features live-llm
export ANTHROPIC_API_KEY=...      # or store in OS keyring
```

## Architecture

```
                ┌──────── Claude Code plugin ─────────┐
                │  commands · skills · agents         │
                │  registers careerai + linkedin MCPs │
                └──────────────────┬──────────────────┘
                                   │ stdio (rmcp)
                ┌──────── careerai-mcp server ────────┐
                │  8 tools · resources · templates    │
                └──────────────────┬──────────────────┘
                                   │
   ┌──── Source trait ────┐ pipeline state machine ┌── Submitter trait ──┐
   │ greenhouse · lever · │ discovered → shortlist │ ats-http · linkedin │
   │ remotive · remoteok ·│ → tailored → rendered →│ · naukri · email    │
   │ mcp-jobs (any MCP)   │ → submitted → responded│   (dry-run default) │
   └──────────────────────┘                        └─────────────────────┘
                                   │
                  SQLite · keyring · governor rate-limits
```

Crate boundaries are deliberate. See [CLAUDE.md](./CLAUDE.md) for the
full architecture, invariants, testing layers, and contribution rules.

## Runtime dependencies

- `pandoc` on PATH for DOCX + PDF rendering.
- OS keychain (Linux Secret Service, macOS Keychain) for cookies and API keys.
- Anthropic API key in env (`ANTHROPIC_API_KEY`) or keyring for LLM features.
- Optional: RapidAPI key for the `linkedin-jobs` MCP server (free tier exists).

## Build from source

```bash
cargo build --release --workspace
cargo nextest run --workspace        # or: cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Prebuilt binaries for Linux / macOS / Windows are published to
[GitHub Releases](https://github.com/justdoGIT/career-ai/releases) once
tagged.

## Safety + legal

LinkedIn and Indeed auto-apply violate their Terms of Service. This tool
ships dry-run by default, enforces per-source rate caps via `governor`
token-buckets, uses stealth browser techniques where applicable, and
requires explicit per-source `submit_enabled: true` plus an
`I_UNDERSTAND_TOS_RISK` confirmation before any real submission. See
[CLAUDE.md](./CLAUDE.md) for the full risk register.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md). Commits are SSH-signed and
sign-off-required (`git commit -s`).
