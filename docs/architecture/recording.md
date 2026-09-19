# Desktop recording workflow

The desktop app connects the consent screen to the proven PulseAudio capture implementation. Audio capture never starts until the user checks the consent acknowledgement and presses **Start recording**.

## Session flow

1. The app discovers PulseAudio sources and lets the user choose microphone-only, system-only, or combined capture.
2. Combined capture can enable the accepted WebRTC echo-cancellation route. The original microphone is preserved alongside the processed microphone and system tracks.
3. The backend embeds and installs the versioned Node capture worker inside the per-user application-data directory, then launches it with explicit source and consent arguments.
4. Start, pause, resume, and stop are mirrored in the transactional meeting state machine. Only one recorder may be active at a time.
5. Stop uses `SIGINT`, waits for the worker to finalize WAV headers and the atomic manifest, records duration and the private recording path, then marks the meeting ready.

The worker writes one-minute PCM WAV chunks so completed chunks remain usable after interruption. The UI restores the active recording display after a frontend reload. Separate microphone and system tracks can be played with `ffplay`; permanent meeting deletion also removes its recording directory after verifying it is inside application storage.

While recording, the worker publishes normalized peak levels for each selected track and the UI polls process health. A disconnected device or stopped worker produces an immediate warning. Capture requires 512 MB free at startup, displays a low-space warning below that threshold, and stops safely if available space drops below 128 MB.

## Runtime requirements

The current Linux MVP uses the system `node`, `pactl`, `parec`, `kill`, `pkill`, and `ffplay` executables. Missing tools produce an actionable error before capture. These dependencies will be replaced or bundled as release packaging matures.
