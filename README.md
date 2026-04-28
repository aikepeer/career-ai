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

For LinkedIn discovery, the supported path is the native
`linkedin_browser` source (PR #21). It drives a stealth Chromium
session, reuses the M5 `li_at` cookie + stealth-v2.js infrastructure,
and integrates with the daemon's cron + `governor` rate limiter. It
defaults to `enabled: false`; flip `sources.linkedin_browser.enabled`
in `config/local.yaml` to opt in (see "Sources" below). The plugin
also registers two community LinkedIn MCPs in `.mcp.json`, both
disabled by default and kept only as alternative discovery surfaces
for power users — `linkedin-jobs` (RapidAPI-backed) and
`linkedin-browser` (community scraper). The native source is the
recommended path. Generic MCP-exposed sources are supported through
the separate `mcp_jobs` adapter (PR #19).

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

## What works today on a Max plan (no API key)

The LLM-backed steps (`tailor`, `cover-letter`, `parse-resume`) drive
the locally installed `claude` CLI by default. If you have a Claude
Max plan and have run `claude login` once, no `ANTHROPIC_API_KEY` is
needed. The end-to-end run after `cargo install` is:

```bash
# 0. One-time prereqs: pandoc on PATH, `claude login` succeeded.
careerai init                                        # create config + DB
careerai profile import resume.pdf LinkedIn.zip      # +--use-llm to route through claude
careerai sources sync                                # preview ~80 known-good Greenhouse /
                                                     # Lever / Ashby slugs filtered against
                                                     # your `domains:` keywords
careerai sources sync --apply                        # merge into config/local.yaml (preserves
                                                     # other user keys; manual additions kept)
careerai discover --source greenhouse,lever,ashby,remotive,remoteok
careerai match                                       # filter + rank against profile
careerai shortlist show --limit 20                   # pick a listing-id
careerai tailor <listing-id>                         # constrained-diff resume + cover letter
careerai render <application-id>                     # DOCX + PDF via pandoc
careerai apply <application-id>                      # DRY-RUN; see /career:apply walkthrough
careerai applied --limit 20                          # confirm recent rows + cadence
careerai digest --since 24h                          # whole-pipeline rollup
```

Live submission is gated three ways: dry-run is the default, you
must flip `submit.per_source.<source>.enabled: true` for that one
source in `config/local.yaml`, and LinkedIn / Indeed additionally
require the literal `I_UNDERSTAND_TOS_RISK` confirmation. See the
[`/career:apply`](.claude-plugin/commands/apply.md) walkthrough.

## Known gaps

The pipeline is end-to-end usable for ATS-API sources today. These
are the open items:

- **LinkedIn discovery** — first-class adapter is the `mcp_jobs`
  source (PR #19, merged), which discovers via any MCP server that
  advertises a known job-search tool name. The bundled
  `linkedin-jobs` MCP (`Rom7699/linkedin-jobs-mcp-server`) currently
  ships as `python main.py` with no script entry — manual enable
  steps are in `.mcp.json`. The `linkedin-browser` MCP is a
  ToS-violating scraper and stays opt-in. A native chromiumoxide
  LinkedIn discovery adapter is in flight separately.
- **Response tracking (M7, planned)** — `submitted` applications do
  not yet roll up into a `responded` state automatically. Until M7
  lands, watch your inbox; `careerai applied --since <window>` shows
  what was submitted, but not whether the employer replied.
- **Semantic matching upgrade (planned)** — current ranking is
  `fastembed` cosine over JD + profile text plus filter-config
  predicates. Roadmap items: per-skill weight tuning from response
  outcomes, per-source priors, and a learned threshold for
  shortlist cutoff.


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

## Sources

| Source              | Default   | Notes                                                                                                       |
|---------------------|-----------|-------------------------------------------------------------------------------------------------------------|
| `greenhouse`        | enabled   | Public ATS API, one entry per company slug.                                                                 |
| `lever`             | enabled   | Public ATS API, one entry per company slug.                                                                 |
| `remotive`          | enabled   | Public job feed.                                                                                            |
| `remoteok`          | enabled   | Public job feed.                                                                                            |
| `naukri`            | disabled  | Undocumented `jobapi/v3/search`; opt-in.                                                                    |
| `linkedin_browser`  | disabled  | Native browser-driven discovery (M2/PR #21). Violates LinkedIn UA §8.2 — opt-in only. Requires `--features browser` and a `li_at` cookie stored in the OS keychain. |
| `mcp_jobs`          | disabled  | Generic MCP-server adapter for community job-search MCPs (PR #19). Per-server opt-in.                       |

To enable the native LinkedIn source, edit `config/local.yaml`:

```yaml
sources:
  linkedin_browser:
    enabled: true
    keywords: "AI engineer remote"
    location: "Worldwide"
    filters:
      remote: true
      posted_within_days: 7
      experience_level: ["mid", "senior"]
    max_pages: 3
    rate_per_minute: 2
```

Store the cookie once: `careerai cookies set linkedin` (writes to the OS
keychain, service `career-ai`, user `linkedin/li_at`). Then rebuild with
`cargo install --git ... careerai-cli --features browser`. **LinkedIn
auto-discovery violates LinkedIn User Agreement §8.2.** This tool ships
with conservative defaults (3 pages × 25 cards max per tick, 2 calls/min,
1.5–3.5s randomized inter-page jitter), but the operator carries the
ToS risk on opting in.

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

## Notifications

career-ai pings you outside the CLI when something needs human
attention — cookie expiring within 48h, discovery source unreachable,
high-score match worth applying same-day, rate-limit window
exhausted, etc. Configure any subset of these channels under
`notify.channels` in `config/local.yaml`:

| Channel  | Cost | Setup                                                          |
|----------|------|----------------------------------------------------------------|
| Slack    | free | Incoming webhook + `webhook_url_env: CAREERAI_SLACK_WEBHOOK`   |
| Telegram | free | Bot token in OS keyring + numeric `chat_id`                    |
| Email    | free | SMTP relay; password in OS keyring                             |
| ntfy.sh  | free | Random topic name; phone push via the ntfy app                 |

All secrets come from env vars or the OS keyring — inline secrets
in YAML are intentionally rejected. Run `careerai notify test`
after editing config to verify each channel end-to-end. See
[docs/NOTIFICATIONS.md](./docs/NOTIFICATIONS.md) for full setup
walkthroughs and the event reference (what gets sent when).

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
