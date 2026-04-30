# careerai-core

Shared types and configuration for the career-ai workspace. The
foundation crate every other crate depends on.

## Boundary

| Owns | Never does |
|---|---|
| `CoreConfig` schema + layered loader (defaults / project / `local.yaml` / env) | Business logic |
| `ListingState` enum + transitions allowlist | I/O of any kind |
| `Domain`, `Skills`, `MatchConfig`, `RatesConfig`, `SubmitConfig`, `LlmConfig`, `SchedulerConfig`, `SourcesConfig`, `RenderConfig`, `DashboardConfig` config sections | DB queries |
| Embedded default config template (`templates/default.yaml`) | Anything that depends on a sibling crate (no inbound deps) |

## Key types

* `CoreConfig::load(cwd: &Path)` — layered loader; succeeds even on a
  fresh directory by falling back to the embedded defaults.
* `ListingState` — the 11-variant pipeline state machine
  (`Discovered → Shortlisted → Tailored → Rendered → Submitted →
  Responded`, plus terminal `FilteredOut`, `Skipped`, `Failed`, and
  WIP `Prepared`, `Drafted`).
* `BackendChoice::{Auto, ClaudeCli, Api}` — LLM backend selector.

## Adding a new config section

1. Add a `pub struct YourConfig` with `Debug + Clone + Serialize +
   Deserialize + Default` in `config.rs`.
2. Mark fields `#[serde(default = "fn_name")]` if they need
   non-`Default::default()` defaults.
3. Add a field to `CoreConfig` with `#[serde(default)]`.
4. Update `templates/default.yaml` with the new block.
5. Wire it into the consuming crate via `cfg.<your_section>`.

## Tests

```bash
cargo test -p careerai-core
```

Snapshot tests use `insta`. Re-record with
`INSTA_UPDATE=always cargo test -p careerai-core`.
