use crate::{
    child_process::terminate_with_parent,
    database::{Database, MeetingState, MeetingTransitionRequest},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
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
    warnings: Vec<String>,
    microphone_source: Option<String>,
    system_source: Option<String>,
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
    warnings: Vec<String>,
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
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSource {
    name: String,
    monitor: bool,
}

struct CapturePlan {
    mode: String,
    mic_source: Option<String>,
    system_source: Option<String>,
    echo_cancellation: bool,
    warnings: Vec<String>,
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
    let plan = prepare_capture_plan(&options)?;
    let mut command = Command::new("node");
    command
        .arg(script_path)
        .args(["record", "--mode", &plan.mode, "--consent-confirmed"]);
    if plan.echo_cancellation {
        command.arg("--echo-cancel");
    }
    if let Some(source) = plan
        .mic_source
        .as_deref()
        .filter(|source| !source.is_empty())
    {
        command.args(["--mic-source", source]);
    }
    if let Some(source) = plan
        .system_source
        .as_deref()
        .filter(|source| !source.is_empty())
    {
        command.args(["--system-source", source]);
    }
    let diagnostics_path = app_data
        .join("runtime")
        .join(format!("capture-{meeting_id}.log"));
    let diagnostics = fs::File::create(&diagnostics_path)
        .map_err(|error| format!("Could not create capture diagnostics: {error}"))?;
    terminate_with_parent(&mut command);
    let mut child = command
        .arg("--output")
        .arg(&recording_path)
        .stdout(Stdio::null())
        .stderr(Stdio::from(diagnostics))
        .spawn()
        .map_err(|error| format!("Could not start the audio recorder: {error}"))?;

    if let Err(error) = wait_for_capture_start(&recording_path, &diagnostics_path, &mut child) {
        stop_child(&mut child);
        if recording_path.exists() {
            let _ = fs::remove_dir_all(&recording_path);
        }
        return Err(error);
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
        warnings: plan.warnings.clone(),
        microphone_source: plan.mic_source,
        system_source: plan.system_source,
    });
    Ok(RecordingStatus {
        active: true,
        meeting_id: Some(meeting_id),
        elapsed_seconds: 0,
        process_running: true,
        paused: false,
        levels: Vec::new(),
        storage_available_bytes: Some(available),
        warnings: plan.warnings,
    })
}

