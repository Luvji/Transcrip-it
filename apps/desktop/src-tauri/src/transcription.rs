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
}

impl Transcriber {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(None),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionModelStatus {
    installed: bool,
    pack: String,
    model_bytes: Option<u64>,
    engine_version: String,
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

#[tauri::command]
pub fn transcription_model_status(
    app: tauri::AppHandle,
) -> Result<TranscriptionModelStatus, String> {
    let paths = model_paths(&app)?;
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
        pack: "Balanced English".to_owned(),
        model_bytes,
        engine_version,
    })
}

#[tauri::command]
pub fn install_transcription_model(
    app: tauri::AppHandle,
) -> Result<TranscriptionModelStatus, String> {
    let paths = model_paths(&app)?;
    let root = paths
        .binary
        .ancestors()
        .nth(4)
        .ok_or_else(|| "Invalid transcription engine path.".to_owned())?;
    fs::create_dir_all(root).map_err(|error| format!("Could not create model storage: {error}"))?;
    let source = root.join("whisper.cpp");
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
                .arg(&source),
            "download the transcription engine",
        )?;
    }
    if !paths.binary.is_file() {
        let build = source.join("build");
        command_success(
            Command::new("cmake")
                .arg("-S")
                .arg(&source)
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
                .arg("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin"),
            "download the Balanced English model",
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
        if actual != "137c40403d78fd54d454da0f9bd998f78703390c" {
            return Err("Downloaded model failed its integrity check.".to_owned());
        }
        fs::rename(&temporary, &paths.model)
            .map_err(|error| format!("Could not finalize model installation: {error}"))?;
    }
    transcription_model_status(app)
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
    let result = run_transcription(&app, &database, &meeting_id);
    *active = None;
    result
}

fn run_transcription(
    app: &tauri::AppHandle,
    database: &Database,
    meeting_id: &str,
) -> Result<Vec<TranscriptSegmentRecord>, String> {
    let paths = model_paths(app)?;
    if !paths.binary.is_file() || !paths.model.is_file() {
        return Err("The Balanced English transcription model is not installed.".to_owned());
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
        let input = prepare_transcription_audio(&recording_path)?;
        let output_base = recording_path.join("transcript-whisper");
        let output = Command::new(&paths.binary)
            .args(["-m"])
            .arg(&paths.model)
            .args(["-f"])
            .arg(&input)
            .args(["-l", "en", "-oj", "-of"])
            .arg(&output_base)
            .arg("-np")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|error| format!("Could not start local transcription: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Local transcription failed: {}",
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
                    text,
                })
            })
            .collect::<Vec<_>>();
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

fn prepare_transcription_audio(recording_path: &Path) -> Result<PathBuf, String> {
    let mic = wav_chunks(recording_path, "mic")?;
    let system = wav_chunks(recording_path, "system")?;
    if mic.is_empty() && system.is_empty() {
        return Err("No captured audio tracks are available.".to_owned());
    }
    let output = recording_path.join("transcription-input.wav");
    let mut command = Command::new("ffmpeg");
    command.args(["-y", "-v", "error"]);
    let mut inputs = 0;
    if !mic.is_empty() {
        let playlist = write_playlist(recording_path, "transcribe-mic", &mic)?;
        command
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(playlist);
        inputs += 1;
    }
    if !system.is_empty() {
        let playlist = write_playlist(recording_path, "transcribe-system", &system)?;
        command
            .args(["-f", "concat", "-safe", "0", "-i"])
            .arg(playlist);
        inputs += 1;
    }
    if inputs == 2 {
        command.args([
            "-filter_complex",
            "[0:a][1:a]amix=inputs=2:duration=longest:normalize=1",
        ]);
    }
    let status = command
        .args(["-ar", "16000", "-ac", "1"])
        .arg(&output)
        .status()
        .map_err(|error| format!("Could not prepare transcription audio: {error}"))?;
    if !status.success() {
        return Err("FFmpeg could not prepare the transcription audio.".to_owned());
    }
    Ok(output)
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
    binary: PathBuf,
    model: PathBuf,
}

fn model_paths(app: &tauri::AppHandle) -> Result<ModelPaths, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("models")
        .join("whisper.cpp");
    Ok(ModelPaths {
        binary: root.join("build/bin/whisper-cli"),
        model: root.join("models/ggml-base.en.bin"),
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
}
