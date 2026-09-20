use crate::database::{
    AuthorKind, Database, DerivativeKind, MeetingState, MeetingTransitionRequest,
    SegmentDerivativeInput, SegmentText, TranscriptSegmentInput, TranscriptSegmentRecord,
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
pub fn live_transcript_preview(
    app: tauri::AppHandle,
    database: tauri::State<'_, Database>,
    transcriber: tauri::State<'_, Transcriber>,
    meeting_id: String,
) -> Result<LiveTranscriptPreview, String> {
    let mut preview_active = transcriber
        .preview_active
        .lock()
        .map_err(|_| "live transcription state lock is poisoned".to_owned())?;
    if *preview_active {
        return Err("A live transcription update is already running.".to_owned());
    }
    *preview_active = true;
    drop(preview_active);

    let result = run_live_preview(&app, &database, &meeting_id);
    if let Ok(mut active) = transcriber.preview_active.lock() {
        *active = false;
    }
    result
}

fn run_live_preview(
    app: &tauri::AppHandle,
    database: &Database,
    meeting_id: &str,
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
    for (track_id, speaker_label) in [("mic", "Microphone"), ("system", "Meeting audio")] {
        let Some((chunk, chunk_index)) = latest_wav_chunk(&recording_path, track_id)? else {
            continue;
        };
        let normalized = preview_directory.join(format!("{track_id}-{run_id}.wav"));
        let status = Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-i"])
            .arg(&chunk)
            .args([
                "-af",
                "highpass=f=80,lowpass=f=7800,loudnorm=I=-16:LRA=11:TP=-1.5",
                "-ar",
                "16000",
                "-ac",
                "1",
            ])
            .arg(&normalized)
            .status()
            .map_err(|error| format!("Could not prepare the live audio preview: {error}"))?;
        if !status.success() {
            continue;
        }
        let output_base = preview_directory.join(format!("{track_id}-{run_id}"));
        let output = Command::new(&paths.binary)
            .args(["-m"])
            .arg(&paths.model)
            .args(["-f"])
            .arg(&normalized)
            .args(["-l", "en", "-oj", "-of"])
            .arg(&output_base)
            .args(["-np", "-sow", "-sns"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
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
            if !text.is_empty() {
                lines.push(LiveTranscriptLine {
                    track_id: track_id.to_owned(),
                    speaker_label: speaker_label.to_owned(),
                    chunk_index,
                    start_ms: chunk_index * 60_000,
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
pub fn transcribe_meeting(
    app: tauri::AppHandle,
    database: tauri::State<'_, Database>,
    transcriber: tauri::State<'_, Transcriber>,
    meeting_id: String,
    model_pack: Option<String>,
) -> Result<Vec<TranscriptSegmentRecord>, String> {
    let existing = database
        .list_transcript(&meeting_id)
        .map_err(|error| error.to_string())?;
    if !existing.is_empty() {
        return Ok(existing);
    }

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
    let result = run_transcription(
        &app,
        &database,
        &meeting_id,
        model_pack.as_deref().unwrap_or("fast"),
    );
    *active = None;
    result
}

fn run_transcription(
    app: &tauri::AppHandle,
    database: &Database,
    meeting_id: &str,
    model_pack: &str,
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
        let tracks = prepare_transcription_audio(&recording_path)?;
        let mut segments = Vec::new();
        for track in tracks {
            segments.extend(transcribe_track(&paths, &recording_path, &track)?);
        }
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

fn prepare_transcription_audio(recording_path: &Path) -> Result<Vec<PreparedTrack>, String> {
    let mic = wav_chunks(recording_path, "mic")?;
    let system = wav_chunks(recording_path, "system")?;
    if mic.is_empty() && system.is_empty() {
        return Err("No captured audio tracks are available.".to_owned());
    }
    let mut tracks = Vec::new();
    if !mic.is_empty() {
        tracks.push(prepare_track(recording_path, "mic", "Microphone", &mic)?);
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

fn prepare_track(
    recording_path: &Path,
    id: &'static str,
    speaker_label: &'static str,
    chunks: &[PathBuf],
) -> Result<PreparedTrack, String> {
    let output = recording_path.join(format!("transcription-{id}.wav"));
    let playlist = write_playlist(recording_path, &format!("transcribe-{id}"), chunks)?;
    let mut command = Command::new("ffmpeg");
    let status = command
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
        .arg(&output)
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
    let output = Command::new(&paths.binary)
        .args(["-m"])
        .arg(&paths.model)
        .args(["-f"])
        .arg(&track.path)
        .args(["-l", "en", "-oj", "-of"])
        .arg(&output_base)
        .args(["-np", "-sow", "-sns"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
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
            (!text.is_empty()).then_some(TranscriptSegmentInput {
                start_ms: segment.offsets.from,
                end_ms: segment.offsets.to,
                speaker_label: Some(track.speaker_label.to_owned()),
                source_track: Some(track.id.to_owned()),
                text,
            })
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
}
