# careerai-mcp

MCP (Model Context Protocol) server that exposes the career-ai pipeline as
tool calls so an LLM client like Claude Code can drive discovery, matching,
tailoring, rendering, and (gated) auto-apply.

## What it gives you

| Tool                       | Purpose                                                                              |
|----------------------------|--------------------------------------------------------------------------------------|
| `careerai_profile_status`  | Inspect `profile/profile.yaml` (path, parse + validate, last-modified, issues).      |
| `careerai_discover`        | Run discovery against one or more sources; persists new listings.                    |
| `careerai_shortlist`       | List shortlisted listings (filterable by `limit` / `min_score`).                     |
| `careerai_tailor`          | Tailor resume + cover letter for a listing (constrained-diff over the master).       |
| `careerai_render`          | Render a tailored application to DOCX + PDF via pandoc.                              |
| `careerai_apply`           | Submit a rendered application. Dry-run by default; real submit needs a confirm token.|
| `careerai_inspect`         | Read-only event history + on-disk artifacts for an application.                      |
| `careerai_digest`          | Daily/weekly digest with a pre-formatted markdown string + raw counts.               |

Resources:

- `careerai://profile` &mdash; full contents of `profile/profile.yaml`.
- `careerai://shortlist/{date}` &mdash; current shortlist as JSON. (Date
  segment is parsed but not yet used to filter; the pipeline only stores
  the live shortlist.)
- `careerai://artifacts/{application_id}` &mdash; the artifact list
  (paths + sizes) for one application, plus its current state.

## Safety gates

- `careerai_apply` defaults to `dry_run = true`. Real submission requires
  **both** `dry_run = false` **and** `confirm =
  "I_UNDERSTAND_TOS_RISK"`. The token is a kill-switch: LinkedIn / Indeed
  auto-apply violates their ToS; passing the token is the operator's
  acknowledgement.
- Per-source `submit_enabled` gates in `config/local.yaml` are still
  honored when the confirm token is present.
- LinkedIn applications, when `submit.linkedin.interactive_only = true`
  (the default), are routed to the assist queue (state `drafted`) rather
  than submitted autonomously. Use `careerai review` to confirm them
  manually.
- Logs go to **stderr only**. Stdout is reserved for MCP JSON-RPC framing.
- Errors are typed (`McpServerError`) and surface as MCP `ErrorData`; the
  server never panics on bad input.

## Build

```bash
cargo build -p careerai-mcp --release
# binary: target/release/careerai-mcp
```

`pandoc` must be on `PATH` for `careerai_render` to succeed (same runtime
dependency as the CLI).

## Run standalone

```bash
# CWD is the project root by default; override with CAREERAI_ROOT.
CAREERAI_ROOT=/path/to/career-ai-data ./target/release/careerai-mcp
# Logs are tracing-formatted on stderr; stdout speaks JSON-RPC.
```

Tweak log verbosity with `CAREERAI_LOG` (uses `tracing-subscriber`
`EnvFilter` syntax, e.g. `CAREERAI_LOG=debug`).

## Register with Claude Code

Add to `~/.claude/.mcp.json` (or the project-local `.mcp.json`):

```json
{
  "mcpServers": {
    "careerai": {
      "command": "/absolute/path/to/career-ai/target/release/careerai-mcp",
      "args": [],
      "env": {
        "CAREERAI_ROOT": "/absolute/path/to/career-ai",
        "CAREERAI_LOG": "info"
      }
    }
  }
}
```

After Claude Code restarts, the tools appear under the `careerai` server
name. The LLM can then call them like any other tool, e.g.:

> "Discover greenhouse listings, then show me the top 5 shortlisted
> matches for AI/ML roles."

## Testing

```bash
cargo test -p careerai-mcp                # unit + integration
cargo clippy -p careerai-mcp --all-targets -- -D warnings
cargo fmt --all -- --check
```

The integration test (`tests/server_it.rs`) drives the server through an
in-process `tokio::io::duplex` to verify:

1. `initialize` succeeds and capabilities advertise tools + resources.
2. `tools/list` returns exactly the 8 tools above.
3. `careerai_profile_status` against an empty tempdir returns
   `{exists: false}` rather than erroring.
4. `careerai_apply` with `dry_run: false` and no `confirm` token is
   rejected with an `InvalidParams`-shaped error.

## Smoke test from the shell

```bash
{
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
  echo '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
  sleep 0.3
} | CAREERAI_ROOT=/tmp careerai-mcp 2>/dev/null | jq '.result.tools[]?.name'
```

Expected output (8 names):

```
"careerai_apply"
"careerai_digest"
"careerai_discover"
"careerai_inspect"
"careerai_profile_status"
"careerai_render"
"careerai_shortlist"
"careerai_tailor"
```

## SDK choice

This crate uses [`rmcp`](https://crates.io/crates/rmcp) 1.5 &mdash; the
official Rust MCP SDK from the `modelcontextprotocol` org. Selected
because:

- Trait-based tool registration (`#[tool_router]`, `#[tool]`,
  `#[tool_handler]`) keeps tool I/O schemas, descriptions, and bodies
  co-located.
- First-class stdio transport (`rmcp::transport::stdio`) is exactly
  what Claude Code expects.
- `schemars` integration auto-publishes JSON Schema for every tool's
  argument struct, so the LLM sees correct types without hand-rolled
  schema boilerplate.

## Architecture

The crate is intentionally thin &mdash; it is an *adapter*, not a feature
crate. Every tool delegates to existing pipeline entrypoints:

```
careerai_mcp::CareerAiServer
  └─ tool: careerai_discover         → careerai_pipeline::discover_all
  └─ tool: careerai_shortlist        → careerai_pipeline::shortlist_show
  └─ tool: careerai_tailor           → careerai_pipeline::tailor_one
  └─ tool: careerai_render           → careerai_pipeline::render_one
  └─ tool: careerai_apply            → careerai_pipeline::apply_one
  └─ tool: careerai_inspect          → careerai_pipeline::inspect_show
  └─ tool: careerai_digest           → careerai_pipeline::digest_summary
  └─ tool: careerai_profile_status   → careerai_profile::Profile::{from_yaml, check}
```

No business logic lives here. Adding a new tool means adding a new
pipeline entrypoint (in `careerai-pipeline`) and a thin wrapper in
`server.rs`.
