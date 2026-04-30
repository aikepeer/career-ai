# careerai-db

`sqlx` models, queries, and migrations for the career-ai SQLite DB.
The persistence layer.

## Boundary

| Owns | Never does |
|---|---|
| `sqlx::SqlitePool` construction (`pool_from_path`, `pool_in_memory`) | Business logic |
| Migration runner (sqlx::migrate! over `migrations/`) | HTTP / network |
| Hand-rolled SQL queries grouped by entity (`queries/{listings, applications, applications_sync, payloads, artifacts, events, linkedin}.rs`) | LLM calls |
| `Listing`, `Application`, `ApplicationPayload`, `Artifact`, `Event` row types + their `New*` builders | UI |

## Schema

Five tables: `listings`, `applications`, `application_payloads`,
`artifacts`, `events`. See `migrations/0001_init.sql` and
`migrations/0002_applications.sql` for shape; `0003_*` adds
performance indexes for the dashboard's source-lag query.

## Query layout

```
src/queries/
  mod.rs                declarations + re-exports + test_support fixtures
  listings.rs           listings CRUD + transition (writes events in same tx)
  applications.rs       single-table applications CRUD
  applications_sync.rs  cross-table queries + lockstep transitions
  linkedin.rs           LinkedIn drafts review flow
  payloads.rs           application_payloads table
  artifacts.rs          artifacts table
  events.rs             read-only event queries
```

`pub use submodule::*` in `mod.rs` keeps the flat path
`careerai_db::queries::find_by_id` working unchanged.

## Tests

```bash
cargo test -p careerai-db
```

Tests use `pool_in_memory()` — no on-disk fixtures, parallel-safe.

## Adding a query

1. Pick the entity module (or `applications_sync.rs` for cross-table).
2. Add `pub async fn`. Use `sqlx::query_as` for typed rows.
3. Re-export from `queries/mod.rs`.
4. Add a test in the same file under `#[cfg(test)] mod tests`.
