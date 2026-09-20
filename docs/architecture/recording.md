# Desktop recording workflow

The desktop app connects the consent screen to the proven PulseAudio capture implementation. Audio capture never starts until the user checks the consent acknowledgement and presses **Start recording**.

## Session flow

1. The app discovers PulseAudio sources and lets the user choose microphone-only, system-only, or combined capture.
2. Combined capture can enable the accepted WebRTC echo-cancellation route. The original microphone is preserved alongside the processed microphone and system tracks.
3. The backend embeds and installs the versioned Node capture worker inside the per-user application-data directory, then launches it with explicit source and consent arguments.
4. Start, pause, resume, and stop are mirrored in the transactional meeting state machine. Only one recorder may be active at a time.
5. Stop uses `SIGINT`, waits for the worker to finalize WAV headers and the atomic manifest, records duration and the private recording path, then marks the meeting ready.

Only one desktop process can hold the per-user application lock. Before device discovery or capture, the backend restores physical PulseAudio defaults, moves playback away from orphaned Transcrip-it virtual sinks, and unloads stale echo-cancellation modules left by an interrupted run. App-owned virtual devices are excluded from the source picker.

The worker writes one-minute PCM WAV chunks so completed chunks remain usable after interruption. The UI restores the active recording display after a frontend reload. Separate microphone and system tracks can be played with `ffplay`; permanent meeting deletion also removes its recording directory after verifying it is inside application storage.

Closing the desktop process safely interrupts its child recorder. On Linux, capture, playback, FFmpeg, and Whisper workers also install a parent-death signal immediately before execution, including a parent-PID check that closes the fork/exec race. This prevents workers from becoming orphans after an abrupt desktop crash. On the next launch, a completed manifest restores missing duration metadata; if manifest finalization was interrupted, the backend instead sums validated numbered PCM WAV chunks per track and uses the longest preserved track. Any lifecycle left in recording, paused, or processing then advances through the audited recovery path to `failed`. The library exposes preserved chunks for playback and allows transcription to be retried instead of silently discarding the meeting.

While recording, the worker publishes normalized peak levels for each selected track and the UI polls process health. Muted, disconnected, stopped-worker, and low-storage conditions appear as fixed warning toasts that remain visible while navigating. A muted source does not stop capture. When combined capture starts with only one source available, the recorder continues with that source and reports the fallback; it fails only when none of the requested audio can be captured. Capture requires 512 MB free at startup and stops safely if available space drops below 128 MB.

## Runtime requirements

The current Linux MVP uses the system `node`, `pactl`, `parec`, `kill`, `pkill`, and `ffplay` executables. Missing tools produce an actionable error before capture. The Debian package declares the corresponding Node.js, PulseAudio, procps, and FFmpeg packages as installation dependencies.
