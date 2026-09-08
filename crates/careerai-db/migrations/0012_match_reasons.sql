-- F02: Explainable match cards — persist the structured match reasons
-- alongside the listing score so the dashboard can surface *why* a listing
-- was shortlisted or filtered out. Each row records the score, the
-- keywords from the profile that matched the listing, the configured
-- keywords that were missing, the filter reason (if rejected), and the
-- legitimacy tier. One row per listing; upserted on every match run.
CREATE TABLE IF NOT EXISTS match_reasons (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    listing_id        TEXT NOT NULL UNIQUE,
    score             REAL NOT NULL,
    matched_keywords  TEXT NOT NULL DEFAULT '[]',   -- JSON array of matched profile keywords
    missing_keywords  TEXT NOT NULL DEFAULT '[]',   -- JSON array of missing configured keywords
    filter_reason     TEXT,                          -- non-null when the listing was filtered out
    legitimacy_tier   TEXT,                          -- legit | caution | suspicious | unknown
    legitimacy_score  REAL,
    eligibility_note  TEXT,                          -- authorization / eligibility summary
    matched_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (listing_id) REFERENCES listings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_match_reasons_listing ON match_reasons(listing_id);
