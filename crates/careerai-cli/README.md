# careerai-cli

The `careerai` binary — `clap`-based CLI that dispatches into the
workspace crates. Owns nothing beyond argument parsing, dispatch
glue, and human-readable output formatting.

## Boundary

| Owns | Never does |
|---|---|
| `clap::Parser` `Cli` + `Command` enums | Business logic beyond dispatch |
| Subcommand handlers in `commands/*.rs` | Direct DB writes (goes through `careerai-db`) |
| Top-level `main()` + `init_tracing` + `load_cfg` | Network I/O (goes through source / submit crates) |
| Exit-code mapping for typed errors (per-stage) | LLM calls (goes through `careerai-llm`) |

## Key entry points

* `careerai init` — scaffold `config/`, `profile/`, `.env`
* `careerai discover [--source X,Y]` — pull listings from sources
* `careerai match [--tune]` — score discovered listings
* `careerai tailor <listing-id>` — LLM-tailor a resume
* `careerai render <application-id>` — emit DOCX + PDF via pandoc
* `careerai apply [--all] [--auto-submit]` — submit (dry-run by default)
* `careerai daemon` — run the cron-driven scheduler in the foreground
* `careerai status serve [--port N]` — read-only HTTP dashboard
* `careerai service install/status/uninstall` — systemd user-service mgmt
* `careerai profile import/show/validate` — profile ingestion
* `careerai sources sync [--apply]` — auto-discover companies from seed
* `careerai mcp probe`, `careerai llm probe`, `careerai notify test` — diagnostics

Run with `--help` for full usage.

## Module layout

```
src/
  main.rs            entry, clap structs, dispatch
  commands/          per-subcommand handlers (one file each, < 300 LOC)
  cookies.rs         keyring cookie management
  digest.rs          daily summary printer
  review.rs          interactive LinkedIn draft review
  service.rs         systemd unit install/uninstall
  sources_sync.rs    company-list merger
  status.rs          dashboard server bootstrap
```

## Features

| Feature | Effect |
|---|---|
| `live-llm-cli` (default) | Build the `claude` CLI subprocess backend |
| `live-llm-api` | Build the rig-core Anthropic API backend |
| `live-llm` | Both backends (umbrella alias) |
| `browser` | Pull in `careerai-submit/browser` + `careerai-sources/browser` for LinkedIn auto-apply |

Default `cargo install` targets `live-llm-cli` so Claude Code subscribers
get the CLI backend out of the box without an API key.

## Tests

```bash
cargo test -p careerai-cli                  # all
cargo test -p careerai-cli sources_sync::   # one module
```

Inline arg-parser regression tests live in `main.rs`'s `mod tests`
block (clap behaviour around `value_delimiter`, `--llm-backend`
override, anyhow chain formatting). Per-command tests live with
their handlers in `commands/`.

## Operator docs

* [`docs/SECURITY.md`](../../docs/SECURITY.md) — threat model + safety invariants
* [`docs/SERVICE.md`](../../docs/SERVICE.md) — systemd-user walkthrough
* [`docs/NOTIFICATIONS.md`](../../docs/NOTIFICATIONS.md) — channel setup
