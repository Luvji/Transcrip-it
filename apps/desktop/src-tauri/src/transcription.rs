use crate::{
    child_process::terminate_with_parent,
    database::{
        AuthorKind, Database, DerivativeKind, MeetingState, MeetingTransitionRequest,
        SegmentDerivativeInput, SegmentText, TranscriptSegmentInput, TranscriptSegmentRecord,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::Manager;

pub struct Transcriber {
    active: Mutex<Option<String>>,
    preview_active: Mutex<bool>,
}

impl Transcriber {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
            preview_active: Mutex::new(false),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionModelStatus {
    installed: bool,
    pack_id: String,
    pack: String,
    model_bytes: Option<u64>,
    engine_version: String,
}

#[derive(Clone, Copy)]
struct ModelSpec {
    id: &'static str,
    label: &'static str,
    filename: &'static str,
    download_url: &'static str,
    sha1: &'static str,
}

fn model_spec(pack: &str) -> Result<ModelSpec, String> {
    match pack {
        "fast" => Ok(ModelSpec {
            id: "fast",
            label: "Fast English",
            filename: "ggml-base.en.bin",
            download_url:
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
            sha1: "137c40403d78fd54d454da0f9bd998f78703390c",
        }),
        "balanced" => Ok(ModelSpec {
            id: "balanced",
            label: "Balanced English",
            filename: "ggml-small.en.bin",
            download_url:
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
            sha1: "db8a495a91d927739e50b3fc1cc4c6b8f6c2d022",
        }),
        "accuracy" => Ok(ModelSpec {
            id: "accuracy",
            label: "Accuracy English",
            filename: "ggml-medium.en.bin",
            download_url:
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin",
            sha1: "8c30f0e44ce9560643ebd10bbe50cd20eafd3723",
        }),
        _ => Err(format!("Unknown transcription model pack: {pack}")),
    }
}

#[derive(Deserialize)]
struct WhisperOutput {
    transcription: Vec<WhisperSegment>,
}

#[derive(Deserialize)]
struct WhisperSegment {
    offsets: WhisperOffsets,
    text: String,
}

#[derive(Deserialize)]
struct WhisperOffsets {
    from: i64,
    to: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTranscriptLine {
    track_id: String,
    speaker_label: String,
    chunk_index: u64,
    window_index: u64,
    start_ms: u64,
    text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTranscriptPreview {
    lines: Vec<LiveTranscriptLine>,
}

#[tauri::command]
pub fn transcription_model_status(
    app: tauri::AppHandle,
    pack: Option<String>,
) -> Result<TranscriptionModelStatus, String> {
    let spec = model_spec(pack.as_deref().unwrap_or("fast"))?;
    let paths = model_paths(&app, spec)?;
    let installed = paths.binary.is_file() && paths.model.is_file();
    let model_bytes = fs::metadata(&paths.model)
        .ok()
        .map(|metadata| metadata.len());
    let engine_version = if paths.binary.is_file() {
        Command::new(&paths.binary)
            .arg("--version")
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "whisper.cpp".to_owned())
    } else {
        "Not installed".to_owned()
    };
    Ok(TranscriptionModelStatus {
        installed,
        pack_id: spec.id.to_owned(),
        pack: spec.label.to_owned(),
        model_bytes,
        engine_version,
    })
}

#[tauri::command]
pub fn install_transcription_model(
    app: tauri::AppHandle,
    pack: String,
) -> Result<TranscriptionModelStatus, String> {
    let spec = model_spec(&pack)?;
    let paths = model_paths(&app, spec)?;
    let source = paths.root.as_path();
    fs::create_dir_all(
        source
            .parent()
            .ok_or_else(|| "Invalid model storage path.".to_owned())?,
    )
    .map_err(|error| format!("Could not create model storage: {error}"))?;
    if !source.join(".git").exists() {
        command_success(
            Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    "--branch",
                    "v1.9.4",
                    "https://github.com/ggml-org/whisper.cpp.git",
                ])
                .arg(source),
            "download the transcription engine",
        )?;
    }
    if !paths.binary.is_file() {
        let build = source.join("build");
        command_success(
            Command::new("cmake")
                .arg("-S")
                .arg(source)
                .arg("-B")
                .arg(&build)
                .args([
                    "-DWHISPER_BUILD_TESTS=OFF",
                    "-DWHISPER_BUILD_EXAMPLES=ON",
                    "-DCMAKE_BUILD_TYPE=Release",
                ]),
            "configure the transcription engine",
        )?;
        command_success(
            Command::new("cmake")
                .args(["--build"])
                .arg(&build)
                .args(["--config", "Release", "-j", "4"]),
            "compile the transcription engine",
        )?;
    }
    if !paths.model.is_file() {
        fs::create_dir_all(
            paths
                .model
                .parent()
                .ok_or_else(|| "Invalid model path.".to_owned())?,
        )
        .map_err(|error| format!("Could not create model directory: {error}"))?;
        let temporary = paths.model.with_extension("download");
        command_success(
            Command::new("curl")
                .args(["-L", "--fail", "--retry", "3", "-o"])
                .arg(&temporary)
                .arg(spec.download_url),
            &format!("download the {} model", spec.label),
        )?;
        let checksum = Command::new("sha1sum")
            .arg(&temporary)
            .output()
            .map_err(|error| format!("Could not verify model integrity: {error}"))?;
        let actual = String::from_utf8_lossy(&checksum.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        if actual != spec.sha1 {
            return Err("Downloaded model failed its integrity check.".to_owned());
        }
        fs::rename(&temporary, &paths.model)
            .map_err(|error| format!("Could not finalize model installation: {error}"))?;
    }
    transcription_model_status(app, Some(spec.id.to_owned()))
}

#[tauri::command]
pub fn list_transcript(
    database: tauri::State<'_, Database>,
    meeting_id: String,
) -> Result<Vec<TranscriptSegmentRecord>, String> {
    database
        .list_transcript(&meeting_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_transcript(
    database: tauri::State<'_, Database>,
    meeting_id: String,
) -> Result<bool, String> {
    database
        .delete_transcript(&meeting_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn rename_transcript_speaker(
    database: tauri::State<'_, Database>,
    meeting_id: String,
    current_label: String,
    new_label: String,
) -> Result<usize, String> {
    database
        .rename_speaker_label(&meeting_id, &current_label, &new_label)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn live_transcript_preview(
    app: tauri::AppHandle,
    transcriber: tauri::State<'_, Transcriber>,
    meeting_id: String,
    microphone_track: Option<String>,
) -> Result<LiveTranscriptPreview, String> {
    let microphone_track = validate_microphone_track(microphone_track.as_deref())?.to_owned();
    {
        let mut preview_active = transcriber
            .preview_active
            .lock()
            .map_err(|_| "live transcription state lock is poisoned".to_owned())?;
        if *preview_active {
            return Err("A live transcription update is already running.".to_owned());
        }
        *preview_active = true;
    }

    let preview_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let database = preview_app.state::<Database>();
        run_live_preview(&preview_app, &database, &meeting_id, &microphone_track)
    })
    .await
    .unwrap_or_else(|error| {
        Err(format!(
            "Live transcription worker stopped unexpectedly: {error}"
        ))
    });
    if let Ok(mut active) = transcriber.preview_active.lock() {
        *active = false;
    }
    result
}

fn run_live_preview(
    app: &tauri::AppHandle,
    database: &Database,
    meeting_id: &str,
    microphone_track: &str,
) -> Result<LiveTranscriptPreview, String> {
    let paths = model_paths(app, model_spec("fast")?)?;
    if !paths.binary.is_file() || !paths.model.is_file() {
        return Err(
            "Install the Fast English model in Settings to enable live transcript previews."
                .to_owned(),
        );
    }
    let state = database
        .meeting_state(meeting_id)
        .map_err(|error| error.to_string())?;
    if !matches!(state, MeetingState::Recording | MeetingState::Paused) {
        return Err("Live transcription is available only while recording.".to_owned());
    }
    let recording_path = database
        .meeting_recording_path(meeting_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The active recording path is not available.".to_owned())?;
    let recording_path = PathBuf::from(recording_path);
    let preview_directory = recording_path.join("live-preview");
    fs::create_dir_all(&preview_directory)
        .map_err(|error| format!("Could not create live preview storage: {error}"))?;
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let mut lines = Vec::new();
    for (source_track, track_id, speaker_label) in [
        (microphone_track, "mic", "Microphone"),
        ("system", "system", "Meeting audio"),
    ] {
        let mut effective_source = source_track;
        let mut latest = latest_wav_chunk(&recording_path, effective_source)?;
        if latest.is_none() && effective_source == "mic_raw" {
            effective_source = "mic";
            latest = latest_wav_chunk(&recording_path, effective_source)?;
        }
        let Some((chunk, chunk_index)) = latest else {
            continue;
        };
        let duration_ms = wav_duration_ms(&chunk)?;
        if duration_ms < 3_000 {
            continue;
        }
        const WINDOW_MS: u64 = 15_000;
        let window_index = (duration_ms.saturating_sub(1) / WINDOW_MS).min(3);
        let window_start_ms = window_index * WINDOW_MS;
        let window_duration_ms = duration_ms.saturating_sub(window_start_ms).min(WINDOW_MS);
        if window_duration_ms < 3_000 {
            continue;
        }
        let normalized = preview_directory.join(format!("{track_id}-{run_id}.wav"));
        let mut normalize_command = Command::new("ffmpeg");
        normalize_command
            .args([
                "-y",
                "-v",
                "error",
                "-ss",
                &format!("{:.3}", window_start_ms as f64 / 1000.0),
                "-t",
                &format!("{:.3}", window_duration_ms as f64 / 1000.0),
                "-i",
            ])
            .arg(&chunk)
            .args([
                "-af",
                "highpass=f=80,lowpass=f=7800",
                "-ar",
                "16000",
                "-ac",
                "1",
            ])
            .arg(&normalized);
        terminate_with_parent(&mut normalize_command);
        let status = normalize_command
            .status()
            .map_err(|error| format!("Could not prepare the live audio preview: {error}"))?;
        if !status.success() {
            continue;
        }
        if !wav_has_speech_energy(&normalized)? {
            let _ = fs::remove_file(normalized);
            continue;
        }
        let output_base = preview_directory.join(format!("{track_id}-{run_id}"));
        let mut whisper_command = Command::new(&paths.binary);
        whisper_command
            .args(["-m"])
            .arg(&paths.model)
            .args(["-f"])
            .arg(&normalized)
            .args(["-l", "en", "-oj", "-of"])
            .arg(&output_base)
            .args(["-np", "-sow", "-sns"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        terminate_with_parent(&mut whisper_command);
        let output = whisper_command
            .output()
            .map_err(|error| format!("Could not start the live transcript preview: {error}"))?;
        if output.status.success() {
            let json_path = output_base.with_extension("json");
            let parsed: WhisperOutput = serde_json::from_str(
                &fs::read_to_string(&json_path)
                    .map_err(|error| format!("Could not read live transcript output: {error}"))?,
            )
            .map_err(|error| format!("Could not parse live transcript output: {error}"))?;
            let text = parsed
                .transcription
                .into_iter()
                .map(|segment| segment.text.trim().to_owned())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !text.is_empty() && !looks_like_hallucination(&text) {
                lines.push(LiveTranscriptLine {
                    track_id: track_id.to_owned(),
                    speaker_label: speaker_label.to_owned(),
                    chunk_index,
                    window_index,
                    start_ms: chunk_index * 60_000 + window_start_ms,
                    text,
                });
            }
            let _ = fs::remove_file(json_path);
        }
        let _ = fs::remove_file(normalized);
    }
    Ok(LiveTranscriptPreview { lines })
}

#[tauri::command]
pub fn search_transcripts(
    database: tauri::State<'_, Database>,
    query: String,
) -> Result<Vec<crate::database::TranscriptSearchResult>, String> {
    database
        .search_transcripts(&query)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn correct_transcript_segment(
    database: tauri::State<'_, Database>,
    segment_id: String,
    correction_id: String,
    idempotency_key: String,
    text: String,
) -> Result<SegmentText, String> {
    database
        .append_segment_derivative(&SegmentDerivativeInput {
            id: correction_id,
            idempotency_key,
            segment_id: segment_id.clone(),
            kind: DerivativeKind::Correction,
            author: AuthorKind::User,
            text,
            language_code: Some("en".to_owned()),
            model_id: None,
        })
        .map_err(|error| error.to_string())?;
    database
        .segment_text(&segment_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn export_transcript(
    database: tauri::State<'_, Database>,
    meeting_id: String,
    format: String,
) -> Result<String, String> {
    if !matches!(format.as_str(), "markdown" | "text") {
        return Err("Export format must be markdown or text.".to_owned());
    }
    let title = database
        .meeting_title(&meeting_id)
        .map_err(|error| error.to_string())?;
    let segments = database
        .list_transcript(&meeting_id)
        .map_err(|error| error.to_string())?;
    if segments.is_empty() {
        return Err("This meeting does not have a transcript yet.".to_owned());
    }
    let recording_path = database
        .meeting_recording_path(&meeting_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "This meeting has no recording directory.".to_owned())?;
    let export_directory = PathBuf::from(recording_path).join("exports");
    fs::create_dir_all(&export_directory)
        .map_err(|error| format!("Could not create export directory: {error}"))?;
    let extension = if format == "markdown" { "md" } else { "txt" };
    let path = export_directory.join(format!("transcript.{extension}"));
    let mut content = if format == "markdown" {
        format!("# {title}\n\n")
    } else {
        format!("{title}\n{}\n\n", "=".repeat(title.chars().count()))
    };
    for segment in segments {
        let timestamp = format_timestamp(segment.start_ms);
        let speaker = segment.speaker_label.as_deref().unwrap_or("Speaker");
        if format == "markdown" {
            content.push_str(&format!(
                "**[{timestamp}] {speaker}**  \n{}\n\n",
                segment.display_text
            ));
        } else {
            content.push_str(&format!(
                "[{timestamp}] {speaker}: {}\n",
                segment.display_text
            ));
        }
    }
    fs::write(&path, content)
        .map_err(|error| format!("Could not write transcript export: {error}"))?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn transcribe_meeting(
    app: tauri::AppHandle,
    transcriber: tauri::State<'_, Transcriber>,
    meeting_id: String,
    model_pack: Option<String>,
    microphone_track: Option<String>,
) -> Result<Vec<TranscriptSegmentRecord>, String> {
    let microphone_track = validate_microphone_track(microphone_track.as_deref())?.to_owned();
    let existing = {
        let database = app.state::<Database>();
        database
            .list_transcript(&meeting_id)
            .map_err(|error| error.to_string())?
    };
    if !existing.is_empty() {
        return Ok(existing);
    }

    {
        let mut active = transcriber
            .active
            .lock()
            .map_err(|_| "transcription state lock is poisoned".to_owned())?;
        if let Some(active_id) = active.as_ref() {
            return Err(format!(
                "Transcription is already running for meeting {active_id}."
            ));
        }
        *active = Some(meeting_id.clone());
    }
    let transcription_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let database = transcription_app.state::<Database>();
        run_transcription(
            &transcription_app,
            &database,
            &meeting_id,
            model_pack.as_deref().unwrap_or("fast"),
            &microphone_track,
        )
    })
    .await
    .unwrap_or_else(|error| {
        Err(format!(
            "Transcription worker stopped unexpectedly: {error}"
        ))
    });
    let mut active = transcriber
        .active
        .lock()
        .map_err(|_| "transcription state lock is poisoned".to_owned())?;
    *active = None;
    result
}

fn run_transcription(
    app: &tauri::AppHandle,
    database: &Database,
    meeting_id: &str,
    model_pack: &str,
    microphone_track: &str,
) -> Result<Vec<TranscriptSegmentRecord>, String> {
    let spec = model_spec(model_pack)?;
    let paths = model_paths(app, spec)?;
    if !paths.binary.is_file() || !paths.model.is_file() {
        return Err(format!(
            "The {} transcription model is not installed. Install it from Settings.",
            spec.label
        ));
    }
    let recording_path = database
        .meeting_recording_path(meeting_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "This meeting has no completed recording.".to_owned())?;
    let recording_path = PathBuf::from(recording_path);
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let initial_state = database
        .meeting_state(meeting_id)
        .map_err(|error| error.to_string())?;
    if !matches!(initial_state, MeetingState::Ready | MeetingState::Failed) {
        return Err(format!(
            "Meeting cannot be transcribed while in state {initial_state:?}."
        ));
    }
    database
        .transition_meeting(&MeetingTransitionRequest {
            meeting_id: meeting_id.to_owned(),
            idempotency_key: format!("transcription-start:{meeting_id}:{run_id}"),
            expected_state: initial_state,
            next_state: MeetingState::Processing,
        })
        .map_err(|error| error.to_string())?;

    let processing = (|| {
        let tracks = prepare_transcription_audio(&recording_path, microphone_track)?;
        let mut segments = Vec::new();
        for track in tracks {
            segments.extend(transcribe_track(&paths, &recording_path, &track)?);
        }
        segments = remove_probable_system_bleed(segments);
        segments.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
        database
            .store_source_transcript(meeting_id, &segments)
            .map_err(|error| error.to_string())
    })();

    let final_state = if processing.is_ok() {
        MeetingState::Ready
    } else {
        MeetingState::Failed
    };
    let transition_result = database.transition_meeting(&MeetingTransitionRequest {
        meeting_id: meeting_id.to_owned(),
        idempotency_key: format!("transcription-finish:{meeting_id}:{run_id}"),
        expected_state: MeetingState::Processing,
        next_state: final_state,
    });
    if let Err(error) = transition_result {
        return Err(error.to_string());
    }
    processing
}

struct PreparedTrack {
    id: &'static str,
    speaker_label: &'static str,
    path: PathBuf,
}

fn prepare_transcription_audio(
    recording_path: &Path,
    microphone_track: &str,
) -> Result<Vec<PreparedTrack>, String> {
    let mut effective_microphone_track = microphone_track;
    let mut mic = wav_chunks(recording_path, effective_microphone_track)?;
    if mic.is_empty() && effective_microphone_track == "mic_raw" {
        effective_microphone_track = "mic";
        mic = wav_chunks(recording_path, effective_microphone_track)?;
    }
    let system = wav_chunks(recording_path, "system")?;
    if mic.is_empty() && system.is_empty() {
        return Err("No captured audio tracks are available.".to_owned());
    }
    let mut tracks = Vec::new();
    if !mic.is_empty() {
        tracks.push(prepare_track(
            recording_path,
            "mic",
            if effective_microphone_track == "mic_raw" {
                "Microphone (original)"
            } else {
                "Microphone (echo-reduced)"
            },
            &mic,
        )?);
    }
    if !system.is_empty() {
        tracks.push(prepare_track(
            recording_path,
            "system",
            "Meeting audio",
            &system,
        )?);
    }
    Ok(tracks)
}

fn validate_microphone_track(track: Option<&str>) -> Result<&str, String> {
    match track.unwrap_or("mic_raw") {
        "mic" => Ok("mic"),
        "mic_raw" => Ok("mic_raw"),
        _ => Err("Microphone transcription source must be mic or mic_raw.".to_owned()),
    }
}

fn prepare_track(
    recording_path: &Path,
    id: &'static str,
    speaker_label: &'static str,
    chunks: &[PathBuf],
) -> Result<PreparedTrack, String> {
    let output = recording_path.join(format!("transcription-{id}.wav"));
    let playlist = write_playlist(recording_path, &format!("transcribe-{id}"), chunks)?;
    let mut command = Command::new("ffmpeg");
    command
        .args(["-y", "-v", "error", "-f", "concat", "-safe", "0", "-i"])
        .arg(playlist)
        .args([
            "-af",
            "highpass=f=80,lowpass=f=7800,loudnorm=I=-16:LRA=11:TP=-1.5",
            "-ar",
            "16000",
            "-ac",
            "1",
        ])
        .arg(&output);
    terminate_with_parent(&mut command);
    let status = command
        .status()
        .map_err(|error| format!("Could not prepare transcription audio: {error}"))?;
    if !status.success() {
        return Err("FFmpeg could not prepare the transcription audio.".to_owned());
    }
    Ok(PreparedTrack {
        id,
        speaker_label,
        path: output,
    })
}

fn transcribe_track(
    paths: &ModelPaths,
    recording_path: &Path,
    track: &PreparedTrack,
) -> Result<Vec<TranscriptSegmentInput>, String> {
    let output_base = recording_path.join(format!("transcript-whisper-{}", track.id));
    let mut command = Command::new(&paths.binary);
    command
        .args(["-m"])
        .arg(&paths.model)
        .args(["-f"])
        .arg(&track.path)
        .args(["-l", "en", "-oj", "-of"])
        .arg(&output_base)
        .args(["-np", "-sow", "-sns"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    terminate_with_parent(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("Could not start local transcription: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Local transcription failed for {}: {}",
            track.speaker_label,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("unknown error")
        ));
    }
    let json_path = output_base.with_extension("json");
    let parsed: WhisperOutput = serde_json::from_str(
        &fs::read_to_string(&json_path)
            .map_err(|error| format!("Could not read transcription output: {error}"))?,
    )
    .map_err(|error| format!("Could not parse transcription output: {error}"))?;
    let segments = parsed
        .transcription
        .into_iter()
        .filter_map(|segment| {
            let text = segment.text.trim().to_owned();
            (!text.is_empty() && !looks_like_hallucination(&text)).then_some(
                TranscriptSegmentInput {
                    start_ms: segment.offsets.from,
                    end_ms: segment.offsets.to,
                    speaker_label: Some(track.speaker_label.to_owned()),
                    source_track: Some(track.id.to_owned()),
                    text,
                },
            )
        })
        .collect::<Vec<_>>();
    Ok(merge_continuous_segments(segments))
}

fn merge_continuous_segments(segments: Vec<TranscriptSegmentInput>) -> Vec<TranscriptSegmentInput> {
    const MAX_GAP_MS: i64 = 1_500;
    const MAX_GROUP_MS: i64 = 45_000;
    const MAX_GROUP_CHARACTERS: usize = 600;
    let mut grouped: Vec<TranscriptSegmentInput> = Vec::new();
    for segment in segments {
        let can_merge = grouped.last().is_some_and(|current| {
            current.speaker_label == segment.speaker_label
                && current.source_track == segment.source_track
                && segment.start_ms.saturating_sub(current.end_ms) <= MAX_GAP_MS
                && segment.end_ms.saturating_sub(current.start_ms) <= MAX_GROUP_MS
                && current.text.chars().count() + segment.text.chars().count()
                    < MAX_GROUP_CHARACTERS
        });
        if can_merge {
            let current = grouped.last_mut().expect("group exists");
            if current.text.ends_with('-') {
                current.text.pop();
            } else if !current.text.chars().last().is_some_and(char::is_whitespace) {
                current.text.push(' ');
            }
            current.text.push_str(segment.text.trim());
            current.end_ms = current.end_ms.max(segment.end_ms);
        } else {
            grouped.push(segment);
        }
    }
    grouped
}

fn remove_probable_system_bleed(
    segments: Vec<TranscriptSegmentInput>,
) -> Vec<TranscriptSegmentInput> {
    let system_segments = segments
        .iter()
        .filter(|segment| segment.source_track.as_deref() == Some("system"))
        .cloned()
        .collect::<Vec<_>>();
    segments
        .into_iter()
        .filter(|segment| {
            if segment.speaker_label.as_deref() != Some("Microphone (original)") {
                return true;
            }
            !system_segments
                .iter()
                .any(|system| passages_are_probable_duplicates(segment, system))
        })
        .collect()
}

fn passages_are_probable_duplicates(
    microphone: &TranscriptSegmentInput,
    system: &TranscriptSegmentInput,
) -> bool {
    let overlap_ms = microphone
        .end_ms
        .min(system.end_ms)
        .saturating_sub(microphone.start_ms.max(system.start_ms));
    let shorter_duration_ms = microphone
        .end_ms
        .saturating_sub(microphone.start_ms)
        .min(system.end_ms.saturating_sub(system.start_ms));
    if overlap_ms <= 0
        || shorter_duration_ms <= 0
        || overlap_ms.saturating_mul(100) < shorter_duration_ms.saturating_mul(65)
    {
        return false;
    }

    let mut microphone_words = normalized_words(&microphone.text);
    let mut system_words = normalized_words(&system.text);
    if microphone_words.len().min(system_words.len()) < 4 {
        return false;
    }
    microphone_words.sort_unstable();
    system_words.sort_unstable();
    let mut microphone_index = 0;
    let mut system_index = 0;
    let mut matching_words = 0;
    while microphone_index < microphone_words.len() && system_index < system_words.len() {
        match microphone_words[microphone_index].cmp(&system_words[system_index]) {
            std::cmp::Ordering::Less => microphone_index += 1,
            std::cmp::Ordering::Greater => system_index += 1,
            std::cmp::Ordering::Equal => {
                matching_words += 1;
                microphone_index += 1;
                system_index += 1;
            }
        }
    }
    matching_words * 200 >= (microphone_words.len() + system_words.len()) * 82
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn wav_chunks(directory: &Path, track: &str) -> Result<Vec<PathBuf>, String> {
    let prefix = format!("{track}-");
    let mut chunks = fs::read_dir(directory)
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
    Ok(chunks)
}

fn latest_wav_chunk(directory: &Path, track: &str) -> Result<Option<(PathBuf, u64)>, String> {
    let Some(path) = wav_chunks(directory, track)?.pop() else {
        return Ok(None);
    };
    let index = path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(&format!("{track}-")))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("Could not read the {track} chunk index."))?;
    Ok(Some((path, index)))
}

fn wav_duration_ms(path: &Path) -> Result<u64, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("Could not read WAV timing data {}: {error}", path.display()))?;
    let wav = parse_pcm_wav(&bytes)?;
    let bytes_per_second =
        u64::from(wav.sample_rate) * u64::from(wav.channels) * u64::from(wav.bits_per_sample / 8);
    if bytes_per_second == 0 {
        return Err("WAV audio format has an invalid sample rate.".to_owned());
    }
    Ok((wav.data.len() as u64).saturating_mul(1000) / bytes_per_second)
}

fn wav_has_speech_energy(path: &Path) -> Result<bool, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "Could not read live preview audio {}: {error}",
            path.display()
        )
    })?;
    let wav = parse_pcm_wav(&bytes)?;
    if wav.bits_per_sample != 16 {
        return Err("Live preview requires 16-bit PCM WAV audio.".to_owned());
    }
    let mut sample_count = 0_u64;
    let mut squared_sum = 0_f64;
    let mut peak = 0_f64;
    let (samples, _) = wav.data.as_chunks::<2>();
    for sample in samples {
        let amplitude = f64::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0;
        squared_sum += amplitude * amplitude;
        peak = peak.max(amplitude.abs());
        sample_count += 1;
    }
    if sample_count == 0 {
        return Ok(false);
    }
    let rms = (squared_sum / sample_count as f64).sqrt();
    Ok(rms >= 0.0025 && peak >= 0.012)
}

struct PcmWav<'a> {
    channels: u16,
    sample_rate: u32,
    bits_per_sample: u16,
    data: &'a [u8],
}

fn parse_pcm_wav(bytes: &[u8]) -> Result<PcmWav<'_>, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Live preview input is not a valid WAV file.".to_owned());
    }
    let mut offset = 12_usize;
    let mut format = None;
    let mut audio = None;
    while offset.saturating_add(8) <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
        let start = offset + 8;
        let end = start.saturating_add(size).min(bytes.len());
        if id == b"fmt " && end.saturating_sub(start) >= 16 {
            let encoding = u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap());
            if encoding != 1 {
                return Err("Live preview input must use PCM WAV audio.".to_owned());
            }
            format = Some((
                u16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap()),
                u32::from_le_bytes(bytes[start + 4..start + 8].try_into().unwrap()),
                u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()),
            ));
        } else if id == b"data" {
            audio = Some(&bytes[start..end]);
        }
        offset = start.saturating_add(size).saturating_add(size % 2);
    }
    let (channels, sample_rate, bits_per_sample) =
        format.ok_or_else(|| "WAV audio format is missing.".to_owned())?;
    Ok(PcmWav {
        channels,
        sample_rate,
        bits_per_sample,
        data: audio.ok_or_else(|| "WAV audio data is missing.".to_owned())?,
    })
}

