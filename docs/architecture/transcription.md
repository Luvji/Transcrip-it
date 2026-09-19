# Local transcription

The Linux MVP uses a pinned `whisper.cpp` v1.9.4 runtime with the English `base.en` model. Settings reports installation status and can install the engine and model on demand. The downloaded model is accepted only when its SHA-1 matches the checksum published by the upstream whisper.cpp model catalog.

The first supported pack is **Balanced English** (approximately 142 MiB). It runs entirely on-device; audio and generated text are not sent to a service. The engine is replaceable through the interfaces described in [`engine-interfaces.md`](engine-interfaces.md).

## Processing flow

1. Recoverable WAV chunks are concatenated by track.
2. Microphone and system tracks are mixed and normalized to 16 kHz mono with FFmpeg.
3. `whisper-cli` produces JSON segments with millisecond offsets.
4. Source segments are inserted transactionally with an anonymous `Speaker 1` label and English language metadata.
5. The meeting returns to `ready`; failures enter `failed` and remain recoverable.

Original source segments are immutable and idempotent. The transcript viewer links timestamps back to matching microphone audio. SQLite FTS5 indexes source text for local search. Markdown and plain-text exports are written beneath the meeting's private recording directory.

User corrections append a new revision through the evidence model rather than changing source text. The viewer can reveal the preserved original, search follows the latest correction, and exports use the current corrected display text.

The command and JSON format follow the official [whisper.cpp CLI documentation](https://github.com/ggml-org/whisper.cpp/tree/master/examples/cli), and model metadata follows the official [model catalog](https://github.com/ggml-org/whisper.cpp/blob/master/models/README.md).
