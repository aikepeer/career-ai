-- Profile-scoped local tailoring material. Rows are invalidated by profile_hash.
CREATE TABLE bullet_variants (
    profile_hash TEXT NOT NULL,
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('experience', 'projects')),
    entry_index INTEGER NOT NULL CHECK (entry_index >= 0),
    bullet_index INTEGER NOT NULL CHECK (bullet_index >= 0),
    original TEXT NOT NULL,
    variants_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (profile_hash, entry_kind, entry_index, bullet_index)
);

CREATE INDEX idx_bullet_variants_profile ON bullet_variants (profile_hash);

CREATE TABLE cover_skeletons (
    profile_hash TEXT NOT NULL,
    domain TEXT NOT NULL,
    keywords_json TEXT NOT NULL,
    template TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (profile_hash, domain)
);

CREATE INDEX idx_cover_skeletons_profile ON cover_skeletons (profile_hash);
