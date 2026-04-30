# careerai-llm

LLM gateway for the workspace. Owns backend resolution between the
`claude` CLI subprocess and the rig-core Anthropic API path, plus a
disk cache that's provider-agnostic.

## Boundary

| Owns | Never does |
|---|---|
| `Llm` trait + `LlmRequest` / `LlmResponse` | Profile/listing schemas (imports from `careerai-profile`) |
| `Backend::resolve(BackendChoice, cfg, cache)` | Tailor prompt construction (lives in `careerai-tailor`) |
| `ClaudeCliLlm` (subprocess driver) | Profile-extract prompt (lives in `careerai-profile`) |
| `RigLlm` (Anthropic API via rig-core, behind `live-llm-api`) | DB queries |
| `Cache` (provider-agnostic, content-hashed) | UI |
| `MockLlm` (test stub) |  |

## Backend selection

```rust
let cache = Arc::new(Cache::new(cache_dir));
let backend = Backend::resolve(cfg.backend, &cfg, cache).await?;
backend.complete(&LlmRequest { ... }).await?;
```

`BackendChoice::Auto` order (when neither is forced):

1. `claude` CLI on PATH AND `claude --print "ping"` exits 0
   within 5s → `ClaudeCli`. (Skip auth probe with
   `CAREERAI_SKIP_CLI_PROBE=1`.)
2. `ANTHROPIC_API_KEY` reachable (env or
   `keyring::Entry::new("career-ai", "anthropic/api_key")`) → `Api`.
3. `BackendError::NoneAvailable`.

When forced (`ClaudeCli` or `Api`): no fallback. Errors if unreachable.

## Cache

`Cache` writes one file per request keyed by SHA-256 of
`(prompt_version, system, user, profile_block, model, temperature,
max_tokens)`. Provider-agnostic — same key whether served from CLI
or API. Disk layout: `<cache_dir>/<sha256-prefix>/<sha256>.json`.

## Features

| Feature | Effect |
|---|---|
| `live-llm-cli` (default) | Build `ClaudeCliLlm` (uses `which` + `tokio::process`) |
| `live-llm-api` | Build `RigLlm` (pulls in `rig-core` + `reqwest`) |

`MockLlm` is always built (used by tests).

## Tests

```bash
cargo test -p careerai-llm
```

`live-claude-cli-real` feature gates an integration test that
hits the actual `claude` binary. CI skips it; local
`cargo test --features live-claude-cli-real` exercises the full
path.

## Probe

`Backend::probe(&cfg)` returns a `ProbeReport` with binary path,
version, ping-ms, API-key source — used by `careerai llm probe`
to render a health-check line for the operator.
