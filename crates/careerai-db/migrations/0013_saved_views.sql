-- F06: Persisted saved views for the explorer. Each row stores a named
-- filter preset (query, source, state, remote) so the user can restore a
-- known-good filter combination across sessions. The filter JSON column
-- is a denormalized copy of the individual columns for forward-compat
-- when new filter dimensions are added.
CREATE TABLE IF NOT EXISTS saved_views (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL UNIQUE,
    query        TEXT,                          -- free-text search; NULL = no query
    source       TEXT,                          -- source filter; NULL = any
    state        TEXT,                          -- listing state filter; NULL = any
    remote_only INTEGER NOT NULL DEFAULT 0,     -- 0 = any, 1 = remote only
    filter_json  TEXT NOT NULL DEFAULT '{}',    -- denormalized full filter for forward-compat
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
