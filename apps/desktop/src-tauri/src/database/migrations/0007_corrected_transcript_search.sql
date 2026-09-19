CREATE TRIGGER transcript_search_after_correction
AFTER INSERT ON transcript_derivatives
WHEN NEW.derivative_kind = 'correction'
BEGIN
    UPDATE transcript_search
    SET text = NEW.derived_text
    WHERE segment_id = NEW.segment_id;
END;
