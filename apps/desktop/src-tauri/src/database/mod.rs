use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;
use std::{
    error::Error,
    fmt, fs,
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "workflow_state",
        sql: include_str!("migrations/0002_workflow_state.sql"),
    },
];

mod workflow;
pub use workflow::{
    JobClaim, JobCompletion, JobSchedule, JobSnapshot, MeetingState, MeetingTransition,
    MeetingTransitionRequest, WorkflowError,
};

#[derive(Debug)]
pub enum DatabaseError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    UnsupportedSchemaVersion { found: i64, supported: i64 },
    LockPoisoned,
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "database filesystem error: {error}"),
            Self::Sql(error) => write!(formatter, "database error: {error}"),
            Self::UnsupportedSchemaVersion { found, supported } => write!(
                formatter,
                "database schema version {found} is newer than supported version {supported}"
            ),
            Self::LockPoisoned => write!(formatter, "database connection lock is poisoned"),
        }
    }
}

impl Error for DatabaseError {}

impl From<std::io::Error> for DatabaseError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for DatabaseError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

pub struct Database {
    connection: Mutex<Connection>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStatus {
    ready: bool,
    schema_version: i64,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    fn from_connection(mut connection: Connection) -> Result<Self, DatabaseError> {
        configure_connection(&connection)?;
        apply_migrations(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, DatabaseError> {
        self.connection
            .lock()
            .map_err(|_| DatabaseError::LockPoisoned)
    }

    pub fn status(&self) -> Result<DatabaseStatus, DatabaseError> {
        let connection = self.connection()?;
        let schema_version = current_schema_version(&connection)?;

        Ok(DatabaseStatus {
            ready: true,
            schema_version,
        })
    }

    #[cfg(test)]
    fn open_in_memory() -> Result<Self, DatabaseError> {
        Self::from_connection(Connection::open_in_memory()?)
    }
}

fn configure_connection(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(())
}

fn apply_migrations(connection: &mut Connection) -> Result<(), DatabaseError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY CHECK (version > 0),
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );",
    )?;

    let current = current_schema_version(connection)?;
    let latest = MIGRATIONS.last().map_or(0, |migration| migration.version);
    if current > latest {
        return Err(DatabaseError::UnsupportedSchemaVersion {
            found: current,
            supported: latest,
        });
    }

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version > current)
    {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            (migration.version, migration.name),
        )?;
        transaction.commit()?;
    }

    Ok(())
}

fn current_schema_version(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_the_initial_schema() {
        let database = Database::open_in_memory().expect("database should initialize");
        let connection = database.connection().expect("connection should lock");
        let tables = [
            "actions",
            "jobs",
            "meetings",
            "schema_migrations",
            "settings",
            "transcript_segments",
        ];

        for table in tables {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
                    [table],
                    |row| row.get(0),
                )
                .expect("schema query should succeed");
            assert!(exists, "expected table {table}");
        }

        assert_eq!(current_schema_version(&connection).unwrap(), 2);
    }

    #[test]
    fn applying_migrations_is_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection).unwrap();

        apply_migrations(&mut connection).unwrap();
        apply_migrations(&mut connection).unwrap();

        let migration_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(migration_count, 2);
    }

    #[test]
    fn upgrades_an_interrupted_version_one_job_safely() {
        let mut connection = Connection::open_in_memory().unwrap();
        configure_connection(&connection).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY CHECK (version > 0),
                    name TEXT NOT NULL UNIQUE,
                    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                );",
            )
            .unwrap();
        connection.execute_batch(MIGRATIONS[0].sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name) VALUES (1, 'initial')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO meetings (id, title) VALUES ('meeting-1', 'Interrupted meeting')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO jobs (id, meeting_id, job_kind, state)
                 VALUES ('job-1', 'meeting-1', 'transcription', 'running')",
                [],
            )
            .unwrap();

        apply_migrations(&mut connection).unwrap();

        let upgraded: (String, String, Option<String>) = connection
            .query_row(
                "SELECT idempotency_key, state, run_token FROM jobs WHERE id = 'job-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(upgraded, ("job-1".to_owned(), "queued".to_owned(), None));
        assert_eq!(current_schema_version(&connection).unwrap(), 2);
    }

    #[test]
    fn deleting_a_meeting_removes_its_owned_records() {
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
                 VALUES ('segment-1', 'meeting-1', 0, 0, 1000, 'Hello')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO jobs (id, idempotency_key, meeting_id, job_kind)
                 VALUES ('job-1', 'job-1', 'meeting-1', 'transcription')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO actions (id, meeting_id, source_segment_id, action_text)
                 VALUES ('action-1', 'meeting-1', 'segment-1', 'Send notes')",
                [],
            )
            .unwrap();

        connection
            .execute("DELETE FROM meetings WHERE id = 'meeting-1'", [])
            .unwrap();

        for table in ["transcript_segments", "jobs", "actions"] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "expected {table} rows to cascade");
        }
    }
}
