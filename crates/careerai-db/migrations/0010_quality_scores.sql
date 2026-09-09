-- Application quality scores produced by the tailor step.
-- Stores the sub-scores (jd_relevance, skill_coverage, cover_letter_depth,
-- bullet_density) and the overall score so the dashboard can surface
-- application quality alongside the listing detail.
CREATE TABLE IF NOT EXISTS application_quality_scores (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL,
    overall REAL NOT NULL,
    jd_relevance REAL NOT NULL,
    skill_coverage REAL NOT NULL,
    cover_letter_depth REAL NOT NULL,
    bullet_density REAL NOT NULL,
    recommendations TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_quality_application ON application_quality_scores(application_id);
