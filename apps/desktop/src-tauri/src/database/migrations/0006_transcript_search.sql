CREATE VIRTUAL TABLE transcript_search USING fts5(
    meeting_id UNINDEXED,
    segment_id UNINDEXED,
    text,
    tokenize = 'unicode61'
);

INSERT INTO transcript_search (meeting_id, segment_id, text)
SELECT meeting_id, id, source_text FROM transcript_segments;

CREATE TRIGGER transcript_search_after_insert
AFTER INSERT ON transcript_segments
BEGIN
    INSERT INTO transcript_search (meeting_id, segment_id, text)
    VALUES (NEW.meeting_id, NEW.id, NEW.source_text);
END;

CREATE TRIGGER transcript_search_after_delete
AFTER DELETE ON transcript_segments
BEGIN
    DELETE FROM transcript_search WHERE segment_id = OLD.id;
END;
