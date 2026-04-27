# career-ai (Claude Code plugin)

A Claude Code plugin that exposes [career-ai](https://github.com/justdoGIT/career-ai)
— a local, single-user job-discovery + resume-tailoring + auto-apply
pipeline — through slash commands, skills, and subagents.

The plugin is the surface; the substance lives in the local Rust binary
and the local `careerai-mcp` server it ships with.

## Install

```
claude plugins install github.com/justdoGIT/career-ai
```

(Plugin name registered in [`plugin.json`](./plugin.json) is
**`career-ai`** — match this exactly when uninstalling or referencing in
other tools.)

## Prerequisites

- **`pandoc`** on PATH (resume rendering). Debian/Ubuntu:
  `sudo apt install pandoc`. macOS: `brew install pandoc`. Arch:
  `sudo pacman -S pandoc`.
- **Anthropic API key** — either `ANTHROPIC_API_KEY` env var or a keyring
  entry (`secret-tool` on Linux, `security` on macOS). Used for LLM
  resume tailoring + the LLM profile extractor.
- **`careerai` CLI + `careerai-mcp` MCP server** built and on PATH.
  After cloning:
  ```
  cd career-ai
  cargo install --path crates/careerai-cli      # `careerai` binary
  cargo install --path crates/careerai-mcp      # `careerai-mcp` MCP server (Phase 2)
  ```
- **Optional: RapidAPI key** (`LINKEDIN_RAPIDAPI_KEY`) for ToS-clean
  LinkedIn discovery via the `linkedin-jobs` MCP server.

## Quick start

```
/career:setup                          # first-run wizard: imports resume + LinkedIn export
/career:discover                       # pull listings from configured sources
/career:status                         # see pipeline counts + cookie expiries
/career:tailor <listing-id>            # tailor + render for one listing
/career:apply  <app-id>                # DRY-RUN by default — safe to run
/career:apply  <app-id> --live         # live submit; requires explicit ToS-risk confirmation
/career:inspect <app-id>               # full row + state history + artifact paths
/career:digest --since 7d              # weekly summary
```

## Safety

career-ai is built around three non-negotiable safety gates. The plugin
does not relax any of them.

1. **Dry-run by default.** `/career:apply` calls
   `careerai_apply(dry_run: true)` unless the user passes `--live`. Dry-run
   never issues a network write — it screenshots the populated form and
   logs a `would_submit` event.
2. **ToS-risk confirmation for LinkedIn / Indeed.** Live submission to
   LinkedIn or Indeed requires the user to type the literal phrase
   `I_UNDERSTAND_TOS_RISK`. No paraphrase. The friction is the feature.
   See [`skills/dry-run-apply/SKILL.md`](./skills/dry-run-apply/SKILL.md).
3. **Constrained-diff resume tailoring.** The LLM tailoring step emits a
   JSON diff that can only reorder or rewrite existing bullets — it
   cannot fabricate new experience, titles, dates, or employers. The
   validator in `careerai-tailor/src/diff.rs` is strict and rejects
   anything outside the grammar. See
   [`skills/tailor-resume/SKILL.md`](./skills/tailor-resume/SKILL.md).

Per-source rate caps (`governor` token-buckets), daily caps, and
quiet-hours windows are enforced at the boundary in `careerai-submit` —
the plugin does not bypass them.

## Architecture

```
+----------------------+        +---------------------------+
|  Claude Code         |        |  Local Rust workspace     |
|  + career-ai plugin  | -----> |  career-ai (CLI + daemon) |
|                      |  MCP   |                           |
|  /career:* commands  | <----> |  careerai-mcp server      |
|  skills + agents     |        |  -> SQLite pipeline DB    |
+----------------------+        |  -> pandoc render         |
           |                    |  -> ATS/LinkedIn/Indeed   |
           | MCP                +---------------------------+
           v
+----------------------+
|  Community MCPs      |
|  - linkedin-jobs     |  (RapidAPI; ToS-clean; default)
|  - linkedin-browser  |  (scraper; disabled by default)
+----------------------+
```

The plugin's `.mcp.json` registers the local `careerai-mcp` server plus
optional community LinkedIn MCPs. The scraper variant is disabled by
default; flip `disabled: false` in `.mcp.json` to opt in (and read the
ToS section above first).

## Layout

```
.claude-plugin/
├── plugin.json               # plugin manifest
├── README.md                 # this file
├── .mcp.json                 # MCP server registrations
├── commands/                 # slash commands (/career:*)
│   ├── setup.md
│   ├── discover.md
│   ├── tailor.md
│   ├── apply.md
│   ├── status.md
│   ├── digest.md
│   └── inspect.md
├── skills/                   # LLM guidance documents
│   ├── ingest-profile/SKILL.md
│   ├── tailor-resume/SKILL.md
│   └── dry-run-apply/SKILL.md
└── agents/                   # subagents callable via Task tool
    ├── job-hunter.md
    ├── resume-tailor.md
    └── application-reviewer.md
```

## Further reading

- Project root: [README.md](../README.md) — full project overview, build,
  and milestone status.
- Architecture + invariants: [CLAUDE.md](../CLAUDE.md) — crate
  boundaries, safety invariants, testing layers, and conventions.
