# Transcrip It

Transcrip It is a planned Linux-first, local-first desktop meeting assistant. Its initial release is intended to record microphone and system audio reliably, transcribe meetings locally, and generate editable summaries whose decisions and action items link to timestamped transcript evidence.

## Current status

The project has completed its initial audio-capture feasibility spike and now includes a persistent local meeting library with consent-gated microphone/system recording, safe controls, provisional live transcription, separate-track playback, selectable offline English accuracy packs, grouped channel-attributed transcripts, local transcript search, and open-format export. The development backlog is maintained in [`dev.tkt`](dev.tkt), based on `Owned_AI_Meeting_Assistant_Project_Plan (1).docx`.

The responsive shell under [`apps/desktop`](apps/desktop) uses Tauri 2, React, TypeScript, Vite, and Tailwind CSS. Its Rust backend initializes a local SQLite database with versioned migrations for meetings, transcript segments, jobs, actions, and settings.

The in-progress feasibility CLI lives under `spikes/audio-capture/`. List available sources with `npm run audio:devices`, or review the capture and WebRTC echo-cancellation instructions in [`docs/feasibility/TI-0012.md`](docs/feasibility/TI-0012.md).

## First-release direction

- Ubuntu desktop, single user, and fully local processing by default.
- Post-meeting transcription before live transcription.
- Supported English transcription with timestamps and basic speaker labels.
- Editable transcripts and grounded summaries with evidence links.
- Local search, retention controls, backup, and Markdown or plain-text export.
- Experimental Malayalam, Hindi, Japanese, and mixed-language support delivered separately from the supported English workflow.

The MVP uses WebRTC acoustic echo cancellation when capturing through speakers. A headset is recommended for best separation because loud system playback during simultaneous speech can suppress the local microphone; improving this double-talk behavior is planned after the working MVP.

Live transcription, Windows and macOS support, meeting bots, mobile capture, cloud collaboration, and calendar joining remain deferred until the core recording and processing pipeline meets its quality gates.

## Planning source

The full scope, architecture, roadmap, privacy requirements, risks, and release criteria are recorded in `Owned_AI_Meeting_Assistant_Project_Plan (1).docx`. Before implementation begins, the open scope and environment decisions marked `~-` in `dev.tkt` need product-owner confirmation.

## Development

Use Node.js 22 and run the repository checks before committing:

```bash
npm ci
npm run check
```

On Ubuntu, install the native packages listed in [`apps/desktop/README.md`](apps/desktop/README.md), then launch the app with:

```bash
npm run desktop:dev
```

For browser-only interface work, use `npm run desktop:web`.

SQLite data is stored in the operating system's application-data directory and is never written inside the repository. See [`docs/architecture/persistence.md`](docs/architecture/persistence.md) for the schema and migration contract and [`docs/architecture/workflow-state.md`](docs/architecture/workflow-state.md) for retry-safe lifecycle rules.

Original transcript segments are immutable. User corrections, translations, and model-derived alternatives are revisioned separately according to [`docs/architecture/evidence-model.md`](docs/architecture/evidence-model.md).

Meeting creation and library lifecycle operations are exposed through retry-safe desktop commands and documented in [`docs/architecture/meeting-management.md`](docs/architecture/meeting-management.md).

The native recording lifecycle and its Linux runtime requirements are documented in [`docs/architecture/recording.md`](docs/architecture/recording.md).

Offline model installation, timestamp persistence, search, and export are documented in [`docs/architecture/transcription.md`](docs/architecture/transcription.md).

Build, install, diagnostics, backup/restore, and uninstall instructions for the Ubuntu release candidate are in [`docs/release/ubuntu.md`](docs/release/ubuntu.md).

Enable the versioned local hook with `git config core.hooksPath .githooks`. See [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`docs/architecture/repository-layout.md`](docs/architecture/repository-layout.md) for standards and ownership boundaries.
