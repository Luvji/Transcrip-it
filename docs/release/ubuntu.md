# Ubuntu release candidate

Transcrip-it 0.1.0 is an Ubuntu 22.04 amd64 release candidate. Recording,
transcription, transcript review, search, and export run locally. The package
declares the system tools required by those workflows so `apt` can install them
with the application.

## Build

From the repository root:

```bash
npm ci
npm run check
npm run tauri --workspace @transcrip-it/desktop -- build --bundles deb
```

The package is written to
`apps/desktop/src-tauri/target/release/bundle/deb/`.

## Install on a clean Ubuntu system

Use `apt`, not `dpkg`, so package dependencies are resolved:

```bash
sudo apt install ./Transcrip-it_0.1.0_amd64.deb
```

Ubuntu 22.04's base repository does not provide the required Node.js 18 or
newer runtime. Configure a trusted Node.js repository before installation when
the target does not already provide it; the tested workstation uses the
NodeSource `node_20.x` repository. Ubuntu 24.04 already provides a compatible
runtime.

Launch **Transcrip-it** from the application menu. On first use, open Settings
and install the Balanced English model. That one-time operation downloads and
compiles the pinned whisper.cpp engine, then verifies the model checksum. A
working internet connection is required only for this setup step.

For the clearest separation of local and remote speech, use a headset. Speaker
capture is supported, but loud simultaneous playback can suppress local speech
in the current echo-cancellation implementation.

## Diagnostics

Settings shows audio-tool readiness, available storage, and transcription model
status without exposing meeting content. If recording is unavailable, verify
the current audio server and required executables:

```bash
pactl info
command -v node parec pactl ffmpeg ffplay
```

Application data is stored at
`$XDG_DATA_HOME/com.transcripit.desktop/` when `XDG_DATA_HOME` is set, or at
`$HOME/.local/share/com.transcripit.desktop/` otherwise. The directory contains
the SQLite database, private recording chunks, exports, and downloaded model.

## Backup and restore

Close Transcrip-it before copying data so the database and audio files are from
the same checkpoint. Back up the complete `com.transcripit.desktop` directory,
including hidden files and subdirectories. To restore, close the app, move the
current directory aside, copy the backup into the same location, preserve its
ownership, and relaunch. The app applies any newer database migrations on start.

Keep backups private: they contain original audio and transcript evidence. Do
not merge individual SQLite or recording files from different backups.

## Uninstall

Remove the application package with:

```bash
sudo apt remove transcrip-it
```

Package removal intentionally preserves local meeting data. After making any
needed backup, the user may delete the complete application-data directory to
remove meetings, recordings, transcripts, exports, and the downloaded model.
Never delete that directory while Transcrip-it is running.

## Release validation

Before publishing a package:

1. Inspect its metadata with `dpkg-deb --info` and confirm all runtime
   dependencies are present.
2. Run `apt-get --simulate install ./Transcrip-it_0.1.0_amd64.deb` on the target
   Ubuntu version.
3. Install on a clean user account, launch, install the model, and complete one
   consented record-to-export workflow.
4. Restart during an active recording and during transcription, then confirm
   the meeting is recoverable and completed chunks remain playable.
5. Uninstall the package and confirm user data remains until explicitly removed.
