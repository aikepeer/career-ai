-- JD + profile similarity index for LLM-call reuse (C3).
--
-- After each successful tailor, we record (listing_id, profile_hash,
-- company, title, title_normalized, jd_hash, application_id). Before
-- calling the LLM for a new listing, we query this index for entries
-- with the same profile_hash and a similar company/title — if found,
-- the stored diff/cover-letter is reused, skipping the LLM call entirely.
--
-- This catches reposts (same company + same title), near-duplicate
-- listings across sources, and re-tailors after minor profile edits.

CREATE TABLE jd_similarity_index (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    listing_id       TEXT NOT NULL REFERENCES listings(id) ON DELETE CASCADE,
    profile_hash     TEXT NOT NULL,
    company          TEXT NOT NULL,
    title            TEXT NOT NULL,
    title_normalized TEXT NOT NULL,   -- lowercased alnum tokens joined by space
    jd_hash          TEXT NOT NULL,
    application_id   TEXT REFERENCES applications(id) ON DELETE SET NULL,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_jd_sim_profile_hash ON jd_similarity_index (profile_hash);
CREATE INDEX idx_jd_sim_company ON jd_similarity_index (company);
CREATE INDEX idx_jd_sim_title_norm ON jd_similarity_index (title_normalized);

-- Cover-letter and resume-bullet reuse library (C4).
-- Indexed by job domain and role. When the LLM generates a new tailored
-- result, bullets and the cover letter are stored by domain/role. When
-- tailoring a new listing in a known domain, stored content is reused
-- directly — no LLM call needed.

CREATE TABLE bullet_library (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    domain      TEXT NOT NULL,
    role        TEXT NOT NULL,
    bullet_text TEXT NOT NULL,
    theme_label TEXT NOT NULL,
    source_application_id TEXT REFERENCES applications(id) ON DELETE SET NULL,
    use_count   INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_bullet_lib_domain_role ON bullet_library (domain, role);

CREATE TABLE cover_letter_library (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    domain      TEXT NOT NULL,
    role        TEXT NOT NULL,
    template_text TEXT NOT NULL,
    source_application_id TEXT REFERENCES applications(id) ON DELETE SET NULL,
    use_count   INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX idx_cl_lib_domain_role ON cover_letter_library (domain, role);
