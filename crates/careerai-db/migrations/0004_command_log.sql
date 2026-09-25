-- Dashboard-spawned CLI command audit log.
--
-- Records every command executed via /api/v1/cli/run (success + failure)
-- so the dashboard Events tab surfaces tailor/render/apply failures and
-- all operator-triggered actions alongside pipeline state transitions.
--
-- `listing_id` is nullable because commands like `discover` / `match`
-- / `run` are not tied to a single listing. `exit_code` is NULL when the
-- process timed out or could not be spawned.

CREATE TABLE command_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    command     TEXT NOT NULL,
    listing_id  TEXT,
    status      TEXT NOT NULL,   -- 'success' | 'failed' | 'timeout' | 'error'
    exit_code   INTEGER,
    message     TEXT,             -- stdout+stderr or error description
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_command_log_created_at ON command_log (created_at);
CREATE INDEX idx_command_log_status ON command_log (status);