fn wait_for_capture_start(
    recording_path: &Path,
    diagnostics_path: &Path,
    child: &mut Child,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(7);
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            let details = fs::read_to_string(diagnostics_path).unwrap_or_default();
            let detail = details
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("Check that the selected audio devices are available.");
            return Err(format!(
                "Audio capture could not start ({status}): {detail}"
            ));
        }
        if let Ok(manifest) = read_manifest(recording_path) {
            match manifest.status.as_str() {
                "recording" => return Ok(()),
                "failed" => return Err("Audio sources could not start. Check the selected microphone and speaker devices.".to_owned()),
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            return Err("Audio sources did not become ready within seven seconds. Check that the microphone or speaker is enabled.".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[tauri::command]
pub fn audio_devices() -> Result<AudioDevices, String> {
    cleanup_stale_echo_cancellation()?;
    let default_source = pactl_output(&["get-default-source"])?;
    let default_sink = pactl_output(&["get-default-sink"])?;
    let sources = parse_sources(&pactl_output(&["list", "short", "sources"])?);
    let expected_monitor = format!("{default_sink}.monitor");
    let system_source = sources
        .iter()
        .find(|source| source.name == expected_monitor)
        .or_else(|| sources.iter().find(|source| source.monitor))
        .map(|source| source.name.clone());
    let warnings = audio_state_warnings(Some(default_source.as_str()), system_source.as_deref());
    Ok(AudioDevices {
        default_source,
        default_sink,
        system_source,
        sources,
        warnings,
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
            warnings: Vec::new(),
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
        let manifest_duration_ms = (manifest.status == "complete")
            .then(|| {
                manifest
                    .tracks
                    .iter()
                    .filter_map(|track| track.captured_duration_seconds)
                    .reduce(f64::max)
                    .map(|seconds| seconds.mul_add(1000.0, 0.0).round() as i64)
            })
            .flatten();
        let Some(duration_ms) =
            manifest_duration_ms.or(recoverable_wav_duration_ms(&recording_path)?)
        else {
            continue;
        };
        database
            .mark_recording_finished(&meeting_id, duration_ms, &stored_path)
            .map_err(|error| error.to_string())?;
        recovered += 1;
    }
    Ok(recovered)
}

fn recoverable_wav_duration_ms(recording_path: &Path) -> Result<Option<i64>, String> {
    let mut track_durations = BTreeMap::<String, i64>::new();
    let entries = fs::read_dir(recording_path).map_err(|error| {
        format!(
            "Could not inspect preserved recording {}: {error}",
            recording_path.display()
        )
    })?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = filename.strip_suffix(".wav") else {
            continue;
        };
        let Some((track, chunk_index)) = stem.rsplit_once('-') else {
            continue;
        };
        if !matches!(track, "mic" | "mic_raw" | "system")
            || chunk_index.len() != 5
            || !chunk_index
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            continue;
        }
        if let Some(duration_ms) = pcm_wav_duration_ms(&path) {
            *track_durations.entry(track.to_owned()).or_default() += duration_ms;
        }
    }
    Ok(track_durations
        .into_values()
        .max()
        .filter(|duration| *duration > 0))
}

fn pcm_wav_duration_ms(path: &Path) -> Option<i64> {
    let mut file = fs::File::open(path).ok()?;
    let mut header = [0_u8; 44];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" || &header[36..40] != b"data" {
        return None;
    }
    let byte_rate = u32::from_le_bytes(header[28..32].try_into().ok()?) as u64;
    let data_bytes = u32::from_le_bytes(header[40..44].try_into().ok()?) as u64;
    (byte_rate > 0)
        .then_some(data_bytes.saturating_mul(1000) / byte_rate)
        .and_then(|duration| i64::try_from(duration).ok())
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

fn prepare_capture_plan(options: &StartRecordingOptions) -> Result<CapturePlan, String> {
    cleanup_stale_echo_cancellation()?;
    let default_source = pactl_output(&["get-default-source"])?;
    let default_sink = pactl_output(&["get-default-sink"])?;
    let sources = parse_sources(&pactl_output(&["list", "short", "sources"])?);
    let requested_mic = options
        .mic_source
        .clone()
        .filter(|source| !source.is_empty())
        .unwrap_or(default_source);
    let requested_system = options
        .system_source
        .clone()
        .filter(|source| {
            !source.is_empty()
                && !is_transcrip_it_virtual(source)
                && sources.iter().any(|candidate| candidate.name == *source)
        })
        .or_else(|| {
            let expected = format!("{default_sink}.monitor");
            sources
                .iter()
                .find(|source| source.name == expected)
                .or_else(|| sources.iter().find(|source| source.monitor))
                .map(|source| source.name.clone())
        });
    let mic_available = sources
        .iter()
        .any(|source| !source.monitor && source.name == requested_mic);
    let system_available = requested_system.as_ref().is_some_and(|requested| {
        sources
            .iter()
            .any(|source| source.monitor && source.name == *requested)
    });
    let mode = effective_capture_mode(&options.mode, mic_available, system_available)?;
    let mut warnings = Vec::new();
    if options.mode == "both" && mode == "system" {
        warnings.push(
            "Microphone unavailable. Recording will continue with system audio only.".to_owned(),
        );
    } else if options.mode == "both" && mode == "mic" {
        warnings.push(
            "System audio unavailable. Recording will continue with microphone audio only."
                .to_owned(),
        );
    }
    let mic_source = (mode != "system").then_some(requested_mic);
    let system_source = if mode != "mic" {
        requested_system
    } else {
        None
    };
    warnings.extend(audio_state_warnings(
        mic_source.as_deref(),
        system_source.as_deref(),
    ));
    warnings.sort();
    warnings.dedup();
    Ok(CapturePlan {
        echo_cancellation: options.echo_cancellation && mode == "both",
        mode: mode.to_owned(),
        mic_source,
        system_source,
        warnings,
    })
}

fn effective_capture_mode(
    requested: &str,
    microphone_available: bool,
    system_available: bool,
) -> Result<&'static str, String> {
    match (requested, microphone_available, system_available) {
        ("both", true, true) => Ok("both"),
        ("both", true, false) => Ok("mic"),
        ("both", false, true) => Ok("system"),
        ("both", false, false) => {
            Err("Neither the selected microphone nor system-audio source is available.".to_owned())
        }
        ("mic", true, _) => Ok("mic"),
        ("mic", false, _) => Err("The selected microphone is unavailable.".to_owned()),
        ("system", _, true) => Ok("system"),
        ("system", _, false) => Err("The selected system-audio source is unavailable.".to_owned()),
        _ => Err("Capture mode must be mic, system, or both.".to_owned()),
    }
}

fn is_transcrip_it_virtual(name: &str) -> bool {
    name.starts_with("transcrip_it_aec_source_") || name.starts_with("transcrip_it_aec_sink_")
}

fn cleanup_stale_echo_cancellation() -> Result<(), String> {
    let modules = pactl_output(&["list", "short", "modules"])?;
    let stale_module_ids = modules
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next()?;
            let module = fields.next()?;
            let arguments = fields.next().unwrap_or_default();
            (module == "module-echo-cancel" && arguments.contains("transcrip_it_aec_"))
                .then_some(id.to_owned())
        })
        .collect::<Vec<_>>();
    if stale_module_ids.is_empty() {
        return Ok(());
    }

    let sinks = parse_short_names(&pactl_output(&["list", "short", "sinks"])?);
    let sources = parse_short_names(&pactl_output(&["list", "short", "sources"])?);
    let fallback_sink = sinks
        .iter()
        .find(|(_, name)| !is_transcrip_it_virtual(name))
        .map(|(_, name)| name.clone());
    let fallback_source = sources
        .iter()
        .find(|(_, name)| !is_transcrip_it_virtual(name) && !name.ends_with(".monitor"))
        .map(|(_, name)| name.clone());

    if let Some(sink) = fallback_sink.as_deref() {
        if pactl_output(&["get-default-sink"])
            .is_ok_and(|current| is_transcrip_it_virtual(&current))
        {
            pactl_output(&["set-default-sink", sink])?;
        }
        let virtual_sink_ids = sinks
            .iter()
            .filter(|(_, name)| is_transcrip_it_virtual(name))
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>();
        for line in pactl_output(&["list", "short", "sink-inputs"])?.lines() {
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() > 1 && virtual_sink_ids.contains(&fields[1]) {
                pactl_output(&["move-sink-input", fields[0], sink])?;
            }
        }
    }
    if let Some(source) = fallback_source.as_deref() {
        if pactl_output(&["get-default-source"])
            .is_ok_and(|current| is_transcrip_it_virtual(&current))
        {
            pactl_output(&["set-default-source", source])?;
        }
    }
    for module_id in stale_module_ids {
        pactl_output(&["unload-module", &module_id])?;
    }
    Ok(())
}

