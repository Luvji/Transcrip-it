CREATE TABLE transcript_derivatives (
    id TEXT PRIMARY KEY NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE CHECK (length(trim(idempotency_key)) > 0),
    segment_id TEXT NOT NULL REFERENCES transcript_segments(id) ON DELETE CASCADE,
    derivative_kind TEXT NOT NULL
        CHECK (derivative_kind IN ('correction', 'translation', 'romanization')),
    author_kind TEXT NOT NULL CHECK (author_kind IN ('user', 'model')),
    revision_number INTEGER NOT NULL CHECK (revision_number > 0),
    derived_text TEXT NOT NULL,
    language_code TEXT,
    model_id TEXT,
    supersedes_id TEXT REFERENCES transcript_derivatives(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (segment_id, derivative_kind, revision_number)
);

CREATE INDEX idx_transcript_derivatives_segment_kind
    ON transcript_derivatives(segment_id, derivative_kind, revision_number DESC);

CREATE TRIGGER protect_transcript_source_evidence
BEFORE UPDATE OF
    meeting_id,
    sequence_number,
    start_ms,
    end_ms,
    source_text,
    created_at
ON transcript_segments
BEGIN
    SELECT RAISE(ABORT, 'source transcript evidence is immutable');
END;

CREATE VIEW transcript_current_text AS
SELECT
    segment.id AS segment_id,
    segment.meeting_id,
    segment.sequence_number,
    segment.start_ms,
    segment.end_ms,
    segment.source_text,
    COALESCE(
        (
            SELECT derivative.derived_text
            FROM transcript_derivatives AS derivative
            WHERE derivative.segment_id = segment.id
              AND derivative.derivative_kind = 'correction'
            ORDER BY derivative.revision_number DESC
            LIMIT 1
        ),
        segment.source_text
    ) AS display_text,
    (
        SELECT derivative.id
        FROM transcript_derivatives AS derivative
        WHERE derivative.segment_id = segment.id
          AND derivative.derivative_kind = 'correction'
        ORDER BY derivative.revision_number DESC
        LIMIT 1
    ) AS active_correction_id
FROM transcript_segments AS segment;
