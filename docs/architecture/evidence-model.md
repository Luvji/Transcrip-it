# Evidence-preserving transcript model

`transcript_segments` is the immutable record produced from source audio. Its meeting, sequence, timing, source text, and creation timestamp cannot be updated after insertion. Deleting the owning meeting remains the explicit way to remove its evidence under retention policy.

Corrections, translations, and romanizations are appended to `transcript_derivatives` instead of changing a source segment. Each derivative records whether a user or model created it, its language and model when applicable, a stable idempotency key, and the prior revision it supersedes.

The `transcript_current_text` view provides convenient display text by selecting the newest correction while returning the original source text alongside it. Consumers that present or export corrected text must retain the segment identifier so the original evidence remains inspectable.

Generated summaries, decisions, and actions remain separate meeting-level records. Future migrations may add their own revision tables, but they must link back to source segments and must never be written into `transcript_segments`.
