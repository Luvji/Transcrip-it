use super::{Database, DatabaseError};
use rusqlite::{params, OptionalExtension};

#[derive(Debug)]
pub(crate) struct StoredMeetingNotes {
    pub payload_json: String,
    pub approved: bool,
}

impl Database {
    pub(crate) fn load_meeting_notes(
        &self,
        meeting_id: &str,
    ) -> Result<Option<StoredMeetingNotes>, DatabaseError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT payload_json, approved FROM meeting_notes WHERE meeting_id = ?1",
                [meeting_id],
                |row| {
                    Ok(StoredMeetingNotes {
                        payload_json: row.get(0)?,
                        approved: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(DatabaseError::from)
    }

    pub(crate) fn store_meeting_notes(
        &self,
        meeting_id: &str,
        payload_json: &str,
        approved: bool,
    ) -> Result<(), DatabaseError> {
        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO meeting_notes (meeting_id, payload_json, approved)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(meeting_id) DO UPDATE SET
                payload_json = excluded.payload_json,
                approved = excluded.approved,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![meeting_id, payload_json, approved],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_notes_round_trip_and_follow_meeting_deletion() {
        let database = Database::open_in_memory().unwrap();
        {
            let connection = database.connection().unwrap();
            connection
                .execute(
                    "INSERT INTO meetings (id, title) VALUES ('meeting-1', 'Review')",
                    [],
                )
                .unwrap();
        }

        database
            .store_meeting_notes("meeting-1", r#"{"meetingId":"meeting-1"}"#, true)
            .unwrap();
        let stored = database.load_meeting_notes("meeting-1").unwrap().unwrap();
        assert_eq!(stored.payload_json, r#"{"meetingId":"meeting-1"}"#);
        assert!(stored.approved);

        database
            .connection()
            .unwrap()
            .execute("DELETE FROM meetings WHERE id = 'meeting-1'", [])
            .unwrap();
        assert!(database.load_meeting_notes("meeting-1").unwrap().is_none());
    }
}
