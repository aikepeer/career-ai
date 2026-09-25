# career-ai

Local single-user automated job discovery & tailoring pipeline (Rust end-to-end; Edition 2021, MSRV 1.78). Targets AI/ML & robotics niches. Runs as local daemon (`tokio-cron-scheduler`) or CLI (`careerai-cli`).

## Architecture & State Machine

Linear state machine in SQLite:
`discovered → filtered_out | shortlisted → tailored → rendered → prepared → submitted | skipped | failed → responded`

### Crate Boundaries
- `careerai-core`: Orchestration, state transitions, config loading (`config/default.yaml` into `CoreConfig`). Never direct I/O or DB queries.
- `careerai-db`: `sqlx` models, queries, migrations. No business logic.
- `careerai-sources`: Job board discovery adapters behind `Source` trait.
- `careerai-match`: Filters, fastembed embeddings, cosine rank. No LLM calls.
- `careerai-llm`: `rig`-based provider gateway, prompt building, response cache.
- `careerai-tailor`: Constrained-diff resume edits, cover letter drafting.
- `careerai-render`: Tera → Markdown → `pandoc` (system dependency) → DOCX/PDF. No network.
- `careerai-submit`: Submitters behind `Submitter` trait (HTTP/browser/email) + dry-run wrapper.
- `careerai-scheduler`: Daemon scheduling, cron wiring, graceful shutdown.
- `careerai-cli`: `clap` CLI dispatch only.

## Safety & Domain Invariants (NON-NEGOTIABLE)

1. **LLM Resume Diff Safety**: `careerai-tailor/src/diff.rs` emits a constrained JSON diff that can ONLY reorder or rewrite existing profile bullets. NEVER invent experience, titles, dates, or employers. Schema validator rejects all out-of-grammar diffs.
2. **Submitter Dry-Run Gate**: `auto_submit` defaults to `false`. In dry-run mode, submitters NEVER issue network writes (screenshot + log `would_submit` instead). Per-source `submit_enabled` gates must be respected even with `--auto-submit`.
3. **Rate Limiting**: Outbound submissions go through `governor` token buckets per source. Quiet hours and daily caps are strictly enforced at the boundary.
4. **Profile Pipeline Invariants**: Bullet dedup is case-sensitive; skill dedup is case-insensitive. Entries with empty `start` dates are never deduped. `dates::two_digit_month` validates `1..=12` (out-of-range passes through verbatim). `LinkedInMissingFile` is tolerated; column drifts (`LinkedInMissingColumn`) error immediately.
5. **Credentials & Secrets**: Keychain via `keyring` crate; `.env` fallback. Redact secrets from logs; tests verify zero secret leakage.

## Testing Layers

- **Unit**: Pure functions (parsing, filters, rank, diff apply). Fast, zero I/O.
- **Integration** (`tests/*_it.rs`): Real SQLite (`tempfile`), `wiremock` HTTP, `MockLLM` stub for `rig`.
- **Golden / Snapshots**: `insta` for filter outputs; `pdf-extract` + `docx-rs` text diffs for render artifacts (`INSTA_UPDATE=always cargo test`).
- **Browser**: `chromiumoxide` against local `tiny-http` fixture server (CI never hits live LinkedIn/Indeed). Pinned `stealth-v2.js` checked by SHA.

## Token Discipline & File Boundaries

- **Forbidden Full-File Reads** (use Serena symbol tools or `offset`+`limit`):
  `careerai-pipeline/src/lib.rs`, `careerai-submit/src/rate_limiter.rs`, `careerai-submit/src/linkedin.rs`, `careerai-scheduler/src/lib.rs`, `careerai-core/src/config.rs`.
- **Search Tooling**: Known symbols → Serena (`find_symbol`, `get_symbols_overview`). Structural patterns → `sg`. Exact/regex → `rg`.
- **Workspace Cargo Commands**: Route large workspace commands (`cargo test --workspace`, `cargo clippy`) through `bash` with `| tail -n 40` or a scoped `--test <name>` filter to avoid dumping unbounded output.