fn looks_like_hallucination(text: &str) -> bool {
    let words = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| !character.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.len() < 8 {
        return false;
    }
    for width in [2_usize, 3, 4] {
        let mut counts = std::collections::HashMap::new();
        for phrase in words.windows(width) {
            *counts.entry(phrase.join(" ")).or_insert(0_usize) += 1;
        }
        if counts
            .values()
            .copied()
            .max()
            .is_some_and(|count| count >= 3 && count * width * 2 >= words.len())
        {
            return true;
        }
    }
    false
}

fn write_playlist(directory: &Path, name: &str, chunks: &[PathBuf]) -> Result<PathBuf, String> {
    let path = directory.join(format!("{name}.ffconcat"));
    let entries = chunks
        .iter()
        .map(|chunk| {
            format!(
                "file '{}'\n",
                chunk.to_string_lossy().replace('\'', "'\\''")
            )
        })
        .collect::<String>();
    fs::write(&path, format!("ffconcat version 1.0\n{entries}"))
        .map_err(|error| format!("Could not create audio playlist: {error}"))?;
    Ok(path)
}

struct ModelPaths {
    root: PathBuf,
    binary: PathBuf,
    model: PathBuf,
}

fn model_paths(app: &tauri::AppHandle, spec: ModelSpec) -> Result<ModelPaths, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("models")
        .join("whisper.cpp");
    Ok(ModelPaths {
        root: root.clone(),
        binary: root.join("build/bin/whisper-cli"),
        model: root.join("models").join(spec.filename),
    })
}

