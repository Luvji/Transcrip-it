# Transcrip It

Transcrip It is a planned Linux-first, local-first desktop meeting assistant. Its initial release is intended to record microphone and system audio reliably, transcribe meetings locally, and generate editable summaries whose decisions and action items link to timestamped transcript evidence.

## Current status

The project has completed its initial audio-capture feasibility spike and now includes the first production desktop shell. The development backlog is maintained in [`dev.tkt`](dev.tkt), based on `Owned_AI_Meeting_Assistant_Project_Plan (1).docx`.

The responsive shell under [`apps/desktop`](apps/desktop) uses Tauri 2, React, TypeScript, Vite, and Tailwind CSS. It establishes the local-first workspace navigation, empty meeting library, privacy state, and headset guidance that later application tickets will connect to persistence and recording behavior.

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

Enable the versioned local hook with `git config core.hooksPath .githooks`. See [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`docs/architecture/repository-layout.md`](docs/architecture/repository-layout.md) for standards and ownership boundaries.
