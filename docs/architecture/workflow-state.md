# Meeting and job state machines

The desktop backend owns lifecycle transitions. UI components and processing engines request transitions through typed Rust APIs; they must not update lifecycle columns directly.

## Meeting lifecycle

```text
draft → recording ↔ paused
             ↘       ↙
              processing → ready
                   ↘       ↙
                    failed
```

Both `ready` and `failed` meetings may return to `processing` when an output is regenerated or a failed pipeline is retried. Archive and reopen behavior remains owned by TI-0029 and will extend this graph with a later migration.

Every meeting transition includes the expected current state and a caller-generated idempotency key. The backend validates the edge, increments `state_revision`, and writes the new state plus its audit record in one transaction. Repeating the same request returns its original result; reusing its key for different input is rejected.

## Processing jobs

```text
queued → running → succeeded
          ↓
        failed → queued
```

Scheduling requires a stable idempotency key derived from the processing step and its input version. Repeated scheduling with the same inputs returns the existing job instead of duplicating work.

A worker claims a job using a unique run token and a time-limited lease. An expired lease allows another worker to reclaim the job and increments its attempt count. Completion is accepted only from the current run token, preventing a stalled worker from overwriting the result of a newer attempt. Successful and failed completion requests are themselves replay-safe, and retrying a failed job clears transient errors before requeueing it.

## Recovery rules

- Migration from schema version 1 requeues any job that was recorded as running because no durable lease existed yet.
- An interrupted job with an expired lease is eligible for a new claim.
- A stale worker receives an explicit lease error and cannot mutate the job.
- State and audit writes are transactional, so a partial transition is never visible.
