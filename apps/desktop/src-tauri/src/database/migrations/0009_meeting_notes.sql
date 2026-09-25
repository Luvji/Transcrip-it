CREATE TABLE meeting_notes (
    meeting_id TEXT PRIMARY KEY NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    approved INTEGER NOT NULL DEFAULT 0 CHECK (approved IN (0, 1)),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

