-- Speed up `SELECT source, MAX(created_at) FROM listings GROUP BY source`
-- which the dashboard issues on every render. On a 9k-listing DB the
-- pre-index version logged a sqlx slow-statement warning at ~2.1s.
-- A composite (source, created_at) lets SQLite use index-only scan
-- + per-group MAX without touching the heap.
CREATE INDEX idx_listings_source_created_at
    ON listings (source, created_at DESC);

-- Also accelerate "count listings created today" — the KPI strip's
-- `today_discovered` does `WHERE created_at >= ?` over the full table
-- on every render. A single-column descending index handles both
-- range scans and the "newest first" pagination cases.
CREATE INDEX idx_listings_created_at
    ON listings (created_at DESC);
