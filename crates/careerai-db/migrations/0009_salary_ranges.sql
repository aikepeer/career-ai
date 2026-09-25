-- Salary ranges extracted from job descriptions.
CREATE TABLE IF NOT EXISTS salary_ranges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    listing_id TEXT NOT NULL,
    company TEXT NOT NULL,
    title TEXT NOT NULL,
    min_salary INTEGER,
    max_salary INTEGER,
    currency TEXT NOT NULL DEFAULT 'USD',
    period TEXT NOT NULL DEFAULT 'year',  -- year | month | hour
    raw_text TEXT,                          -- original salary text from JD
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (listing_id) REFERENCES listings(id)
);
CREATE INDEX IF NOT EXISTS idx_salary_listing ON salary_ranges(listing_id);
CREATE INDEX IF NOT EXISTS idx_salary_company ON salary_ranges(company);
