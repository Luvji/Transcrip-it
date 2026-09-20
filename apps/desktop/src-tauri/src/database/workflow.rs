use super::{Database, DatabaseError};
use rusqlite::{params, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{error::Error, fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingState {
    Draft,
    Recording,
    Paused,
    Processing,
    Ready,
    Failed,
    Archived,
}

impl MeetingState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Recording => "recording",
            Self::Paused => "paused",
            Self::Processing => "processing",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Archived => "archived",
        }
    }

    fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Draft, Self::Recording)
                | (Self::Recording, Self::Paused | Self::Processing)
                | (Self::Paused, Self::Recording | Self::Processing)
                | (Self::Processing, Self::Ready | Self::Failed)
                | (Self::Failed | Self::Ready, Self::Processing)
                | (Self::Draft | Self::Ready | Self::Failed, Self::Archived)
                | (Self::Archived, Self::Draft | Self::Ready | Self::Failed)
        )
    }
}

impl FromStr for MeetingState {
    type Err = WorkflowError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "recording" => Ok(Self::Recording),
            "paused" => Ok(Self::Paused),
            "processing" => Ok(Self::Processing),
            "ready" => Ok(Self::Ready),
            "failed" => Ok(Self::Failed),
            "archived" => Ok(Self::Archived),
            _ => Err(WorkflowError::InvalidStoredState(value.to_owned())),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTransitionRequest {
    pub meeting_id: String,
    pub idempotency_key: String,
    pub expected_state: MeetingState,
    pub next_state: MeetingState,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTransition {
    pub meeting_id: String,
    pub state: MeetingState,
    pub revision: i64,
    pub replayed: bool,
}

#[derive(Debug)]
pub struct JobSchedule {
    pub id: String,
    pub idempotency_key: String,
    pub meeting_id: Option<String>,
    pub job_kind: String,
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobCompletion<'a> {
    Succeeded,
    Failed {
        error_code: &'a str,
        error_message: &'a str,
    },
}

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for JobState {
    type Err = WorkflowError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(WorkflowError::InvalidStoredState(value.to_owned())),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSnapshot {
    pub id: String,
    pub meeting_id: Option<String>,
    pub job_kind: String,
    pub state: JobState,
    pub attempt_count: i64,
    pub run_token: Option<String>,
    pub replayed: bool,
}

#[derive(Debug)]
pub struct JobClaim<'a> {
    pub job_kind: &'a str,
    pub run_token: &'a str,
    pub lease_seconds: i64,
}

#[derive(Debug)]
pub enum WorkflowError {
    Database(DatabaseError),
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    InvalidStoredState(String),
    InvalidTransition {
        current: MeetingState,
        requested: MeetingState,
    },
    StateConflict {
        expected: MeetingState,
        actual: MeetingState,
    },
    IdempotencyConflict(String),
    MeetingNotFound(String),
    JobNotFound(String),
    InvalidLeaseDuration,
    StaleJobLease(String),
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "{error}"),
            Self::Sql(error) => write!(formatter, "workflow database error: {error}"),
            Self::Json(error) => write!(formatter, "workflow JSON error: {error}"),
            Self::InvalidStoredState(state) => write!(formatter, "invalid stored state: {state}"),
            Self::InvalidTransition { current, requested } => {
                write!(
                    formatter,
                    "cannot transition from {current:?} to {requested:?}"
                )
            }
            Self::StateConflict { expected, actual } => write!(
                formatter,
                "meeting state changed: expected {expected:?}, found {actual:?}"
            ),
            Self::IdempotencyConflict(key) => {
                write!(
                    formatter,
                    "idempotency key was reused with different input: {key}"
                )
            }
            Self::MeetingNotFound(id) => write!(formatter, "meeting not found: {id}"),
            Self::JobNotFound(id) => write!(formatter, "job not found: {id}"),
            Self::InvalidLeaseDuration => write!(formatter, "job lease duration must be positive"),
            Self::StaleJobLease(id) => write!(formatter, "job lease is no longer current: {id}"),
        }
    }
}

impl Error for WorkflowError {}

impl From<DatabaseError> for WorkflowError {
    fn from(error: DatabaseError) -> Self {
        Self::Database(error)
    }
}

