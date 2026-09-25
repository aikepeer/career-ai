-- Follow-up email scheduler: tracks pending and sent follow-ups for
-- applications that haven't received a response within N days.
CREATE TABLE IF NOT EXISTS follow_ups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL,
    listing_id TEXT NOT NULL,
    scheduled_at TEXT NOT NULL,   -- ISO8601 — when the follow-up should be sent
    sent_at TEXT,                 -- NULL = pending, set when sent
    status TEXT NOT NULL DEFAULT 'pending',  -- pending | sent | skipped
    body TEXT,                    -- draft follow-up email body
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id),
    FOREIGN KEY (listing_id) REFERENCES listings(id)
);
CREATE INDEX IF NOT EXISTS idx_follow_ups_status ON follow_ups(status);
CREATE INDEX IF NOT EXISTS idx_follow_ups_scheduled ON follow_ups(scheduled_at);
