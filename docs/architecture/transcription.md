# Local transcription

The Linux app uses a pinned `whisper.cpp` v1.9.4 runtime with independently downloadable English models. Settings reports installation status and installs the selected engine/model on demand. Every downloaded model is accepted only when its SHA-1 matches the checksum published by the upstream whisper.cpp model catalog.

The available packs are **Fast English** (`base.en`, approximately 142 MiB), **Balanced English** (`small.en`, approximately 466 MiB), and **Accuracy English** (`medium.en`, approximately 1.5 GiB). Balanced is recommended when clearer wording is more important than processing speed. All packs run entirely on-device; audio and generated text are not sent to a service. The engine is replaceable through the interfaces described in [`engine-interfaces.md`](engine-interfaces.md).

## Processing flow

1. Recoverable WAV chunks are concatenated independently for microphone and system audio.
2. Each track is filtered, loudness-normalized, and converted to 16 kHz mono with FFmpeg.
3. The selected `whisper-cli` model produces JSON segments with millisecond offsets for each channel.
4. Adjacent segments from continuous speech are grouped into readable passages, bounded by pauses, duration, and size.
5. When the original microphone is selected, raw model segments are checked before passage grouping. A microphone segment is treated as speaker bleed only when system speech covers at least half its duration, it contains at least four words, and at least 72% of its normalized microphone words also occur in the overlapping system transcript. Short, mixed, and uncertain microphone segments are preserved.
6. Remaining passages are merged onto one timeline with automatic microphone-source and `Meeting audio` labels and immutable source-track provenance.
7. The meeting returns to `ready`; failures enter `failed` and remain recoverable.

Original source passages are immutable and idempotent. The transcript viewer links timestamps back to the matching microphone or system track. Channel labels may be renamed to known participant names without changing transcript evidence. SQLite FTS5 indexes source text for local search. Markdown and plain-text exports are written beneath the meeting's private recording directory.

User corrections append a new revision through the evidence model rather than changing source text. The viewer can reveal the preserved original, search follows the latest correction, and exports use the current corrected display text.

During recording, the app uses the Fast English model on non-overlapping eight-second windows from the current recoverable audio chunks. Each partial window is replaced as more audio arrives, and window numbering continues for the full one-minute chunk rather than stopping after 45 seconds. The preserved `test2` fixture took 1.60 seconds for its system window and 1.81 seconds for its original-microphone window on the development machine; the UI requests another update after 2.5 seconds. A lower-threshold same-window comparison suppresses likely live microphone bleed because this text is explicitly provisional. Preview passages remain in the recording view, carry a visible delay/accuracy warning, and are not stored as final evidence. After stop, the selected pack reprocesses the full tracks for the final transcript. Live transcription therefore requires Fast English to be installed even when Balanced or Accuracy is selected for final processing.

Deleting a transcript removes its source passages, corrections, and search entries while retaining the recording so it can be transcribed again with a different model. Deleting a meeting remains a separate, explicit operation that also removes its private recording directory.

The command and JSON format follow the official [whisper.cpp CLI documentation](https://github.com/ggml-org/whisper.cpp/tree/master/examples/cli), and model metadata follows the official [model catalog](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md).