impl From<rusqlite::Error> for WorkflowError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<serde_json::Error> for WorkflowError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl Database {
    pub fn recover_interrupted_meetings(&self) -> Result<usize, WorkflowError> {
        let interrupted = {
            let connection = self.connection()?;
            let mut statement = connection.prepare(
                "SELECT id, lifecycle_state, state_revision FROM meetings
                 WHERE lifecycle_state IN ('recording', 'paused', 'processing')",
            )?;
            let meetings = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            meetings
        };
        for (meeting_id, stored_state, stored_revision) in &interrupted {
            let state = MeetingState::from_str(stored_state)?;
            let mut recovery_revision = *stored_revision;
            if matches!(state, MeetingState::Recording | MeetingState::Paused) {
                self.transition_meeting(&MeetingTransitionRequest {
                    meeting_id: meeting_id.clone(),
                    idempotency_key: format!(
                        "startup-recovery-process:{meeting_id}:{recovery_revision}"
                    ),
                    expected_state: state,
                    next_state: MeetingState::Processing,
                })?;
                recovery_revision += 1;
            }
            self.transition_meeting(&MeetingTransitionRequest {
                meeting_id: meeting_id.clone(),
                idempotency_key: format!("startup-recovery-fail:{meeting_id}:{recovery_revision}"),
                expected_state: MeetingState::Processing,
                next_state: MeetingState::Failed,
            })?;
        }
        Ok(interrupted.len())
    }

    pub fn transition_meeting(
        &self,
        request: &MeetingTransitionRequest,
    ) -> Result<MeetingTransition, WorkflowError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = find_transition(&transaction, &request.idempotency_key)? {
            if existing.meeting_id != request.meeting_id
                || existing.from_state != request.expected_state
                || existing.to_state != request.next_state
            {
                return Err(WorkflowError::IdempotencyConflict(
                    request.idempotency_key.clone(),
                ));
            }

            transaction.commit()?;
            return Ok(MeetingTransition {
                meeting_id: existing.meeting_id,
                state: existing.to_state,
                revision: existing.resulting_revision,
                replayed: true,
            });
        }

        let (stored_state, revision): (String, i64) = transaction
            .query_row(
                "SELECT lifecycle_state, state_revision FROM meetings WHERE id = ?1",
                [&request.meeting_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| WorkflowError::MeetingNotFound(request.meeting_id.clone()))?;
        let current_state = MeetingState::from_str(&stored_state)?;

        if current_state != request.expected_state {
            return Err(WorkflowError::StateConflict {
                expected: request.expected_state,
                actual: current_state,
            });
        }
        if !current_state.can_transition_to(request.next_state) {
            return Err(WorkflowError::InvalidTransition {
                current: current_state,
                requested: request.next_state,
            });
        }

        let next_revision = revision + 1;
        let updated = transaction.execute(
            "UPDATE meetings
             SET lifecycle_state = ?1,
                 state_revision = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?3 AND state_revision = ?4",
            params![
                request.next_state.as_str(),
                next_revision,
                request.meeting_id,
                revision
            ],
        )?;
        if updated != 1 {
            return Err(WorkflowError::StateConflict {
                expected: request.expected_state,
                actual: current_state,
            });
        }

        transaction.execute(
            "INSERT INTO meeting_transitions
                (idempotency_key, meeting_id, from_state, to_state, resulting_revision)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                request.idempotency_key,
                request.meeting_id,
                current_state.as_str(),
                request.next_state.as_str(),
                next_revision
            ],
        )?;
        transaction.commit()?;

