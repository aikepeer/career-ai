-- Interview feedback loop: log questions asked and how the interview went.
CREATE TABLE IF NOT EXISTS interview_feedback (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL,
    listing_id TEXT NOT NULL,
    company TEXT NOT NULL,
    title TEXT NOT NULL,
    questions_text TEXT,                -- free-text: questions asked
    rating INTEGER NOT NULL DEFAULT 3,  -- 1-5 self-assessment
    went_well TEXT,                     -- what went well
    could_improve TEXT,                 -- what to improve
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id),
    FOREIGN KEY (listing_id) REFERENCES listings(id)
);
CREATE INDEX IF NOT EXISTS idx_feedback_listing ON interview_feedback(listing_id);
