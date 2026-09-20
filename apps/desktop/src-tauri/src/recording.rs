use crate::{
    child_process::terminate_with_parent,
    database::{Database, MeetingState, MeetingTransitionRequest},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use tauri::Manager;

const CAPTURE_SCRIPT: &str = include_str!("../../../../spikes/audio-capture/audio.mjs");
const RECORDING_PREFLIGHT_BYTES: u64 = 512 * 1024 * 1024;

pub struct Recorder {
    active: Mutex<Option<ActiveRecording>>,
    playback: Mutex<Option<ActivePlayback>>,
}

struct ActiveRecording {
    meeting_id: String,
    output_path: PathBuf,
    child: Child,
    started: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
    pause_count: u32,
}

struct ActivePlayback {
    meeting_id: String,
    track: String,
    start_ms: i64,
    chunk_count: usize,
    started: Instant,
    child: Child,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    active: bool,
    meeting_id: Option<String>,
    elapsed_seconds: u64,
    process_running: bool,
    paused: bool,
    levels: Vec<AudioLevel>,
    storage_available_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AudioLevel {
    name: String,
    peak: f64,
}

#[derive(Deserialize)]
struct LevelSnapshot {
    tracks: Vec<AudioLevel>,
    #[serde(rename = "storageAvailableBytes")]
    storage_available_bytes: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRecordingOptions {
    mode: String,
    mic_source: Option<String>,
    system_source: Option<String>,
    echo_cancellation: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevices {
    default_source: String,
    default_sink: String,
    system_source: Option<String>,
    sources: Vec<AudioSource>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSource {
    name: String,
    monitor: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedRecording {
    meeting_id: String,
    duration_ms: i64,
    recording_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatus {
    active: bool,
    meeting_id: Option<String>,
    track: Option<String>,
    position_ms: i64,
    chunk_count: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureManifest {
    status: String,
    tracks: Vec<CaptureTrack>,
    errors: Vec<CaptureError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureTrack {
    captured_duration_seconds: Option<f64>,
}

#[derive(Deserialize)]
struct CaptureError {
    message: String,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
            playback: Mutex::new(None),
        }
    }

    pub fn stop_playback_for(&self, meeting_id: Option<&str>) -> Result<bool, String> {
        let mut playback = self
            .playback
            .lock()
            .map_err(|_| "playback state lock is poisoned".to_owned())?;
        let should_stop = playback
            .as_ref()
            .is_some_and(|active| meeting_id.is_none_or(|expected| active.meeting_id == expected));
        if should_stop {
            let mut active = playback.take().expect("matching playback exists");
            stop_child(&mut active.child);
        }
        Ok(should_stop)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Ok(active) = self.active.get_mut() {
            if let Some(recording) = active.as_mut() {
                let _ = signal_tree(&recording.child, "CONT");
                stop_child(&mut recording.child);
            }
        }
        if let Ok(playback) = self.playback.get_mut() {
            if let Some(playback) = playback.as_mut() {
                stop_child(&mut playback.child);
            }
        }
    }
}

#[tauri::command]
pub fn start_recording(
    app: tauri::AppHandle,
    database: tauri::State<'_, Database>,
    recorder: tauri::State<'_, Recorder>,
    meeting_id: String,
    consent_confirmed: bool,
    options: StartRecordingOptions,
) -> Result<RecordingStatus, String> {
    if !consent_confirmed {
        return Err("Recording requires explicit consent confirmation.".to_owned());
    }
    validate_meeting_id(&meeting_id)?;
    if !matches!(options.mode.as_str(), "mic" | "system" | "both") {
        return Err("Capture mode must be mic, system, or both.".to_owned());
    }
    if options.echo_cancellation && options.mode != "both" {
        return Err("Echo cancellation requires microphone + system capture.".to_owned());
    }
    recorder.stop_playback_for(None)?;
    let mut active = recorder
        .active
        .lock()
        .map_err(|_| "recording state lock is poisoned".to_owned())?;
    if let Some(recording) = active.as_ref() {
        return Err(format!(
            "A recording is already active for meeting {}.",
            recording.meeting_id
        ));
    }

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let script_path = install_capture_script(&app_data)?;
    let recording_path = app_data.join("recordings").join(&meeting_id);
    if recording_path.exists() {
        return Err("A recording directory already exists for this meeting.".to_owned());
    }
    fs::create_dir_all(
        recording_path
            .parent()
            .ok_or_else(|| "recording path has no parent".to_owned())?,
    )
    .map_err(|error| format!("Could not create recording storage: {error}"))?;
    let available = available_storage_bytes(
        recording_path
            .parent()
            .ok_or_else(|| "recording path has no parent".to_owned())?,
    )?;
    if available < RECORDING_PREFLIGHT_BYTES {
        return Err(format!(
            "Recording needs at least 512 MB free; only {} MB is available.",
            available / 1024 / 1024
        ));
    }

    ensure_capture_dependencies()?;
    let mut command = Command::new("node");
    command
        .arg(script_path)
        .args(["record", "--mode", &options.mode, "--consent-confirmed"]);
    if options.echo_cancellation {
        command.arg("--echo-cancel");
    }
    if let Some(source) = options
        .mic_source
        .as_deref()
        .filter(|source| !source.is_empty())
    {
        command.args(["--mic-source", source]);
    }
    if let Some(source) = options
        .system_source
        .as_deref()
        .filter(|source| !source.is_empty())
    {
        command.args(["--system-source", source]);
    }
    terminate_with_parent(&mut command);
    let mut child = command
        .arg("--output")
        .arg(&recording_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start the audio recorder: {error}"))?;

    thread::sleep(Duration::from_millis(350));
    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
        return Err(format!(
            "Audio capture stopped during startup with status {status}. Check that microphone and system-audio devices are available."
        ));
    }

    let transition = MeetingTransitionRequest {
        meeting_id: meeting_id.clone(),
        idempotency_key: format!("recording-start:{meeting_id}"),
        expected_state: MeetingState::Draft,
        next_state: MeetingState::Recording,
    };
    if let Err(error) = database.transition_meeting(&transition) {
        stop_child(&mut child);
        return Err(error.to_string());
    }
    database
        .mark_recording_started(&meeting_id, &recording_path.to_string_lossy())
        .map_err(|error| error.to_string())?;

    *active = Some(ActiveRecording {
        meeting_id: meeting_id.clone(),
        output_path: recording_path,
        child,
        started: Instant::now(),
        paused_at: None,
        paused_total: Duration::ZERO,
        pause_count: 0,
    });
    Ok(RecordingStatus {
        active: true,
        meeting_id: Some(meeting_id),
        elapsed_seconds: 0,
        process_running: true,
        paused: false,
        levels: Vec::new(),
        storage_available_bytes: Some(available),
    })
}

#[tauri::command]
pub fn audio_devices() -> Result<AudioDevices, String> {
    let default_source = pactl_output(&["get-default-source"])?;
    let default_sink = pactl_output(&["get-default-sink"])?;
    let sources = parse_sources(&pactl_output(&["list", "short", "sources"])?);
    let expected_monitor = format!("{default_sink}.monitor");
    let system_source = sources
        .iter()
        .find(|source| source.name == expected_monitor)
        .or_else(|| sources.iter().find(|source| source.monitor))
        .map(|source| source.name.clone());
    Ok(AudioDevices {
        default_source,
        default_sink,
        system_source,
        sources,
    })
}

#[tauri::command]
pub fn recording_status(recorder: tauri::State<'_, Recorder>) -> Result<RecordingStatus, String> {
    let mut active = recorder
        .active
        .lock()
        .map_err(|_| "recording state lock is poisoned".to_owned())?;
    let Some(recording) = active.as_mut() else {
        return Ok(RecordingStatus {
            active: false,
            meeting_id: None,
            elapsed_seconds: 0,
            process_running: false,
            paused: false,
            levels: Vec::new(),
            storage_available_bytes: None,
        });
    };
    status_for(recording)
}

#[tauri::command]
pub fn recover_interrupted_recordings(
    database: tauri::State<'_, Database>,
    recorder: tauri::State<'_, Recorder>,
) -> Result<usize, String> {
    let mut active = recorder
        .active
        .lock()
        .map_err(|_| "recording state lock is poisoned".to_owned())?;
    if let Some(recording) = active.as_mut() {
        if recording
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Ok(0);
        }
        active.take();
    }
    drop(active);
    recover_completed_recording_metadata(&database)?;
    database
        .recover_interrupted_meetings()
        .map_err(|error| error.to_string())
}

pub(crate) fn recover_completed_recording_metadata(database: &Database) -> Result<usize, String> {
    let candidates = database
        .recordings_missing_duration()
        .map_err(|error| error.to_string())?;
    let mut recovered = 0;
    for (meeting_id, stored_path) in candidates {
        let recording_path = PathBuf::from(&stored_path);
        let Ok(manifest) = read_manifest(&recording_path) else {
            continue;
        };
        if manifest.status != "complete" {
            continue;
        }
        let Some(duration_seconds) = manifest
            .tracks
            .iter()
            .filter_map(|track| track.captured_duration_seconds)
            .reduce(f64::max)
        else {
            continue;
        };
        let duration_ms = duration_seconds.mul_add(1000.0, 0.0).round() as i64;
        database
            .mark_recording_finished(&meeting_id, duration_ms, &stored_path)
            .map_err(|error| error.to_string())?;
        recovered += 1;
    }
    Ok(recovered)
}

#[tauri::command]
pub fn pause_recording(
    database: tauri::State<'_, Database>,
    recorder: tauri::State<'_, Recorder>,
    meeting_id: String,
) -> Result<RecordingStatus, String> {
    let mut active = recorder
        .active
        .lock()
        .map_err(|_| "recording state lock is poisoned".to_owned())?;
    let recording = active
        .as_mut()
        .ok_or_else(|| "No recording is active.".to_owned())?;
    ensure_active_meeting(recording, &meeting_id)?;
    if recording.paused_at.is_some() {
        return status_for(recording);
    }
    recording.pause_count += 1;
    database
        .transition_meeting(&MeetingTransitionRequest {
            meeting_id: meeting_id.clone(),
            idempotency_key: format!("recording-pause:{meeting_id}:{}", recording.pause_count),
            expected_state: MeetingState::Recording,
            next_state: MeetingState::Paused,
        })
        .map_err(|error| error.to_string())?;
    signal_tree(&recording.child, "STOP")?;
    recording.paused_at = Some(Instant::now());
    status_for(recording)
}

#[tauri::command]
pub fn resume_recording(
    database: tauri::State<'_, Database>,
    recorder: tauri::State<'_, Recorder>,
    meeting_id: String,
) -> Result<RecordingStatus, String> {
    let mut active = recorder
        .active
        .lock()
        .map_err(|_| "recording state lock is poisoned".to_owned())?;
    let recording = active
        .as_mut()
        .ok_or_else(|| "No recording is active.".to_owned())?;
    ensure_active_meeting(recording, &meeting_id)?;
    resume_active(&database, recording)?;
    status_for(recording)
}

#[tauri::command]
pub fn stop_recording(
    database: tauri::State<'_, Database>,
    recorder: tauri::State<'_, Recorder>,
    meeting_id: String,
) -> Result<CompletedRecording, String> {
    let mut guard = recorder
        .active
        .lock()
        .map_err(|_| "recording state lock is poisoned".to_owned())?;
    let mut recording = guard
        .take()
        .ok_or_else(|| "No recording is active.".to_owned())?;
    if recording.meeting_id != meeting_id {
        let active_id = recording.meeting_id.clone();
        *guard = Some(recording);
        return Err(format!(
            "The active recording belongs to meeting {active_id}."
        ));
    }

    resume_active(&database, &mut recording)?;
    stop_child(&mut recording.child);
    let manifest = read_manifest(&recording.output_path)?;
    if manifest.status != "complete" {
        fail_recording(&database, &meeting_id)?;
        let details = manifest
            .errors
            .iter()
            .map(|error| error.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(if details.is_empty() {
            "Audio capture did not complete successfully.".to_owned()
        } else {
            format!("Audio capture failed: {details}")
        });
    }

    let duration_ms = manifest
        .tracks
        .iter()
        .filter_map(|track| track.captured_duration_seconds)
        .fold(0.0_f64, f64::max)
        .mul_add(1000.0, 0.0)
        .round() as i64;
    database
        .mark_recording_finished(
            &meeting_id,
            duration_ms,
            &recording.output_path.to_string_lossy(),
        )
        .map_err(|error| error.to_string())?;
    transition_after_capture(&database, &meeting_id, MeetingState::Ready)?;

    Ok(CompletedRecording {
        meeting_id,
        duration_ms,
        recording_path: recording.output_path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn play_recording_track(
    database: tauri::State<'_, Database>,
    recorder: tauri::State<'_, Recorder>,
    meeting_id: String,
    track: String,
    start_ms: Option<i64>,
) -> Result<PlaybackStatus, String> {
    if !matches!(track.as_str(), "mic" | "mic_raw" | "system") {
        return Err("Unsupported recording track.".to_owned());
    }
    let start_ms = start_ms.unwrap_or(0);
    if start_ms < 0 {
        return Err("Playback start time cannot be negative.".to_owned());
    }
    let recording_path = database
        .meeting_recording_path(&meeting_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "This meeting has no completed recording.".to_owned())?;
    let recording_path = PathBuf::from(recording_path);
    let prefix = format!("{track}-");
    let mut chunks = fs::read_dir(&recording_path)
        .map_err(|error| format!("Could not read recording directory: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".wav"))
        })
        .collect::<Vec<_>>();
    chunks.sort();
    if chunks.is_empty() {
        return Err(format!("No {track} audio was captured for this meeting."));
    }

    let mut playback = recorder
        .playback
        .lock()
        .map_err(|_| "playback state lock is poisoned".to_owned())?;
    if let Some(active) = playback.as_mut() {
        let running = active
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none();
        if running
            && active.meeting_id == meeting_id
            && active.track == track
            && active.start_ms == start_ms
        {
            return Ok(active_playback_status(active));
        }
        stop_child(&mut active.child);
        *playback = None;
    }

    let playlist_path = recording_path.join(format!("play-{track}.ffconcat"));
    let playlist = chunks
        .iter()
        .map(|path| format!("file '{}'\n", path.to_string_lossy().replace('\'', "'\\''")))
        .collect::<String>();
    fs::write(&playlist_path, format!("ffconcat version 1.0\n{playlist}"))
        .map_err(|error| format!("Could not prepare playback: {error}"))?;
    let mut command = Command::new("ffplay");
    command.args([
        "-nodisp",
        "-autoexit",
        "-loglevel",
        "error",
        "-f",
        "concat",
        "-safe",
        "0",
    ]);
    if start_ms > 0 {
        command.args(["-ss", &format!("{:.3}", start_ms as f64 / 1000.0)]);
    }
    terminate_with_parent(&mut command);
    let child = command
        .arg("-i")
        .arg(playlist_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start audio playback: {error}"))?;
    let active = ActivePlayback {
        meeting_id,
        track,
        start_ms,
        chunk_count: chunks.len(),
        started: Instant::now(),
        child,
    };
    let status = active_playback_status(&active);
    *playback = Some(active);
    Ok(status)
}

#[tauri::command]
pub fn playback_status(recorder: tauri::State<'_, Recorder>) -> Result<PlaybackStatus, String> {
    let mut playback = recorder
        .playback
        .lock()
        .map_err(|_| "playback state lock is poisoned".to_owned())?;
    let finished = match playback.as_mut() {
        Some(active) => active
            .child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some(),
        None => return Ok(inactive_playback_status()),
    };
    if finished {
        *playback = None;
        return Ok(inactive_playback_status());
    }
    Ok(active_playback_status(
        playback.as_ref().expect("active playback exists"),
    ))
}

#[tauri::command]
pub fn stop_recording_playback(
    recorder: tauri::State<'_, Recorder>,
) -> Result<PlaybackStatus, String> {
    recorder.stop_playback_for(None)?;
    Ok(inactive_playback_status())
}

fn active_playback_status(playback: &ActivePlayback) -> PlaybackStatus {
    PlaybackStatus {
        active: true,
        meeting_id: Some(playback.meeting_id.clone()),
        track: Some(playback.track.clone()),
        position_ms: playback
            .start_ms
            .saturating_add(playback.started.elapsed().as_millis() as i64),
        chunk_count: playback.chunk_count,
    }
}

fn inactive_playback_status() -> PlaybackStatus {
    PlaybackStatus {
        active: false,
        meeting_id: None,
        track: None,
        position_ms: 0,
        chunk_count: 0,
    }
}

fn install_capture_script(app_data: &Path) -> Result<PathBuf, String> {
    let runtime_directory = app_data.join("runtime");
    fs::create_dir_all(&runtime_directory)
        .map_err(|error| format!("Could not create recorder runtime directory: {error}"))?;
    let script_path = runtime_directory.join("audio-capture.mjs");
    if fs::read_to_string(&script_path).ok().as_deref() != Some(CAPTURE_SCRIPT) {
        fs::write(&script_path, CAPTURE_SCRIPT)
            .map_err(|error| format!("Could not install the audio recorder: {error}"))?;
    }
    Ok(script_path)
}

fn ensure_capture_dependencies() -> Result<(), String> {
    for executable in ["node", "pactl", "parec"] {
        let available = Command::new(executable)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !available {
            return Err(format!("Required audio tool is unavailable: {executable}"));
        }
    }
    Ok(())
}

fn pactl_output(arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("pactl")
        .args(arguments)
        .output()
        .map_err(|error| format!("Could not run pactl: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "PulseAudio device query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn available_storage_bytes(path: &Path) -> Result<u64, String> {
    let output = Command::new("df")
        .args(["-Pk"])
        .arg(path)
        .output()
        .map_err(|error| format!("Could not check available storage: {error}"))?;
    if !output.status.success() {
        return Err("Could not check available storage before recording.".to_owned());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let available_kib = text
        .lines()
        .last()
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "Could not understand available storage information.".to_owned())?;
    Ok(available_kib.saturating_mul(1024))
}

fn parse_sources(output: &str) -> Vec<AudioSource> {
    output
        .lines()
        .filter_map(|line| line.split('\t').nth(1))
        .map(|name| AudioSource {
            name: name.to_owned(),
            monitor: name.ends_with(".monitor"),
        })
        .collect()
}

fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let signalled = Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !signalled {
            let _ = child.kill();
        }
    }
    let _ = child.wait();
}

fn signal_tree(child: &Child, signal: &str) -> Result<(), String> {
    let pid = child.id().to_string();
    let children_first = signal == "STOP";
    let signal_parent = || {
        Command::new("kill")
            .args([format!("-{signal}"), pid.clone()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    };
    let signal_children = || {
        Command::new("pkill")
            .args([format!("-{signal}"), "-P".to_owned(), pid.clone()])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    };
    let parent_ok = if children_first {
        let _ = signal_children();
        signal_parent()
    } else {
        let parent_ok = signal_parent();
        let _ = signal_children();
        parent_ok
    };
    if parent_ok {
        Ok(())
    } else {
        Err(format!("Could not {signal} the recording process."))
    }
}

fn resume_active(database: &Database, recording: &mut ActiveRecording) -> Result<(), String> {
    let Some(paused_at) = recording.paused_at else {
        return Ok(());
    };
    database
        .transition_meeting(&MeetingTransitionRequest {
            meeting_id: recording.meeting_id.clone(),
            idempotency_key: format!(
                "recording-resume:{}:{}",
                recording.meeting_id, recording.pause_count
            ),
            expected_state: MeetingState::Paused,
            next_state: MeetingState::Recording,
        })
        .map_err(|error| error.to_string())?;
    signal_tree(&recording.child, "CONT")?;
    recording.paused_total += paused_at.elapsed();
    recording.paused_at = None;
    Ok(())
}

fn ensure_active_meeting(recording: &ActiveRecording, meeting_id: &str) -> Result<(), String> {
    if recording.meeting_id == meeting_id {
        Ok(())
    } else {
        Err(format!(
            "The active recording belongs to meeting {}.",
            recording.meeting_id
        ))
    }
}

fn recording_elapsed(recording: &ActiveRecording) -> Duration {
    let current_pause = recording
        .paused_at
        .map_or(Duration::ZERO, |paused| paused.elapsed());
    recording
        .started
        .elapsed()
        .saturating_sub(recording.paused_total + current_pause)
}

fn status_for(recording: &mut ActiveRecording) -> Result<RecordingStatus, String> {
    let process_running = recording
        .child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_none();
    let snapshot = read_levels(&recording.output_path, recording.paused_at.is_some());
    Ok(RecordingStatus {
        active: true,
        meeting_id: Some(recording.meeting_id.clone()),
        elapsed_seconds: recording_elapsed(recording).as_secs(),
        process_running,
        paused: recording.paused_at.is_some(),
        levels: snapshot
            .as_ref()
            .map_or_else(Vec::new, |value| value.tracks.clone()),
        storage_available_bytes: snapshot.and_then(|value| value.storage_available_bytes),
    })
}

fn read_manifest(recording_path: &Path) -> Result<CaptureManifest, String> {
    let path = recording_path.join("manifest.json");
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "Could not read capture manifest {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_str(&content).map_err(|error| format!("Invalid capture manifest: {error}"))
}

fn read_levels(recording_path: &Path, paused: bool) -> Option<LevelSnapshot> {
    if paused {
        return None;
    }
    fs::read_to_string(recording_path.join("levels.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<LevelSnapshot>(&content).ok())
}

fn transition_after_capture(
    database: &Database,
    meeting_id: &str,
    final_state: MeetingState,
) -> Result<(), String> {
    database
        .transition_meeting(&MeetingTransitionRequest {
            meeting_id: meeting_id.to_owned(),
            idempotency_key: format!("recording-process:{meeting_id}"),
            expected_state: MeetingState::Recording,
            next_state: MeetingState::Processing,
        })
        .map_err(|error| error.to_string())?;
    database
        .transition_meeting(&MeetingTransitionRequest {
            meeting_id: meeting_id.to_owned(),
            idempotency_key: format!("recording-finish:{meeting_id}"),
            expected_state: MeetingState::Processing,
            next_state: final_state,
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn fail_recording(database: &Database, meeting_id: &str) -> Result<(), String> {
    transition_after_capture(database, meeting_id, MeetingState::Failed)
}

fn validate_meeting_id(meeting_id: &str) -> Result<(), String> {
    if !meeting_id.is_empty()
        && meeting_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        Ok(())
    } else {
        Err("Meeting ID contains unsupported characters.".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CreateMeetingInput;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn meeting_ids_are_safe_for_recording_paths() {
        assert!(validate_meeting_id("0199-demo_meeting").is_ok());
        assert!(validate_meeting_id("../escape").is_err());
        assert!(validate_meeting_id("").is_err());
    }

    #[test]
    fn managed_playback_stops_only_the_requested_meeting() {
        let recorder = Recorder::new();
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        *recorder.playback.lock().unwrap() = Some(ActivePlayback {
            meeting_id: "meeting-1".to_owned(),
            track: "mic".to_owned(),
            start_ms: 0,
            chunk_count: 1,
            started: Instant::now(),
            child,
        });

        assert!(!recorder.stop_playback_for(Some("another-meeting")).unwrap());
        assert!(recorder
            .playback
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .child
            .try_wait()
            .unwrap()
            .is_none());
        assert!(recorder.stop_playback_for(Some("meeting-1")).unwrap());
        assert!(recorder.playback.lock().unwrap().is_none());
    }

    #[test]
    fn embedded_capture_script_is_present() {
        assert!(CAPTURE_SCRIPT.contains("--consent-confirmed"));
        assert!(CAPTURE_SCRIPT.contains("module-echo-cancel"));
    }

    #[test]
    fn completed_manifest_restores_duration_after_interruption() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "transcrip-it-recovery-{}-{unique}",
            std::process::id()
        ));
        let recording_path = root.join("recording");
        fs::create_dir_all(&recording_path).unwrap();
        let database = Database::open(&root.join("test.sqlite3")).unwrap();
        database
            .create_meeting(&CreateMeetingInput {
                id: "meeting-1".to_owned(),
                idempotency_key: "create-meeting-1".to_owned(),
                title: "Interrupted recording".to_owned(),
            })
            .unwrap();
        database
            .mark_recording_started("meeting-1", &recording_path.to_string_lossy())
            .unwrap();
        fs::write(
            recording_path.join("manifest.json"),
            r#"{"status":"complete","tracks":[{"capturedDurationSeconds":5178.125}],"errors":[]}"#,
        )
        .unwrap();

        assert_eq!(recover_completed_recording_metadata(&database).unwrap(), 1);
        let meeting = database.list_meetings(false).unwrap().remove(0);
        assert_eq!(meeting.duration_ms, Some(5_178_125));
        assert_eq!(recover_completed_recording_metadata(&database).unwrap(), 0);

        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_microphone_and_monitor_sources() {
        let sources = parse_sources(
            "1\talsa_input.usb-mic\tmodule\ts16le\n2\talsa_output.pci.monitor\tmodule\ts16le",
        );
        assert_eq!(sources.len(), 2);
        assert!(!sources[0].monitor);
        assert!(sources[1].monitor);
    }
}
