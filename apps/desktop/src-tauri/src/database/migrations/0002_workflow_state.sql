ALTER TABLE meetings ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0 CHECK (state_revision >= 0);

CREATE TABLE meeting_transitions (
    idempotency_key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(idempotency_key)) > 0),
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    resulting_revision INTEGER NOT NULL CHECK (resulting_revision > 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_meeting_transitions_meeting_revision
    ON meeting_transitions(meeting_id, resulting_revision);

DROP INDEX idx_jobs_state_created_at;
DROP INDEX idx_jobs_meeting_id;
ALTER TABLE jobs RENAME TO jobs_before_workflow_state;

CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE CHECK (length(trim(idempotency_key)) > 0),
    meeting_id TEXT REFERENCES meetings(id) ON DELETE CASCADE,
    job_kind TEXT NOT NULL CHECK (length(trim(job_kind)) > 0),
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'running', 'succeeded', 'failed', 'cancelled')),
    progress REAL NOT NULL DEFAULT 0.0 CHECK (progress BETWEEN 0.0 AND 1.0),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    payload_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(payload_json)),
    checkpoint_json TEXT CHECK (checkpoint_json IS NULL OR json_valid(checkpoint_json)),
    run_token TEXT,
    available_at INTEGER NOT NULL DEFAULT (unixepoch()),
    lease_expires_at INTEGER,
    error_code TEXT,
    error_message TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK (state != 'running' OR (run_token IS NOT NULL AND lease_expires_at IS NOT NULL))
);

INSERT INTO jobs (
    id,
    idempotency_key,
    meeting_id,
    job_kind,
    state,
    progress,
    attempt_count,
    payload_json,
    checkpoint_json,
    error_code,
    error_message,
    started_at,
    finished_at,
    created_at,
    updated_at
)
SELECT
    id,
    id,
    meeting_id,
    job_kind,
    CASE
        WHEN state = 'running' THEN 'queued'
        WHEN state IN ('queued', 'succeeded', 'failed', 'cancelled') THEN state
        ELSE 'failed'
    END,
    progress,
    attempt_count,
    payload_json,
    checkpoint_json,
    error_code,
    error_message,
    started_at,
    finished_at,
    created_at,
    updated_at
FROM jobs_before_workflow_state;

DROP TABLE jobs_before_workflow_state;

CREATE INDEX idx_jobs_state_available_at ON jobs(state, available_at, created_at);
CREATE INDEX idx_jobs_kind_state ON jobs(job_kind, state);
CREATE INDEX idx_jobs_meeting_id ON jobs(meeting_id);