        Ok(MeetingTransition {
            meeting_id: request.meeting_id.clone(),
            state: request.next_state,
            revision: next_revision,
            replayed: false,
        })
    }

    pub fn schedule_job(&self, schedule: &JobSchedule) -> Result<JobSnapshot, WorkflowError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let payload = serde_json::to_string(&schedule.payload)?;

        if let Some(existing) =
            find_job_by_idempotency_key(&transaction, &schedule.idempotency_key)?
        {
            let stored_payload: String = transaction.query_row(
                "SELECT payload_json FROM jobs WHERE id = ?1",
                [&existing.id],
                |row| row.get(0),
            )?;
            let stored_payload: Value = serde_json::from_str(&stored_payload)?;
            if existing.id != schedule.id
                || existing.meeting_id != schedule.meeting_id
                || existing.job_kind != schedule.job_kind
                || stored_payload != schedule.payload
            {
                return Err(WorkflowError::IdempotencyConflict(
                    schedule.idempotency_key.clone(),
                ));
            }

            transaction.commit()?;
            return Ok(JobSnapshot {
                replayed: true,
                ..existing
            });
        }

        transaction.execute(
            "INSERT INTO jobs
                (id, idempotency_key, meeting_id, job_kind, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                schedule.id,
                schedule.idempotency_key,
                schedule.meeting_id,
                schedule.job_kind,
                payload
            ],
        )?;
        let job = find_job(&transaction, &schedule.id)?
            .ok_or_else(|| WorkflowError::JobNotFound(schedule.id.clone()))?;
        transaction.commit()?;
        Ok(job)
    }

    pub fn claim_next_job(
        &self,
        claim: &JobClaim<'_>,
    ) -> Result<Option<JobSnapshot>, WorkflowError> {
        if claim.lease_seconds <= 0 {
            return Err(WorkflowError::InvalidLeaseDuration);
        }

        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job_id: Option<String> = transaction
            .query_row(
                "SELECT id
                 FROM jobs
                 WHERE job_kind = ?1
                   AND available_at <= unixepoch()
                   AND (
                       state = 'queued'
                       OR (state = 'running' AND lease_expires_at <= unixepoch())
                   )
                 ORDER BY available_at, created_at, id
                 LIMIT 1",
                [claim.job_kind],
                |row| row.get(0),
            )
            .optional()?;

        let Some(job_id) = job_id else {
            transaction.commit()?;
            return Ok(None);
        };

        transaction.execute(
            "UPDATE jobs
             SET state = 'running',
                 run_token = ?1,
                 lease_expires_at = unixepoch() + ?2,
                 attempt_count = attempt_count + 1,
                 started_at = COALESCE(started_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 finished_at = NULL,
                 error_code = NULL,
                 error_message = NULL,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?3",
            params![claim.run_token, claim.lease_seconds, job_id],
        )?;
        let job = find_job(&transaction, &job_id)?
            .ok_or_else(|| WorkflowError::JobNotFound(job_id.clone()))?;
        transaction.commit()?;
        Ok(Some(job))
    }

    pub fn complete_job(
        &self,
        job_id: &str,
        run_token: &str,
        completion: JobCompletion<'_>,
    ) -> Result<JobSnapshot, WorkflowError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = find_job(&transaction, job_id)?
            .ok_or_else(|| WorkflowError::JobNotFound(job_id.to_owned()))?;
        let target_state = match completion {
            JobCompletion::Succeeded => JobState::Succeeded,
            JobCompletion::Failed { .. } => JobState::Failed,
        };

        if existing.state == target_state && existing.run_token.as_deref() == Some(run_token) {
            transaction.commit()?;
            return Ok(JobSnapshot {
                replayed: true,
                ..existing
            });
        }
        if existing.state != JobState::Running || existing.run_token.as_deref() != Some(run_token) {
            return Err(WorkflowError::StaleJobLease(job_id.to_owned()));
        }

        let (error_code, error_message) = match completion {
            JobCompletion::Succeeded => (None, None),
            JobCompletion::Failed {
                error_code,
                error_message,
            } => (Some(error_code), Some(error_message)),
        };
        transaction.execute(
            "UPDATE jobs
             SET state = ?1,
                 progress = CASE WHEN ?1 = 'succeeded' THEN 1.0 ELSE progress END,
                 lease_expires_at = NULL,
                 error_code = ?2,
                 error_message = ?3,
                 finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?4 AND state = 'running' AND run_token = ?5",
            params![
                target_state.as_str(),
                error_code,
                error_message,
                job_id,
                run_token
            ],
        )?;
        let job = find_job(&transaction, job_id)?
            .ok_or_else(|| WorkflowError::JobNotFound(job_id.to_owned()))?;
        transaction.commit()?;
        Ok(job)
    }

    pub fn retry_job(&self, job_id: &str) -> Result<JobSnapshot, WorkflowError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = find_job(&transaction, job_id)?
            .ok_or_else(|| WorkflowError::JobNotFound(job_id.to_owned()))?;

        if existing.state == JobState::Queued {
            transaction.commit()?;
            return Ok(JobSnapshot {
                replayed: true,
                ..existing
            });
        }
        if existing.state != JobState::Failed {
            return Err(WorkflowError::StaleJobLease(job_id.to_owned()));
        }

        transaction.execute(
            "UPDATE jobs
             SET state = 'queued',
                 progress = 0.0,
                 run_token = NULL,
                 lease_expires_at = NULL,
                 error_code = NULL,
                 error_message = NULL,
                 finished_at = NULL,
                 available_at = unixepoch(),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND state = 'failed'",
            [job_id],
        )?;
        let job = find_job(&transaction, job_id)?
            .ok_or_else(|| WorkflowError::JobNotFound(job_id.to_owned()))?;
        transaction.commit()?;
        Ok(job)
    }
}

