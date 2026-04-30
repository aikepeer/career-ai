# careerai-pipeline

End-to-end pipeline orchestration. The single place where
discovery → match → tailor → render → apply gets composed; both
`careerai-cli` and `careerai-scheduler` call into this crate.

## Boundary

| Owns | Never does |
|---|---|
| `discover_all`, `discover_one` | Direct DB writes (delegates to `careerai-db`) |
| `match_all`, `match_one` (calls into `careerai-match`) | LLM calls (delegates to `careerai-tailor` / `careerai-llm`) |
| `tailor_one` (LLM-tailored resume + cover) | HTTP (delegates to source / submit crates) |
| `render_one` (DOCX + PDF via pandoc) | Browser automation (delegates to `careerai-submit`) |
| `apply_one`, `apply_all`, `applied_show` | Argument parsing |
| `inspect_show`, `digest_summary` | Cron scheduling |
| `confirm_linkedin_submit` (race-guarded claim) |  |

## Architectural rule

This is the **only** crate where pipeline stages compose end-to-end.
`careerai-cli` and `careerai-scheduler` MUST go through this crate
rather than reaching past into the implementation crates.

## Stage entry points

```rust
pipeline::discover_all(root, cfg, source_filter) -> DiscoveryReport
pipeline::match_all(root, cfg, tune)              -> MatchReport
pipeline::tailor_one(root, cfg, listing_id)       -> TailoredOutcome
pipeline::render_one(root, cfg, application_id)   -> RenderedOutcome
pipeline::apply_one(root, cfg, app_id, override)  -> AppliedOutcome
pipeline::apply_all(root, cfg, src_filter, override) -> Vec<AppliedOutcome>
pipeline::inspect_show(root, application_id)      -> InspectReport
pipeline::digest_summary(root, since)             -> DigestReport
```

## Notify wiring

`match_all` builds a `careerai_notify::Pipeline` from `cfg.notify`
once per run and fires `NotifyEvent::HighScoreMatch` when a kept
listing's score is `>= cfg.matching.notify_threshold`. Init
failures degrade to "no channels" with a warn log, never killing
the run. Double-fire is prevented structurally — `match_all` only
walks `discovered`-state rows.

## Tests

```bash
cargo test -p careerai-pipeline
```

Integration tests use `wiremock` for HTTP and an in-memory SQLite.
End-to-end pipeline tests live in `tests/`.

## Features

| Feature | Effect |
|---|---|
| `live-llm-cli` (default) | Pull in `careerai-llm/live-llm-cli` |
| `live-llm-api` | Pull in `careerai-llm/live-llm-api` |
| `live-llm` | Both backends |
| `browser` | Pass through to `careerai-submit/browser` + `careerai-sources/browser` |
