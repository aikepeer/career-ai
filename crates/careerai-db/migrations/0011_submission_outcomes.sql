-- R01: Submission attempts — durable claim and recovery record.
-- Each attempt records who claimed it, when, the remote receipt (when
-- available), and an explicit status so uncertain remote results can be
-- reconciled before retry. The UNIQUE index on (application_id, attempt_no)
-- prevents duplicate attempt numbers for one application.
CREATE TABLE IF NOT EXISTS submission_attempts (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id  TEXT NOT NULL,
    attempt_no      INTEGER NOT NULL DEFAULT 1,
    status          TEXT NOT NULL DEFAULT 'claimed',  -- claimed | submitted | failed | uncertain
    approver         TEXT,                               -- who approved the submission
    remote_receipt   TEXT,                               -- provider-returned id / receipt
    idempotency_key  TEXT,                               -- provider idempotency key when supported
    error            TEXT,                               -- captured error on failure
    pre_submit_state TEXT,                               -- listing state before the attempt
    claimed_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    submitted_at    TEXT,
    resolved_at     TEXT,
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_submission_attempts_app ON submission_attempts(application_id);
CREATE INDEX IF NOT EXISTS idx_submission_attempts_status ON submission_attempts(status);
CREATE UNIQUE INDEX IF NOT EXISTS uq_submission_attempts_app_attempt
    ON submission_attempts(application_id, attempt_no);

-- R10: Follow-up cadence — associate each follow-up with a real submitted
-- attempt and a cadence step so reminders recur only when their next
-- configured step is due. The UNIQUE index on (application_id, cadence_step)
-- prevents concurrent schedulers from creating duplicate drafts for the
-- same logical follow-up.
ALTER TABLE follow_ups ADD COLUMN cadence_step INTEGER NOT NULL DEFAULT 1;
ALTER TABLE follow_ups ADD COLUMN submitted_attempt_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS uq_follow_ups_app_step
    ON follow_ups(application_id, cadence_step)
    WHERE status = 'pending';

-- F04: Manual employer-outcome timeline — records externally applied,
-- replied, interview, rejection, offer, and withdrawal events separately
-- from technical pipeline state. Each row references a submission attempt
-- when one exists so the outcome is tied to a real submission.
CREATE TABLE IF NOT EXISTS employer_outcomes (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id  TEXT,                                -- nullable: outcome may precede formal application
    listing_id      TEXT NOT NULL,
    attempt_id      INTEGER,                             -- submission_attempts.id when known
    outcome_type    TEXT NOT NULL,                       -- applied | replied | interview | rejection | offer | withdrawal
    occurred_at     TEXT NOT NULL,                       -- when the event happened (user-supplied)
    is_manual       INTEGER NOT NULL DEFAULT 1,          -- 1 = user-recorded, 0 = system-derived
    note            TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (application_id) REFERENCES applications(id) ON DELETE CASCADE,
    FOREIGN KEY (listing_id) REFERENCES listings(id) ON DELETE CASCADE,
    FOREIGN KEY (attempt_id) REFERENCES submission_attempts(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_outcomes_app ON employer_outcomes(application_id);
CREATE INDEX IF NOT EXISTS idx_outcomes_type ON employer_outcomes(outcome_type);
CREATE INDEX IF NOT EXISTS idx_outcomes_listing ON employer_outcomes(listing_id);
-- R02: add profile_hash to cover_letter_library so reuse can validate
-- the stored letter belongs to the same candidate. Existing rows get
-- a sentinel hash that matches nothing (so they are never reused
-- without a re-store), preventing cross-candidate leakage.
ALTER TABLE cover_letter_library ADD COLUMN profile_hash TEXT;
UPDATE cover_letter_library SET profile_hash = '__legacy__' WHERE profile_hash IS NULL;
CREATE INDEX IF NOT EXISTS idx_cl_lib_profile_hash ON cover_letter_library (profile_hash);
-- R16: A/B variant provenance — record the content version (resume diff
-- hash) alongside the variant label so outcomes can be attributed to a
-- specific resume treatment, not just an arbitrary A/B label. The unique
-- index on (application_id, variant_label) prevents duplicate records
-- for one assignment.
ALTER TABLE application_variants ADD COLUMN content_version TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS uq_variants_app_label
    ON application_variants(application_id, variant_label);
