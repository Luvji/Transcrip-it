ALTER TABLE transcript_segments
ADD COLUMN source_track TEXT CHECK (source_track IS NULL OR source_track IN ('mic', 'system'));

CREATE TRIGGER protect_transcript_source_track
BEFORE UPDATE OF source_track ON transcript_segments
BEGIN
    SELECT RAISE(ABORT, 'source transcript evidence is immutable');
END;
