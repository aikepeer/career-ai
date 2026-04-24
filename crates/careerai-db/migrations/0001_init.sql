-- Listings: one row per job seen, regardless of source.
-- (source, external_id) is the natural key from the upstream board; the
-- internal `id` is a UUIDv7 we mint so foreign keys are stable even if a
-- source recycles an external id.
CREATE TABLE listings (
    id            TEXT PRIMARY KEY NOT NULL,
    source        TEXT NOT NULL,
    external_id   TEXT NOT NULL,
    title         TEXT NOT NULL,
    company       TEXT NOT NULL,
    location      TEXT,
    url           TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    raw_json      TEXT,
    state         TEXT NOT NULL DEFAULT 'discovered',
    score         REAL,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (source, external_id)
);

CREATE INDEX idx_listings_state ON listings (state);
CREATE INDEX idx_listings_source_state ON listings (source, state);
CREATE INDEX idx_listings_score ON listings (score) WHERE score IS NOT NULL;

-- Events: append-only audit log of state transitions.
CREATE TABLE events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    listing_id  TEXT NOT NULL REFERENCES listings(id) ON DELETE CASCADE,
    from_state  TEXT,
    to_state    TEXT NOT NULL,
    note        TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_events_listing_id ON events (listing_id);
CREATE INDEX idx_events_created_at ON events (created_at);
