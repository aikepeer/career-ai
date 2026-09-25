-- A/B variant tracking: which resume variant was submitted for each
-- application, and whether it got a response. Used to learn which
-- phrasing patterns convert.
CREATE TABLE IF NOT EXISTS application_variants (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL,
    variant_label TEXT NOT NULL,         -- e.g. "A", "B", "concise", "detailed"
    variant_metadata TEXT,               -- JSON: which bullets changed, tone, etc.
    submitted_at TEXT,                    -- when this variant was submitted
    response_status TEXT DEFAULT 'pending', -- pending | responded | rejected | ghosted
    response_at TEXT,                     -- when response was received
    response_type TEXT,                   -- interview | rejection | auto_reply | manual
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id)
);
CREATE INDEX IF NOT EXISTS idx_variants_application ON application_variants(application_id);
CREATE INDEX IF NOT EXISTS idx_variants_status ON application_variants(response_status);
