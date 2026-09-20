# Local processing and network boundary

Transcrip-it stores meeting metadata, audio chunks, transcripts, corrections, and exports under the current user's local application-data and document directories. Recording uses local PulseAudio Unix sockets, transcription invokes the installed local FFmpeg and `whisper.cpp` processes, and search uses the bundled SQLite database. No recording, transcript, title, tag, correction, or search text is sent to a network service.

The normal create, record, live-preview, transcribe, search, playback, edit, export, archive, and delete workflows do not contain an HTTP client or remote endpoint. An inspection of the running Linux desktop process on Sep 20, 2026 found no Internet sockets.

Network access is limited to an explicit model-install action in Settings. That action downloads the pinned `whisper.cpp` source and the selected public model file with `git` and `curl`; it does not upload meeting data. The downloaded model is checksum-verified before installation. Package installation and developer build tooling can also access their public package registries outside the application runtime.

Any future synchronization, cloud inference, telemetry, or collaboration service requires a separate opt-in design and must not change this default boundary silently.
