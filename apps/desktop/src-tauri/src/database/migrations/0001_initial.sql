CREATE TABLE meetings (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    lifecycle_state TEXT NOT NULL DEFAULT 'draft',
    source_kind TEXT NOT NULL DEFAULT 'recording',
    source_language TEXT,
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    started_at TEXT,
    ended_at TEXT,
    archived_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE transcript_segments (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    sequence_number INTEGER NOT NULL CHECK (sequence_number >= 0),
    start_ms INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms INTEGER NOT NULL CHECK (end_ms >= start_ms),
    speaker_label TEXT,
    language_code TEXT,
    source_text TEXT NOT NULL DEFAULT '',
    confidence REAL CHECK (confidence IS NULL OR confidence BETWEEN 0.0 AND 1.0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (meeting_id, sequence_number)
);

CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT REFERENCES meetings(id) ON DELETE CASCADE,
    job_kind TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued',
    progress REAL NOT NULL DEFAULT 0.0 CHECK (progress BETWEEN 0.0 AND 1.0),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    payload_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(payload_json)),
    checkpoint_json TEXT CHECK (checkpoint_json IS NULL OR json_valid(checkpoint_json)),
    error_code TEXT,
    error_message TEXT,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE actions (
    id TEXT PRIMARY KEY NOT NULL,
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    source_segment_id TEXT REFERENCES transcript_segments(id) ON DELETE SET NULL,
    action_text TEXT NOT NULL CHECK (length(trim(action_text)) > 0),
    state TEXT NOT NULL DEFAULT 'open',
    assignee TEXT,
    due_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE settings (
    key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(key)) > 0),
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_meetings_updated_at ON meetings(updated_at DESC);
CREATE INDEX idx_segments_meeting_timeline ON transcript_segments(meeting_id, start_ms);
CREATE INDEX idx_jobs_state_created_at ON jobs(state, created_at);
CREATE INDEX idx_jobs_meeting_id ON jobs(meeting_id);
CREATE INDEX idx_actions_meeting_state ON actions(meeting_id, state);
CREATE INDEX idx_actions_source_segment ON actions(source_segment_id);
