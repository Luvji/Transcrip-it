use super::{Database, DatabaseError};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::Serialize;
use std::{error::Error, fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeKind {
    Correction,
    Translation,
    Romanization,
}

impl DerivativeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Correction => "correction",
            Self::Translation => "translation",
            Self::Romanization => "romanization",
        }
    }
}

impl FromStr for DerivativeKind {
    type Err = EvidenceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "correction" => Ok(Self::Correction),
            "translation" => Ok(Self::Translation),
            "romanization" => Ok(Self::Romanization),
            _ => Err(EvidenceError::InvalidStoredValue(value.to_owned())),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    User,
    Model,
}

impl AuthorKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Model => "model",
        }
    }
}

impl FromStr for AuthorKind {
    type Err = EvidenceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "user" => Ok(Self::User),
            "model" => Ok(Self::Model),
            _ => Err(EvidenceError::InvalidStoredValue(value.to_owned())),
        }
    }
}

#[derive(Debug)]
pub struct SegmentDerivativeInput {
    pub id: String,
    pub idempotency_key: String,
    pub segment_id: String,
    pub kind: DerivativeKind,
    pub author: AuthorKind,
    pub text: String,
    pub language_code: Option<String>,
    pub model_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentDerivative {
    pub id: String,
    pub segment_id: String,
    pub kind: DerivativeKind,
    pub author: AuthorKind,
    pub revision_number: i64,
    pub text: String,
    pub language_code: Option<String>,
    pub model_id: Option<String>,
    pub supersedes_id: Option<String>,
    pub replayed: bool,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentText {
    pub segment_id: String,
    pub source_text: String,
    pub display_text: String,
    pub active_correction_id: Option<String>,
}

#[derive(Debug)]
pub enum EvidenceError {
    Database(DatabaseError),
    Sql(rusqlite::Error),
    SegmentNotFound(String),
    EmptyText,
    IdempotencyConflict(String),
    InvalidStoredValue(String),
}

impl fmt::Display for EvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "{error}"),
            Self::Sql(error) => write!(formatter, "evidence database error: {error}"),
            Self::SegmentNotFound(id) => write!(formatter, "transcript segment not found: {id}"),
            Self::EmptyText => write!(formatter, "derived transcript text must not be empty"),
            Self::IdempotencyConflict(key) => {
                write!(
                    formatter,
                    "idempotency key was reused with different input: {key}"
                )
            }
            Self::InvalidStoredValue(value) => {
                write!(formatter, "invalid stored evidence value: {value}")
            }
        }
    }
}

impl Error for EvidenceError {}

impl From<DatabaseError> for EvidenceError {
    fn from(error: DatabaseError) -> Self {
        Self::Database(error)
    }
}

impl From<rusqlite::Error> for EvidenceError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

impl Database {
    pub fn append_segment_derivative(
        &self,
        input: &SegmentDerivativeInput,
    ) -> Result<SegmentDerivative, EvidenceError> {
        if input.text.trim().is_empty() {
            return Err(EvidenceError::EmptyText);
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = find_by_idempotency_key(&transaction, &input.idempotency_key)? {
            if existing.id != input.id
                || existing.segment_id != input.segment_id
                || existing.kind != input.kind
                || existing.author != input.author
                || existing.text != input.text
                || existing.language_code.as_deref() != input.language_code.as_deref()
                || existing.model_id.as_deref() != input.model_id.as_deref()
            {
                return Err(EvidenceError::IdempotencyConflict(
                    input.idempotency_key.clone(),
                ));
            }

            transaction.commit()?;
            return Ok(SegmentDerivative {
                replayed: true,
                ..existing
            });
        }

        let segment_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM transcript_segments WHERE id = ?1)",
            [&input.segment_id],
            |row| row.get(0),
        )?;
        if !segment_exists {
            return Err(EvidenceError::SegmentNotFound(input.segment_id.clone()));
        }

        let (next_revision, supersedes_id): (i64, Option<String>) = transaction.query_row(
            "SELECT
                COALESCE(MAX(revision_number), 0) + 1,
                (SELECT id
                 FROM transcript_derivatives
                 WHERE segment_id = ?1 AND derivative_kind = ?2
                 ORDER BY revision_number DESC
                 LIMIT 1)
             FROM transcript_derivatives
             WHERE segment_id = ?1 AND derivative_kind = ?2",
            params![input.segment_id, input.kind.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        transaction.execute(
            "INSERT INTO transcript_derivatives (
                id,
                idempotency_key,
                segment_id,
                derivative_kind,
                author_kind,
                revision_number,
                derived_text,
                language_code,
                model_id,
                supersedes_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                input.id,
                input.idempotency_key,
                input.segment_id,
                input.kind.as_str(),
                input.author.as_str(),
                next_revision,
                input.text,
                input.language_code,
                input.model_id,
                supersedes_id
            ],
        )?;
        let derivative = find_by_id(&transaction, &input.id)?
            .ok_or_else(|| EvidenceError::SegmentNotFound(input.segment_id.clone()))?;
        transaction.commit()?;
        Ok(derivative)
    }

