use super::{Database, DatabaseError, MeetingState, WorkflowError};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, error::Error, fmt, str::FromStr};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMeetingInput {
    pub id: String,
    pub idempotency_key: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRecord {
    pub id: String,
    pub title: String,
    pub state: MeetingState,
    pub source_kind: String,
    pub duration_ms: Option<i64>,
    pub recording_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
    pub transcript_segment_count: i64,
}

#[derive(Debug)]
pub enum MeetingError {
    Database(DatabaseError),
    Sql(rusqlite::Error),
    Workflow(WorkflowError),
    NotFound(String),
    EmptyTitle,
    IdempotencyConflict(String),
    CannotArchive(MeetingState),
    NotArchived,
}

impl fmt::Display for MeetingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "{error}"),
            Self::Sql(error) => write!(formatter, "meeting database error: {error}"),
            Self::Workflow(error) => write!(formatter, "{error}"),
            Self::NotFound(id) => write!(formatter, "meeting not found: {id}"),
            Self::EmptyTitle => write!(formatter, "meeting title must not be empty"),
            Self::IdempotencyConflict(key) => {
                write!(
                    formatter,
                    "idempotency key was reused with different input: {key}"
                )
            }
            Self::CannotArchive(state) => {
                write!(
                    formatter,
                    "meeting cannot be archived while in state {state:?}"
                )
            }
            Self::NotArchived => write!(formatter, "meeting is not archived"),
        }
    }
}

impl Error for MeetingError {}

impl From<DatabaseError> for MeetingError {
    fn from(error: DatabaseError) -> Self {
        Self::Database(error)
    }
}

impl From<rusqlite::Error> for MeetingError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<WorkflowError> for MeetingError {
    fn from(error: WorkflowError) -> Self {
        Self::Workflow(error)
    }
}