struct StoredTransition {
    meeting_id: String,
    from_state: MeetingState,
    to_state: MeetingState,
    resulting_revision: i64,
}

fn find_transition(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
) -> Result<Option<StoredTransition>, WorkflowError> {
    let row: Option<(String, String, String, i64)> = transaction
        .query_row(
            "SELECT meeting_id, from_state, to_state, resulting_revision
             FROM meeting_transitions
             WHERE idempotency_key = ?1",
            [idempotency_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;

    row.map(|(meeting_id, from_state, to_state, resulting_revision)| {
        Ok(StoredTransition {
            meeting_id,
            from_state: MeetingState::from_str(&from_state)?,
            to_state: MeetingState::from_str(&to_state)?,
            resulting_revision,
        })
    })
    .transpose()
}

fn find_job_by_idempotency_key(
    transaction: &Transaction<'_>,
    idempotency_key: &str,
) -> Result<Option<JobSnapshot>, WorkflowError> {
    transaction
        .query_row(
            "SELECT id, meeting_id, job_kind, state, attempt_count, run_token
             FROM jobs WHERE idempotency_key = ?1",
            [idempotency_key],
            map_job,
        )
        .optional()
        .map_err(WorkflowError::from)
}

fn find_job(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<Option<JobSnapshot>, WorkflowError> {
    transaction
        .query_row(
            "SELECT id, meeting_id, job_kind, state, attempt_count, run_token
             FROM jobs WHERE id = ?1",
            [job_id],
            map_job,
        )
        .optional()
        .map_err(WorkflowError::from)
}

fn map_job(row: &Row<'_>) -> rusqlite::Result<JobSnapshot> {
    let stored_state: String = row.get(3)?;
    let state = JobState::from_str(&stored_state).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(JobSnapshot {
        id: row.get(0)?,
        meeting_id: row.get(1)?,
        job_kind: row.get(2)?,
        state,
        attempt_count: row.get(4)?,
        run_token: row.get(5)?,
        replayed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_with_meeting() -> Database {
        let database = Database::open_in_memory().unwrap();
        database
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO meetings (id, title) VALUES ('meeting-1', 'Weekly sync')",
                [],
            )
            .unwrap();
        database
    }

    fn schedule() -> JobSchedule {
        JobSchedule {
            id: "job-1".to_owned(),
            idempotency_key: "transcribe:meeting-1:source-v1".to_owned(),
            meeting_id: Some("meeting-1".to_owned()),
            job_kind: "transcription".to_owned(),
            payload: serde_json::json!({ "sourceVersion": 1 }),
        }
    }

    #[test]
    fn meeting_transitions_are_validated_and_idempotent() {
        let database = database_with_meeting();
        let request = MeetingTransitionRequest {
            meeting_id: "meeting-1".to_owned(),
            idempotency_key: "start-recording-1".to_owned(),
            expected_state: MeetingState::Draft,
            next_state: MeetingState::Recording,
        };

        let first = database.transition_meeting(&request).unwrap();
        let replay = database.transition_meeting(&request).unwrap();

        assert_eq!(first.state, MeetingState::Recording);
        assert_eq!(first.revision, 1);
        assert!(!first.replayed);
        assert!(replay.replayed);
        assert_eq!(replay.revision, 1);

        let transition_count: i64 = database
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM meeting_transitions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(transition_count, 1);
    }

    #[test]
    fn meeting_transitions_reject_invalid_edges() {
        let database = database_with_meeting();
        let error = database
            .transition_meeting(&MeetingTransitionRequest {
                meeting_id: "meeting-1".to_owned(),
                idempotency_key: "skip-to-ready".to_owned(),
                expected_state: MeetingState::Draft,
                next_state: MeetingState::Ready,
            })
            .unwrap_err();

        assert!(matches!(error, WorkflowError::InvalidTransition { .. }));
    }

    #[test]
    fn startup_recovery_moves_interrupted_meetings_to_failed() {
        let database = database_with_meeting();
        database
            .transition_meeting(&MeetingTransitionRequest {
                meeting_id: "meeting-1".to_owned(),
                idempotency_key: "start-before-crash".to_owned(),
                expected_state: MeetingState::Draft,
                next_state: MeetingState::Recording,
            })
            .unwrap();
        assert_eq!(database.recover_interrupted_meetings().unwrap(), 1);
        assert_eq!(
            database.meeting_state("meeting-1").unwrap(),
            MeetingState::Failed
        );
        assert_eq!(database.recover_interrupted_meetings().unwrap(), 0);

        database
            .transition_meeting(&MeetingTransitionRequest {
                meeting_id: "meeting-1".to_owned(),
                idempotency_key: "retry-before-second-crash".to_owned(),
                expected_state: MeetingState::Failed,
                next_state: MeetingState::Processing,
            })
            .unwrap();
        assert_eq!(database.recover_interrupted_meetings().unwrap(), 1);
        assert_eq!(
            database.meeting_state("meeting-1").unwrap(),
            MeetingState::Failed
        );
    }

    #[test]
    fn job_scheduling_is_idempotent() {
        let database = database_with_meeting();
        let schedule = schedule();

        let first = database.schedule_job(&schedule).unwrap();
        let replay = database.schedule_job(&schedule).unwrap();

        assert_eq!(first.state, JobState::Queued);
        assert!(!first.replayed);
        assert!(replay.replayed);
        let count: i64 = database
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn stale_workers_cannot_complete_a_reclaimed_job() {
        let database = database_with_meeting();
        database.schedule_job(&schedule()).unwrap();

        let first = database
            .claim_next_job(&JobClaim {
                job_kind: "transcription",
                run_token: "worker-1",
                lease_seconds: 60,
            })
            .unwrap()
            .unwrap();
        assert_eq!(first.attempt_count, 1);

        database
            .connection()
            .unwrap()
            .execute(
                "UPDATE jobs SET lease_expires_at = 0 WHERE id = 'job-1'",
                [],
            )
            .unwrap();
        let reclaimed = database
            .claim_next_job(&JobClaim {
                job_kind: "transcription",
                run_token: "worker-2",
                lease_seconds: 60,
            })
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.attempt_count, 2);

        let stale = database.complete_job("job-1", "worker-1", JobCompletion::Succeeded);
        assert!(matches!(stale, Err(WorkflowError::StaleJobLease(_))));

        let completed = database
            .complete_job("job-1", "worker-2", JobCompletion::Succeeded)
            .unwrap();
        let replay = database
            .complete_job("job-1", "worker-2", JobCompletion::Succeeded)
            .unwrap();
        assert_eq!(completed.state, JobState::Succeeded);
        assert!(replay.replayed);
    }

    #[test]
    fn failed_jobs_can_be_retried_safely() {
        let database = database_with_meeting();
        database.schedule_job(&schedule()).unwrap();
        database
            .claim_next_job(&JobClaim {
                job_kind: "transcription",
                run_token: "worker-1",
                lease_seconds: 60,
            })
            .unwrap();
        database
            .complete_job(
                "job-1",
                "worker-1",
                JobCompletion::Failed {
                    error_code: "engine_unavailable",
                    error_message: "engine stopped",
                },
            )
            .unwrap();

        let queued = database.retry_job("job-1").unwrap();
        let replay = database.retry_job("job-1").unwrap();
        assert_eq!(queued.state, JobState::Queued);
        assert!(replay.replayed);
    }
}
