ALTER TABLE meetings ADD COLUMN creation_key TEXT;
ALTER TABLE meetings ADD COLUMN pre_archive_state TEXT;

CREATE UNIQUE INDEX idx_meetings_creation_key
    ON meetings(creation_key)
    WHERE creation_key IS NOT NULL;

CREATE TABLE meeting_tags (
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    tag TEXT NOT NULL COLLATE NOCASE CHECK (length(trim(tag)) > 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (meeting_id, tag)
);

CREATE INDEX idx_meeting_tags_tag ON meeting_tags(tag, meeting_id);