impl Database {
    pub fn create_meeting(
        &self,
        input: &CreateMeetingInput,
    ) -> Result<MeetingRecord, MeetingError> {
        let title = input.title.trim();
        if title.is_empty() {
            return Err(MeetingError::EmptyTitle);
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = find_by_creation_key(&transaction, &input.idempotency_key)? {
            if existing.id != input.id || existing.title != title {
                return Err(MeetingError::IdempotencyConflict(
                    input.idempotency_key.clone(),
                ));
            }
            transaction.commit()?;
            return Ok(existing);
        }

        transaction.execute(
            "INSERT INTO meetings (id, creation_key, title)
             VALUES (?1, ?2, ?3)",
            params![input.id, input.idempotency_key, title],
        )?;
        let meeting = find_meeting(&transaction, &input.id)?
            .ok_or_else(|| MeetingError::NotFound(input.id.clone()))?;
        transaction.commit()?;
        Ok(meeting)
    }

    pub fn list_meetings(
        &self,
        include_archived: bool,
    ) -> Result<Vec<MeetingRecord>, MeetingError> {
        let connection = self.connection()?;
        let sql = if include_archived {
            "SELECT id, title, lifecycle_state, source_kind, duration_ms, recording_path, created_at, updated_at,
                (SELECT COUNT(*) FROM transcript_segments WHERE meeting_id = meetings.id)
             FROM meetings ORDER BY updated_at DESC, id"
        } else {
            "SELECT id, title, lifecycle_state, source_kind, duration_ms, recording_path, created_at, updated_at,
                (SELECT COUNT(*) FROM transcript_segments WHERE meeting_id = meetings.id)
             FROM meetings WHERE lifecycle_state != 'archived'
             ORDER BY updated_at DESC, id"
        };
        let mut statement = connection.prepare(sql)?;
        let rows = statement.query_map([], map_meeting_row)?;
        let mut meetings = Vec::new();
        for row in rows {
            let mut meeting = row?;
            meeting.tags = meeting_tags(&connection, &meeting.id)?;
            meetings.push(meeting);
        }
        Ok(meetings)
    }

    pub fn rename_meeting(&self, meeting_id: &str, title: &str) -> Result<(), MeetingError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(MeetingError::EmptyTitle);
        }
        let changed = self.connection()?.execute(
            "UPDATE meetings
             SET title = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?2",
            params![title, meeting_id],
        )?;
        if changed == 0 {
            Err(MeetingError::NotFound(meeting_id.to_owned()))
        } else {
            Ok(())
        }
    }

    pub fn replace_meeting_tags(
        &self,
        meeting_id: &str,
        tags: &[String],
    ) -> Result<Vec<String>, MeetingError> {
        let normalized = normalize_tags(tags);
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM meetings WHERE id = ?1)",
            [meeting_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(MeetingError::NotFound(meeting_id.to_owned()));
        }

        transaction.execute(
            "DELETE FROM meeting_tags WHERE meeting_id = ?1",
            [meeting_id],
        )?;
        for tag in &normalized {
            transaction.execute(
                "INSERT INTO meeting_tags (meeting_id, tag) VALUES (?1, ?2)",
                params![meeting_id, tag],
            )?;
        }
        transaction.execute(
            "UPDATE meetings
             SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            [meeting_id],
        )?;
        transaction.commit()?;
        Ok(normalized)
    }

    pub fn archive_meeting(
        &self,
        meeting_id: &str,
        idempotency_key: &str,
    ) -> Result<(), MeetingError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(target) = applied_transition_target(&transaction, meeting_id, idempotency_key)?
        {
            if target != "archived" {
                return Err(MeetingError::IdempotencyConflict(
                    idempotency_key.to_owned(),
                ));
            }
            transaction.commit()?;
            return Ok(());
        }

        let (stored_state, revision): (String, i64) = transaction
            .query_row(
                "SELECT lifecycle_state, state_revision FROM meetings WHERE id = ?1",
                [meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| MeetingError::NotFound(meeting_id.to_owned()))?;
        let current = MeetingState::from_str(&stored_state)?;
        if !matches!(
            current,
            MeetingState::Draft | MeetingState::Ready | MeetingState::Failed
        ) {
            return Err(MeetingError::CannotArchive(current));
        }

        apply_archive_transition(
            &transaction,
            meeting_id,
            idempotency_key,
            current,
            MeetingState::Archived,
            revision,
            Some(current),
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn reopen_meeting(
        &self,
        meeting_id: &str,
        idempotency_key: &str,
    ) -> Result<(), MeetingError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(target) = applied_transition_target(&transaction, meeting_id, idempotency_key)?
        {
            if target == "archived" {
                return Err(MeetingError::IdempotencyConflict(
                    idempotency_key.to_owned(),
                ));
            }
            transaction.commit()?;
            return Ok(());
        }
        let (stored_state, pre_archive_state, revision): (String, Option<String>, i64) =
            transaction
                .query_row(
                    "SELECT lifecycle_state, pre_archive_state, state_revision
                     FROM meetings WHERE id = ?1",
                    [meeting_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
                .ok_or_else(|| MeetingError::NotFound(meeting_id.to_owned()))?;
        let current = MeetingState::from_str(&stored_state)?;
        let target = pre_archive_state
            .as_deref()
            .map(MeetingState::from_str)
            .transpose()?
            .unwrap_or(MeetingState::Draft);

        if current != MeetingState::Archived {
            return Err(MeetingError::NotArchived);
        }

        apply_archive_transition(
            &transaction,
            meeting_id,
            idempotency_key,
            current,
            target,
            revision,
            None,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn delete_meeting(&self, meeting_id: &str) -> Result<bool, MeetingError> {
        Ok(self
            .connection()?
            .execute("DELETE FROM meetings WHERE id = ?1", [meeting_id])?
            == 1)
    }

    pub fn meeting_recording_path(&self, meeting_id: &str) -> Result<Option<String>, MeetingError> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT recording_path FROM meetings WHERE id = ?1",
                [meeting_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(MeetingError::from)?
            .flatten())
    }

    pub fn meeting_title(&self, meeting_id: &str) -> Result<String, MeetingError> {
        self.connection()?
            .query_row(
                "SELECT title FROM meetings WHERE id = ?1",
                [meeting_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| MeetingError::NotFound(meeting_id.to_owned()))
    }

    pub fn mark_recording_started(&self, meeting_id: &str) -> Result<(), MeetingError> {
        let changed = self.connection()?.execute(
            "UPDATE meetings
             SET started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 ended_at = NULL,
                 duration_ms = NULL,
                 recording_path = NULL,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            [meeting_id],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(MeetingError::NotFound(meeting_id.to_owned()))
        }
    }

    pub fn mark_recording_finished(
        &self,
        meeting_id: &str,
        duration_ms: i64,
        recording_path: &str,
    ) -> Result<(), MeetingError> {
        let changed = self.connection()?.execute(
            "UPDATE meetings
             SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 duration_ms = ?1,
                 recording_path = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?3",
            params![duration_ms, recording_path, meeting_id],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(MeetingError::NotFound(meeting_id.to_owned()))
        }
    }
}

fn apply_archive_transition(
    transaction: &Transaction<'_>,
    meeting_id: &str,
    idempotency_key: &str,
    current: MeetingState,
    target: MeetingState,
    revision: i64,
    pre_archive_state: Option<MeetingState>,
) -> Result<(), MeetingError> {
    let next_revision = revision + 1;
    transaction.execute(
        "UPDATE meetings
         SET lifecycle_state = ?1,
             pre_archive_state = ?2,
             state_revision = ?3,
             archived_at = CASE WHEN ?1 = 'archived'
                 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 ELSE NULL
             END,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?4 AND state_revision = ?5",
        params![
            target.as_str(),
            pre_archive_state.map(MeetingState::as_str),
            next_revision,
            meeting_id,
            revision
        ],
    )?;
    transaction.execute(
        "INSERT INTO meeting_transitions
            (idempotency_key, meeting_id, from_state, to_state, resulting_revision)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            idempotency_key,
            meeting_id,
            current.as_str(),
            target.as_str(),
            next_revision
        ],
    )?;
    Ok(())
}

fn applied_transition_target(
    transaction: &Transaction<'_>,
    meeting_id: &str,
    idempotency_key: &str,
) -> Result<Option<String>, MeetingError> {
    let existing: Option<(String, String)> = transaction
        .query_row(
            "SELECT meeting_id, to_state
             FROM meeting_transitions WHERE idempotency_key = ?1",
            [idempotency_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        Some((stored_meeting, stored_target)) if stored_meeting == meeting_id => {
            Ok(Some(stored_target))
        }
        Some(_) => Err(MeetingError::IdempotencyConflict(
            idempotency_key.to_owned(),
        )),
        None => Ok(None),
    }
}

fn find_by_creation_key(
    transaction: &Transaction<'_>,
    creation_key: &str,
) -> Result<Option<MeetingRecord>, MeetingError> {
    transaction
        .query_row(
            "SELECT id, title, lifecycle_state, source_kind, duration_ms, recording_path, created_at, updated_at,
                (SELECT COUNT(*) FROM transcript_segments WHERE meeting_id = meetings.id)
             FROM meetings WHERE creation_key = ?1",
            [creation_key],
            map_meeting_row,
        )
        .optional()
        .map_err(MeetingError::from)
}

fn find_meeting(
    transaction: &Transaction<'_>,
    meeting_id: &str,
) -> Result<Option<MeetingRecord>, MeetingError> {
    transaction
        .query_row(
            "SELECT id, title, lifecycle_state, source_kind, duration_ms, recording_path, created_at, updated_at,
                (SELECT COUNT(*) FROM transcript_segments WHERE meeting_id = meetings.id)
             FROM meetings WHERE id = ?1",
            [meeting_id],
            map_meeting_row,
        )
        .optional()
        .map_err(MeetingError::from)
}

fn map_meeting_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingRecord> {
    let stored_state: String = row.get(2)?;
    let state = MeetingState::from_str(&stored_state).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(MeetingRecord {
        id: row.get(0)?,
        title: row.get(1)?,
        state,
        source_kind: row.get(3)?,
        duration_ms: row.get(4)?,
        recording_path: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        tags: Vec::new(),
        transcript_segment_count: row.get(8)?,
    })
}

fn meeting_tags(
    connection: &rusqlite::Connection,
    meeting_id: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement =
        connection.prepare("SELECT tag FROM meeting_tags WHERE meeting_id = ?1 ORDER BY tag")?;
    let tags = statement
        .query_map([meeting_id], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tags)
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for tag in tags {
        let trimmed = tag.trim();
        let key = trimmed.to_lowercase();
        if !trimmed.is_empty() && seen.insert(key) {
            normalized.push(trimmed.to_owned());
        }
    }
    normalized.sort_by_key(|tag| tag.to_lowercase());
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> CreateMeetingInput {
        CreateMeetingInput {
            id: "meeting-1".to_owned(),
            idempotency_key: "create-meeting-1".to_owned(),
            title: " Weekly sync ".to_owned(),
        }
    }

    #[test]
    fn creates_and_replays_a_meeting() {
        let database = Database::open_in_memory().unwrap();
        let first = database.create_meeting(&input()).unwrap();
        let replay = database.create_meeting(&input()).unwrap();
        assert_eq!(first.title, "Weekly sync");
        assert_eq!(replay.id, first.id);
        assert_eq!(database.list_meetings(false).unwrap().len(), 1);
    }

    #[test]
    fn renames_and_normalizes_tags() {
        let database = Database::open_in_memory().unwrap();
        database.create_meeting(&input()).unwrap();
        database.rename_meeting("meeting-1", "Planning").unwrap();
        let tags = database
            .replace_meeting_tags(
                "meeting-1",
                &[
                    " Work ".to_owned(),
                    "weekly".to_owned(),
                    "work".to_owned(),
                    "".to_owned(),
                ],
            )
            .unwrap();
        let meeting = database.list_meetings(false).unwrap().remove(0);
        assert_eq!(meeting.title, "Planning");
        assert_eq!(tags, vec!["weekly", "Work"]);
        assert_eq!(meeting.tags, tags);
    }

    #[test]
    fn archives_reopens_and_deletes_idempotently() {
        let database = Database::open_in_memory().unwrap();
        database.create_meeting(&input()).unwrap();
        database.archive_meeting("meeting-1", "archive-1").unwrap();
        database.archive_meeting("meeting-1", "archive-1").unwrap();
        assert!(database.list_meetings(false).unwrap().is_empty());
        assert_eq!(
            database.list_meetings(true).unwrap()[0].state,
            MeetingState::Archived
        );

        database.reopen_meeting("meeting-1", "reopen-1").unwrap();
        assert_eq!(
            database.list_meetings(false).unwrap()[0].state,
            MeetingState::Draft
        );
        assert!(database.delete_meeting("meeting-1").unwrap());
        assert!(!database.delete_meeting("meeting-1").unwrap());
    }

    #[test]
    fn active_recordings_cannot_be_archived() {
        let database = Database::open_in_memory().unwrap();
        database.create_meeting(&input()).unwrap();
        database
            .transition_meeting(&super::super::MeetingTransitionRequest {
                meeting_id: "meeting-1".to_owned(),
                idempotency_key: "start-1".to_owned(),
                expected_state: MeetingState::Draft,
                next_state: MeetingState::Recording,
            })
            .unwrap();
        let error = database
            .archive_meeting("meeting-1", "archive-1")
            .unwrap_err();
        assert!(matches!(
            error,
            MeetingError::CannotArchive(MeetingState::Recording)
        ));
    }
}