fn format_timestamp(milliseconds: i64) -> String {
    format!(
        "{:02}:{:02}",
        milliseconds / 60_000,
        (milliseconds / 1000) % 60
    )
}

fn command_success(command: &mut Command, action: &str) -> Result<(), String> {
    let output = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("Could not {action}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not {action}: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("unknown error")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamped_whisper_segments() {
        let parsed: WhisperOutput = serde_json::from_str(
            r#"{"transcription":[{"offsets":{"from":120,"to":980},"text":" Hello"}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.transcription[0].offsets.from, 120);
        assert_eq!(parsed.transcription[0].text.trim(), "Hello");
    }

    #[test]
    fn exposes_three_integrity_checked_english_model_packs() {
        assert_eq!(model_spec("fast").unwrap().filename, "ggml-base.en.bin");
        assert_eq!(
            model_spec("balanced").unwrap().filename,
            "ggml-small.en.bin"
        );
        assert_eq!(
            model_spec("accuracy").unwrap().filename,
            "ggml-medium.en.bin"
        );
        assert!(model_spec("unknown").is_err());
    }

    #[test]
    fn groups_continuous_speech_but_preserves_pauses_and_speakers() {
        let segments = vec![
            TranscriptSegmentInput {
                start_ms: 0,
                end_ms: 1_000,
                speaker_label: Some("Microphone".to_owned()),
                source_track: Some("mic".to_owned()),
                text: "This is one".to_owned(),
            },
            TranscriptSegmentInput {
                start_ms: 1_300,
                end_ms: 2_500,
                speaker_label: Some("Microphone".to_owned()),
                source_track: Some("mic".to_owned()),
                text: "continuous thought.".to_owned(),
            },
            TranscriptSegmentInput {
                start_ms: 4_500,
                end_ms: 5_000,
                speaker_label: Some("Microphone".to_owned()),
                source_track: Some("mic".to_owned()),
                text: "After a pause.".to_owned(),
            },
            TranscriptSegmentInput {
                start_ms: 5_100,
                end_ms: 6_000,
                speaker_label: Some("Meeting audio".to_owned()),
                source_track: Some("system".to_owned()),
                text: "Another channel.".to_owned(),
            },
        ];

        let grouped = merge_continuous_segments(segments);
        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped[0].text, "This is one continuous thought.");
        assert_eq!(grouped[0].end_ms, 2_500);
        assert_eq!(grouped[2].speaker_label.as_deref(), Some("Meeting audio"));
    }

    #[test]
    fn removes_only_strong_time_aligned_system_bleed_from_original_mic() {
        let original_duplicate = TranscriptSegmentInput {
            start_ms: 1_100,
            end_ms: 5_000,
            speaker_label: Some("Microphone (original)".to_owned()),
            source_track: Some("mic".to_owned()),
            text: "We should review the release plan tomorrow morning.".to_owned(),
        };
        let system = TranscriptSegmentInput {
            start_ms: 1_000,
            end_ms: 4_900,
            speaker_label: Some("Meeting audio".to_owned()),
            source_track: Some("system".to_owned()),
            text: "We should review release plan tomorrow morning".to_owned(),
        };
        let local_voice = TranscriptSegmentInput {
            start_ms: 5_100,
            end_ms: 7_500,
            speaker_label: Some("Microphone (original)".to_owned()),
            source_track: Some("mic".to_owned()),
            text: "I will send my separate notes after this call.".to_owned(),
        };

        let filtered = remove_probable_system_bleed(vec![
            original_duplicate.clone(),
            system.clone(),
            local_voice.clone(),
        ]);
        assert_eq!(filtered, vec![system, local_voice]);

        let uncertain_mixed_voice = TranscriptSegmentInput {
            text: format!(
                "{} I disagree and need another option.",
                original_duplicate.text
            ),
            ..original_duplicate
        };
        let preserved =
            remove_probable_system_bleed(vec![uncertain_mixed_voice.clone(), filtered[0].clone()]);
        assert_eq!(preserved, vec![uncertain_mixed_voice, filtered[0].clone()]);
    }

    #[test]
    fn keeps_short_or_processed_microphone_matches() {
        let system = TranscriptSegmentInput {
            start_ms: 0,
            end_ms: 2_000,
            speaker_label: Some("Meeting audio".to_owned()),
            source_track: Some("system".to_owned()),
            text: "Thank you".to_owned(),
        };
        let processed = TranscriptSegmentInput {
            speaker_label: Some("Microphone (echo-reduced)".to_owned()),
            source_track: Some("mic".to_owned()),
            ..system.clone()
        };
        let original_short = TranscriptSegmentInput {
            speaker_label: Some("Microphone (original)".to_owned()),
            source_track: Some("mic".to_owned()),
            ..system.clone()
        };

        let filtered = remove_probable_system_bleed(vec![
            processed.clone(),
            original_short.clone(),
            system.clone(),
        ]);
        assert_eq!(filtered, vec![processed, original_short, system]);
    }

    #[test]
    fn rejects_repetitive_whisper_hallucinations() {
        assert!(looks_like_hallucination(
            "If you are able to do it, you will be able to do it. If you are able to do it, you will be able to do it."
        ));
        assert!(looks_like_hallucination(
            "All right. All right. All right. All right. All right."
        ));
        assert!(!looks_like_hallucination(
            "We reviewed the release plan and agreed to test recording recovery tomorrow."
        ));
    }

    #[test]
    fn defaults_to_voice_safe_original_microphone() {
        assert_eq!(validate_microphone_track(None).unwrap(), "mic_raw");
        assert_eq!(validate_microphone_track(Some("mic")).unwrap(), "mic");
        assert!(validate_microphone_track(Some("system")).is_err());
    }

    #[test]
    fn reads_pcm_wav_duration_and_energy() {
        let sample_rate = 16_000_u32;
        let samples = vec![2_000_i16; sample_rate as usize];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let parsed = parse_pcm_wav(&bytes).unwrap();
        assert_eq!(parsed.sample_rate, sample_rate);
        assert_eq!(parsed.data.len(), sample_rate as usize * 2);
    }
}
