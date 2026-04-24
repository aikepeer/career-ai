-- An Application is one user-approved "take this listing and prepare a
-- submission" attempt. Multiple applications per listing are allowed
-- (e.g. re-tailor after profile edits); the latest is "current".
CREATE TABLE applications (
    id             TEXT PRIMARY KEY NOT NULL,           -- uuid v7
    listing_id     TEXT NOT NULL REFERENCES listings(id) ON DELETE RESTRICT,
    state          TEXT NOT NULL DEFAULT 'tailored',    -- tailored|rendered|prepared|submitted|failed
    profile_hash   TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    llm_model      TEXT NOT NULL,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_applications_listing_id ON applications (listing_id);
CREATE INDEX idx_applications_state ON applications (state);

-- Stores the tailored ResumeView + CoverLetter as JSON for later inspect/diff
-- without re-calling the LLM. Kept separate from artifacts (files on disk).
CREATE TABLE application_payloads (
    application_id   TEXT PRIMARY KEY NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    resume_view_json TEXT NOT NULL,
    cover_letter_text TEXT NOT NULL,
    diff_json        TEXT NOT NULL,   -- raw LLM diff document, for debugging/audit
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Files on disk produced by render. One row per output file.
CREATE TABLE artifacts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL,    -- resume_md|resume_docx|resume_pdf|cover_md|cover_docx
    path           TEXT NOT NULL,
    bytes          INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (application_id, kind)
);

CREATE INDEX idx_artifacts_application_id ON artifacts (application_id);
