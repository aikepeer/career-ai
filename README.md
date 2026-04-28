# career-ai

Automated job discovery, resume tailoring, and auto-apply for a single user —
runs as a local daemon and as a Claude Code plugin.

**Status (2026-04-27):** M0–M6 shipped. The pipeline state machine, five
discovery adapters (Greenhouse, Lever, Remotive, RemoteOK, Naukri),
LinkedIn submitter, LLM-backed resume importer, MCP server, and Claude
Code plugin shell are all merged on `main`. See
[`CHANGELOG.md`](./CHANGELOG.md) for milestone detail.

## What it does

Finds jobs across ATSes and job boards (Greenhouse, Lever, Remotive,
RemoteOK, Naukri; plus any MCP-exposed source via the upcoming `mcp_jobs`
adapter — see PR #19), matches them against your profile, tailors a
resume + cover letter per job description via Claude, and submits
applications with a hard dry-run gate enabled by default. Niche focus:
AI/ML + LLM apps and embedded platforms / robotics. Remote-first with
Delhi-NCR fallback.

## Install — Claude Code plugin (recommended)

**Claude Code (Max/Pro) subscriber? No API key needed.** career-ai's
default LLM backend is the `claude` CLI subprocess driver — every
inference call bills against your existing Claude Code session.

```bash
# 1. Add the plugin
claude plugins install github.com/justdoGIT/career-ai

# 2. Install the local MCP server binary that the plugin's slash commands call
cargo install --git https://github.com/justdoGIT/career-ai careerai-mcp careerai-cli

# 3. Inside Claude Code
/career:setup        # walks pandoc check + LLM probe + profile import
/career:discover     # pulls fresh listings
/career:status       # shortlist + pipeline overview
/career:tailor <id>  # constrained-diff resume + cover-letter for one listing
/career:apply <id>   # DRY-RUN by default; --live needs an explicit confirm
```

The plugin also registers two community LinkedIn MCPs in `.mcp.json`,
both disabled by default. `linkedin-jobs` (RapidAPI-backed, ToS-clean)
points at `Rom7699/linkedin-jobs-mcp-server`, which currently ships as
`python main.py` with no script entry — see the comment in `.mcp.json`
for manual-enable steps. `linkedin-browser` (`adhikasp/mcp-linkedin`,
properly packaged) is a scraper that violates LinkedIn ToS §8.2 and
stays opt-in. The first-class path for community MCP discovery sources
is the upcoming `mcp_jobs` adapter (PR #19).

## Install — CLI / daemon

```bash
cargo install --git https://github.com/justdoGIT/career-ai careerai-cli

careerai init
careerai llm probe                                        # confirm a backend is reachable
careerai profile import resume.pdf LinkedIn-Export.zip    # auto-uses LLM when reachable
careerai discover --source greenhouse,lever
careerai match
careerai shortlist show --limit 20
careerai tailor <listing-id>
careerai render <application-id>
careerai apply <application-id>                           # dry-run
careerai daemon                                           # long-running scheduler
careerai digest --since 24h
```

The default install ships with the `claude` CLI subprocess backend
enabled — Claude Code subscribers don't need an API key. To also
build the rig-core / Anthropic API path (for hosts without Claude
Code, or to force `--llm-backend=api`):

```bash
cargo install --git https://github.com/justdoGIT/career-ai \
    careerai-cli --features live-llm-api
export ANTHROPIC_API_KEY=...      # or store in OS keyring
```

Override the backend per command with `--llm-backend=auto|claude-cli|api`,
or persistently via `llm.backend` in `config/local.yaml`.

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
   │ remotive · remoteok ·│ → tailored → rendered →│ · naukri            │
   │ naukri               │ → submitted → responded│   (dry-run default) │
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
