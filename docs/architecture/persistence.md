# Local persistence

Transcrip-it stores application state in `transcrip-it.sqlite3` beneath Tauri's platform-specific application-data directory. The database is local to the current operating-system user and is not created in the source repository.

## Connection policy

The Rust backend owns the SQLite connection for the lifetime of the application. Every connection enables foreign-key enforcement, uses a five-second busy timeout, and requests write-ahead logging with normal synchronization. SQLite is bundled through `rusqlite` so supported installations use the same database engine behavior.

## Migration policy

Migrations are ordered SQL files under `apps/desktop/src-tauri/src/database/migrations/`. At startup, each unapplied migration runs inside an immediate transaction and is recorded in `schema_migrations` only after its SQL succeeds. Reopening an existing database is idempotent, and the application refuses to open a database whose schema is newer than it supports.

Once a migration has shipped, do not edit it. Add the next numbered migration and append it to `MIGRATIONS` in `database/mod.rs`. A migration must preserve user data or document an explicit recovery strategy.

Meeting and processing transitions follow the transactional contract in [`workflow-state.md`](workflow-state.md).

## Initial schema

- `meetings` owns the lifecycle-level record and source metadata.
- `meeting_tags` stores the normalized, user-managed tags used by the local library.
- `transcript_segments` stores ordered, time-aligned source evidence for a meeting.
- `transcript_derivatives` stores revisioned corrections, translations, and romanizations without overwriting source evidence.
- `jobs` stores recoverable processing work, progress, attempts, checkpoints, and errors.
- `actions` stores meeting action items and optional links to their source segment.
- `settings` stores typed configuration as validated JSON values.

Meeting-owned segments, jobs, and actions cascade on meeting deletion. Deleting an individual transcript segment retains its action item but clears the evidence link. Durations and transcript offsets are integer milliseconds; timestamps are UTC ISO-8601 strings.

Completed recordings store their private application-data directory and measured duration on the meeting record. See [`recording.md`](recording.md) for capture, recovery, playback, and media-deletion rules.

The immutability and revision rules are documented in [`evidence-model.md`](evidence-model.md).
Meeting creation, archival, reopening, tagging, and deletion rules are documented in [`meeting-management.md`](meeting-management.md).
