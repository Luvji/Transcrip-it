# Replaceable processing engines

The desktop backend depends on capability-oriented Rust traits rather than concrete model runtimes. Transcription, speaker, and summary engines accept stable request and output types while adapters own runtime-specific process, model, and parsing details.

Every engine publishes a stable identifier, human-readable name, implementation version, and capabilities. The engine registry resolves implementations by identifier and permits an adapter to be replaced without changing persisted meetings, transcript evidence, or queued job kinds.

All engine calls receive a shared cancellation token and progress reporter. Errors carry a stable code, safe user-facing message, and retryable flag so the job state machine can make consistent recovery decisions.

Adapters must obey these boundaries:

- Never write directly to SQLite; return typed output to the orchestration layer.
- Never overwrite source transcript evidence.
- Check cancellation during long operations and report monotonic progress from zero to one.
- Avoid logging meeting content, prompts, transcripts, or generated notes.
- Keep engine-specific paths and options in settings or model-pack metadata rather than stored meeting records.
