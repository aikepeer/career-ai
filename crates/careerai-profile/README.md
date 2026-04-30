# careerai-profile

Profile schema + ingestion. Parses PDF/DOCX resumes and the
LinkedIn data export ZIP into the canonical `profile.yaml`.

## Boundary

| Owns | Never does |
|---|---|
| `Profile`, `Personal`, `Skills`, `Experience`, `Project`, `Education` schema | LLM transport (uses an injected `LlmCaller` trait) |
| `import_paths(paths) -> Profile` (heuristic / regex extractor) | DB queries |
| `import_paths_with_llm(paths, ctx)` (LLM-extractor entry) | UI |
| `LinkedIn data-export CSV stitcher` (`linkedin.rs`) | Cover letter / tailoring (`careerai-tailor` does that) |
| `Profile::check()` validation | Network |
| `Profile::to_yaml()` / `Profile::from_yaml()` |  |

## Inputs

| Input | Path |
|---|---|
| Resume PDF | text via `pdf-extract`, then heuristic OR LLM extractor |
| Resume DOCX | text via `docx-rs`, then heuristic OR LLM extractor |
| LinkedIn data export ZIP | structured CSV stitch (Profile.csv, Positions.csv, Education.csv, Skills.csv, ...) — never goes through the LLM |

## LLM adapter

The crate doesn't depend on `careerai-llm` directly. Instead it
exposes an `LlmCaller` trait that `careerai-cli` adapts via the
`profile_llm_adapter::Adapter` (in the CLI crate). Breaks the
`profile ↔ llm` cycle.

```rust
let adapter = Adapter::new(&backend);
let ctx = LlmExtractContext::new(&adapter, opts);
careerai_profile::import_paths_with_llm(&paths, Some(&ctx))?;
```

## Error semantics

| Error | Behavior |
|---|---|
| `LinkedInMissingFile` | Silently tolerated when stitching optional CSVs |
| `LinkedInMissingColumn` | Surfaced loudly — indicates LinkedIn changed its export format |
| Stale schema (skills as flat list) | Detected by CLI's `profile validate` before serde, with a clear "re-import with --force" hint |

## Bullet & skill dedup

* Bullets: case-sensitive (so `API` and `api` survive separately).
* Skills: case-insensitive.
* Experience entries with empty `start` dates: never deduped.

## Tests

```bash
cargo test -p careerai-profile
```

Fixture LinkedIn ZIPs + sample PDFs/DOCXs live under `tests/fixtures/`.
