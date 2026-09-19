use super::{Database, DatabaseError};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use std::{error::Error, fmt};

#[derive(Debug)]
pub struct TranscriptSegmentInput {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegmentRecord {
    pub id: String,
    pub sequence_number: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker_label: Option<String>,
    pub source_text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSearchResult {
    pub meeting_id: String,
    pub meeting_title: String,
    pub segment_id: String,
    pub start_ms: i64,
    pub snippet: String,
}

#[derive(Debug)]
pub enum TranscriptError {
    Database(DatabaseError),
    Sql(rusqlite::Error),
    MeetingNotFound(String),
    InvalidSegment,
}

impl fmt::Display for TranscriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "{error}"),
            Self::Sql(error) => write!(formatter, "transcript database error: {error}"),
            Self::MeetingNotFound(id) => write!(formatter, "meeting not found: {id}"),
            Self::InvalidSegment => write!(
                formatter,
                "transcript segment has invalid timing or empty text"
            ),
        }
    }
}

impl Error for TranscriptError {}

impl From<DatabaseError> for TranscriptError {
    fn from(error: DatabaseError) -> Self {
        Self::Database(error)
    }
}

impl From<rusqlite::Error> for TranscriptError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

impl Database {
    pub fn store_source_transcript(
        &self,
        meeting_id: &str,
        segments: &[TranscriptSegmentInput],
    ) -> Result<Vec<TranscriptSegmentRecord>, TranscriptError> {
        if segments.iter().any(|segment| {
            segment.start_ms < 0
                || segment.end_ms < segment.start_ms
                || segment.text.trim().is_empty()
        }) {
            return Err(TranscriptError::InvalidSegment);
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
            [meeting_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(TranscriptError::MeetingNotFound(meeting_id.to_owned()));
        }

        let existing: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM transcript_segments WHERE meeting_id = ?1",
            [meeting_id],
            |row| row.get(0),
        )?;
        if existing == 0 {
            for (index, segment) in segments.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO transcript_segments
                        (id, meeting_id, sequence_number, start_ms, end_ms, speaker_label, language_code, source_text)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'Speaker 1', 'en', ?6)",
                    params![
                        format!("{meeting_id}:source:{index}"),
                        meeting_id,
                        index as i64,
                        segment.start_ms,
                        segment.end_ms,
                        segment.text.trim(),
                    ],
                )?;
            }
            transaction.execute(
                "UPDATE meetings SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                [meeting_id],
            )?;
        }
        let records = query_transcript(&transaction, meeting_id)?;
        transaction.commit()?;
        Ok(records)
    }

    pub fn list_transcript(
        &self,
        meeting_id: &str,
    ) -> Result<Vec<TranscriptSegmentRecord>, TranscriptError> {
        let connection = self.connection()?;
        let exists: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM meetings WHERE id = ?1",
                [meeting_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(TranscriptError::MeetingNotFound(meeting_id.to_owned()));
        }
        query_transcript(&connection, meeting_id)
    }

    pub fn search_transcripts(
        &self,
        query: &str,
    ) -> Result<Vec<TranscriptSearchResult>, TranscriptError> {
        let terms = query
            .split_whitespace()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ");
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT s.meeting_id, m.title, s.segment_id, t.start_ms,
                    snippet(transcript_search, 2, '<mark>', '</mark>', '…', 16)
             FROM transcript_search s
             JOIN meetings m ON m.id = s.meeting_id
             JOIN transcript_segments t ON t.id = s.segment_id
             WHERE transcript_search MATCH ?1 AND m.lifecycle_state != 'archived'
             ORDER BY rank LIMIT 50",
        )?;
        let results = statement
            .query_map([terms], |row| {
                Ok(TranscriptSearchResult {
                    meeting_id: row.get(0)?,
                    meeting_title: row.get(1)?,
                    segment_id: row.get(2)?,
                    start_ms: row.get(3)?,
                    snippet: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }
}

fn query_transcript(
    connection: &Connection,
    meeting_id: &str,
) -> Result<Vec<TranscriptSegmentRecord>, TranscriptError> {
    let mut statement = connection.prepare(
        "SELECT id, sequence_number, start_ms, end_ms, speaker_label, source_text
         FROM transcript_segments WHERE meeting_id = ?1 ORDER BY sequence_number",
    )?;
    let records = statement
        .query_map([meeting_id], |row| {
            Ok(TranscriptSegmentRecord {
                id: row.get(0)?,
                sequence_number: row.get(1)?,
                start_ms: row.get(2)?,
                end_ms: row.get(3)?,
                speaker_label: row.get(4)?,
                source_text: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CreateMeetingInput;

    #[test]
    fn source_transcript_is_stored_once_with_timestamps() {
        let database = Database::open_in_memory().unwrap();
        database
            .create_meeting(&CreateMeetingInput {
                id: "m1".to_owned(),
                idempotency_key: "create-m1".to_owned(),
                title: "Test".to_owned(),
            })
            .unwrap();
        let input = [TranscriptSegmentInput {
            start_ms: 100,
            end_ms: 900,
            text: " Hello ".to_owned(),
        }];
        let first = database.store_source_transcript("m1", &input).unwrap();
        let replay = database.store_source_transcript("m1", &input).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source_text, "Hello");
        assert_eq!(replay.len(), 1);
        let results = database.search_transcripts("hello").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].meeting_id, "m1");
    }
}