fn parse_short_names(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            Some((fields.next()?.to_owned(), fields.next()?.to_owned()))
        })
        .collect()
}

fn audio_state_warnings(
    microphone_source: Option<&str>,
    system_source: Option<&str>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(source) = microphone_source {
        match pactl_output(&["get-source-mute", source]) {
            Ok(value) if value.ends_with("yes") => warnings.push(
                "Microphone is muted. System audio can still be recorded; unmute to include your voice."
                    .to_owned(),
            ),
            Err(_) => warnings.push(
                "Microphone is disconnected. Other available audio will continue recording."
                    .to_owned(),
            ),
            _ => {}
        }
    }
    if let Some(source) = system_source {
        let sink = source.strip_suffix(".monitor").unwrap_or(source);
        match pactl_output(&["get-sink-mute", sink]) {
            Ok(value) if value.ends_with("yes") => warnings.push(
                "Speakers are muted. System audio capture will continue, but playback is inaudible."
                    .to_owned(),
            ),
            Err(_) => warnings.push(
                "Speaker output is disconnected. Microphone audio will continue recording."
                    .to_owned(),
            ),
            _ => {}
        }
    }
    warnings
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
        .filter(|name| !is_transcrip_it_virtual(name))
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
    let mut warnings = recording.warnings.clone();
    warnings.extend(audio_state_warnings(
        recording.microphone_source.as_deref(),
        recording.system_source.as_deref(),
    ));
    warnings.sort();
    warnings.dedup();
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
        warnings,
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
    fn preserved_wav_chunks_restore_duration_without_a_completed_manifest() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "transcrip-it-chunk-recovery-{}-{unique}",
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
            r#"{"status":"recording","tracks":[{"capturedDurationSeconds":null}],"errors":[]}"#,
        )
        .unwrap();
        write_test_wav(&recording_path.join("mic-00000.wav"), 60_000);
        write_test_wav(&recording_path.join("mic-00001.wav"), 7_250);
        write_test_wav(&recording_path.join("system-00000.wav"), 60_000);
        fs::write(recording_path.join("transcription-mic.wav"), b"not a chunk").unwrap();

        assert_eq!(recover_completed_recording_metadata(&database).unwrap(), 1);
        let meeting = database.list_meetings(false).unwrap().remove(0);
        assert_eq!(meeting.duration_ms, Some(67_250));

        drop(database);
        fs::remove_dir_all(root).unwrap();
    }

    fn write_test_wav(path: &Path, duration_ms: u32) {
        let sample_rate = 48_000_u32;
        let channels = 1_u16;
        let block_align = channels * 2;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_bytes = u64::from(byte_rate) * u64::from(duration_ms) / 1000;
        let data_bytes = u32::try_from(data_bytes).unwrap();
        let mut header = [0_u8; 44];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16_u32.to_le_bytes());
        header[20..22].copy_from_slice(&1_u16.to_le_bytes());
        header[22..24].copy_from_slice(&channels.to_le_bytes());
        header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
        header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        header[32..34].copy_from_slice(&block_align.to_le_bytes());
        header[34..36].copy_from_slice(&16_u16.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
        fs::write(path, header).unwrap();
    }

    #[test]
    fn parses_microphone_and_monitor_sources() {
        let sources = parse_sources(
            "1\talsa_input.usb-mic\tmodule\ts16le\n2\talsa_output.pci.monitor\tmodule\ts16le\n3\ttranscrip_it_aec_sink_42.monitor\tmodule\tfloat32le",
        );
        assert_eq!(sources.len(), 2);
        assert!(!sources[0].monitor);
        assert!(sources[1].monitor);
    }

    #[test]
    fn combined_capture_falls_back_to_the_available_source() {
        assert_eq!(
            effective_capture_mode("both", false, true).unwrap(),
            "system"
        );
        assert_eq!(effective_capture_mode("both", true, false).unwrap(), "mic");
        assert!(effective_capture_mode("both", false, false).is_err());
        assert!(effective_capture_mode("mic", false, true).is_err());
    }
}
