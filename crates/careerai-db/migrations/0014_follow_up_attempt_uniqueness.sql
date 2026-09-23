-- R10: one reminder cadence step belongs to one submitted attempt.
-- Historical rows without attempt identity remain valid and are not
-- retroactively assigned to a newer submission.
CREATE UNIQUE INDEX IF NOT EXISTS uq_follow_ups_attempt_step
    ON follow_ups(submitted_attempt_id, cadence_step)
    WHERE submitted_attempt_id IS NOT NULL;