    pub fn segment_text(&self, segment_id: &str) -> Result<SegmentText, EvidenceError> {
        self.connection()?
            .query_row(
                "SELECT segment_id, source_text, display_text, active_correction_id
                 FROM transcript_current_text
                 WHERE segment_id = ?1",
                [segment_id],
                |row| {
                    Ok(SegmentText {
                        segment_id: row.get(0)?,
                        source_text: row.get(1)?,
                        display_text: row.get(2)?,
                        active_correction_id: row.get(3)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| EvidenceError::SegmentNotFound(segment_id.to_owned()))
    }
}

fn find_by_idempotency_key(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
) -> Result<Option<SegmentDerivative>, EvidenceError> {
    find_derivative(
        transaction,
        "SELECT id, segment_id, derivative_kind, author_kind, revision_number,
                derived_text, language_code, model_id, supersedes_id
         FROM transcript_derivatives WHERE idempotency_key = ?1",
        idempotency_key,
    )
}

fn find_by_id(
    transaction: &Transaction<'_>,
    derivative_id: &str,
) -> Result<Option<SegmentDerivative>, EvidenceError> {
    find_derivative(
        transaction,
        "SELECT id, segment_id, derivative_kind, author_kind, revision_number,
                derived_text, language_code, model_id, supersedes_id
         FROM transcript_derivatives WHERE id = ?1",
        derivative_id,
    )
}

fn find_derivative(
    transaction: &Transaction<'_>,
    sql: &str,
    value: &str,
) -> Result<Option<SegmentDerivative>, EvidenceError> {
    transaction
        .query_row(sql, [value], map_derivative)
        .optional()
        .map_err(EvidenceError::from)
}

fn map_derivative(row: &Row<'_>) -> rusqlite::Result<SegmentDerivative> {
    let stored_kind: String = row.get(2)?;
    let stored_author: String = row.get(3)?;
    let kind = DerivativeKind::from_str(&stored_kind).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let author = AuthorKind::from_str(&stored_author).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(SegmentDerivative {
        id: row.get(0)?,
        segment_id: row.get(1)?,
        kind,
        author,
        revision_number: row.get(4)?,
        text: row.get(5)?,
        language_code: row.get(6)?,
        model_id: row.get(7)?,
        supersedes_id: row.get(8)?,
        replayed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_with_segment() -> Database {
        let database = Database::open_in_memory().unwrap();
        let connection = database.connection().unwrap();
        connection
            .execute(
                "INSERT INTO meetings (id, title) VALUES ('meeting-1', 'Weekly sync')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO transcript_segments
                    (id, meeting_id, sequence_number, start_ms, end_ms, source_text)
                 VALUES ('segment-1', 'meeting-1', 0, 0, 1000, 'Original words')",
                [],
            )
            .unwrap();
        drop(connection);
        database
    }

    fn correction(id: &str, key: &str, text: &str) -> SegmentDerivativeInput {
        SegmentDerivativeInput {
            id: id.to_owned(),
            idempotency_key: key.to_owned(),
            segment_id: "segment-1".to_owned(),
            kind: DerivativeKind::Correction,
            author: AuthorKind::User,
            text: text.to_owned(),
            language_code: Some("en".to_owned()),
            model_id: None,
        }
    }

    #[test]
    fn source_evidence_cannot_be_overwritten() {
        let database = database_with_segment();
        let error = database
            .connection()
            .unwrap()
            .execute(
                "UPDATE transcript_segments SET source_text = 'Changed' WHERE id = 'segment-1'",
                [],
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("source transcript evidence is immutable"));
    }

    #[test]
    fn corrections_are_revisioned_without_changing_source_text() {
        let database = database_with_segment();
        let first = database
            .append_segment_derivative(&correction("edit-1", "edit-key-1", "Correct words"))
            .unwrap();
        let second = database
            .append_segment_derivative(&correction("edit-2", "edit-key-2", "Final words"))
            .unwrap();
        let current = database.segment_text("segment-1").unwrap();

        assert_eq!(first.revision_number, 1);
        assert_eq!(second.revision_number, 2);
        assert_eq!(second.supersedes_id.as_deref(), Some("edit-1"));
        assert_eq!(current.source_text, "Original words");
        assert_eq!(current.display_text, "Final words");
        assert_eq!(current.active_correction_id.as_deref(), Some("edit-2"));
    }

    #[test]
    fn derivative_writes_are_idempotent() {
        let database = database_with_segment();
        let input = correction("edit-1", "edit-key-1", "Correct words");

        let first = database.append_segment_derivative(&input).unwrap();
        let replay = database.append_segment_derivative(&input).unwrap();

        assert!(!first.replayed);
        assert!(replay.replayed);
        let count: i64 = database
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM transcript_derivatives", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
}
