mod child_process;
pub mod database;
pub mod engines;
mod recording;
mod transcription;

use database::{CreateMeetingInput, Database, DatabaseStatus, MeetingRecord};
use recording::Recorder;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::fd::AsRawFd,
    path::Path,
};
use tauri::Manager;
use transcription::Transcriber;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDiagnostics {
    storage_bytes: u64,
    recording_bytes: u64,
    database_bytes: u64,
    #[serde(flatten)]
    database: database::DatabaseDiagnostics,
}

fn directory_size(path: &Path) -> io::Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut bytes = 0_u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        bytes = bytes.saturating_add(if metadata.is_dir() {
            directory_size(&entry.path())?
        } else {
            metadata.len()
        });
    }
    Ok(bytes)
}

#[derive(Debug)]
struct SingleInstanceGuard {
    _file: File,
}

fn acquire_single_instance(path: &Path) -> io::Result<SingleInstanceGuard> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::WouldBlock {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Transcrip-it is already running",
            ));
        }
        return Err(error);
    }
    Ok(SingleInstanceGuard { _file: file })
}

#[tauri::command]
fn database_status(database: tauri::State<'_, Database>) -> Result<DatabaseStatus, String> {
    database.status().map_err(|error| error.to_string())
}

#[tauri::command]
fn workspace_diagnostics(
    app: tauri::AppHandle,
    database: tauri::State<'_, Database>,
) -> Result<WorkspaceDiagnostics, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let database_path = app_data.join("transcrip-it.sqlite3");
    Ok(WorkspaceDiagnostics {
        storage_bytes: directory_size(&app_data).map_err(|error| error.to_string())?,
        recording_bytes: directory_size(&app_data.join("recordings"))
            .map_err(|error| error.to_string())?,
        database_bytes: fs::metadata(database_path).map_or(0, |metadata| metadata.len()),
        database: database.diagnostics().map_err(|error| error.to_string())?,
    })
}

#[tauri::command]
fn create_meeting(
    database: tauri::State<'_, Database>,
    input: CreateMeetingInput,
) -> Result<MeetingRecord, String> {
    database
        .create_meeting(&input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_meetings(
    database: tauri::State<'_, Database>,
    include_archived: bool,
) -> Result<Vec<MeetingRecord>, String> {
    database
        .list_meetings(include_archived)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn rename_meeting(
    database: tauri::State<'_, Database>,
    meeting_id: String,
    title: String,
) -> Result<(), String> {
    database
        .rename_meeting(&meeting_id, &title)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn replace_meeting_tags(
    database: tauri::State<'_, Database>,
    meeting_id: String,
    tags: Vec<String>,
) -> Result<Vec<String>, String> {
    database
        .replace_meeting_tags(&meeting_id, &tags)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn archive_meeting(
    database: tauri::State<'_, Database>,
    meeting_id: String,
    idempotency_key: String,
) -> Result<(), String> {
    database
        .archive_meeting(&meeting_id, &idempotency_key)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn reopen_meeting(
    database: tauri::State<'_, Database>,
    meeting_id: String,
    idempotency_key: String,
) -> Result<(), String> {
    database
        .reopen_meeting(&meeting_id, &idempotency_key)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_meeting(
    app: tauri::AppHandle,
    database: tauri::State<'_, Database>,
    recorder: tauri::State<'_, Recorder>,
    meeting_id: String,
) -> Result<bool, String> {
    recorder.stop_playback_for(Some(&meeting_id))?;
    if let Some(stored_path) = database
        .meeting_recording_path(&meeting_id)
        .map_err(|error| error.to_string())?
    {
        let recording_root = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("recordings");
        let stored_path = std::path::PathBuf::from(stored_path);
        if !stored_path.starts_with(&recording_root) {
            return Err("Refusing to delete a recording outside application storage.".to_owned());
        }
        if stored_path.exists() {
            std::fs::remove_dir_all(&stored_path)
                .map_err(|error| format!("Could not remove recording files: {error}"))?;
        }
    }
    database
        .delete_meeting(&meeting_id)
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let result = tauri::Builder::default()
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            fs::create_dir_all(&app_data)?;
            let instance = match acquire_single_instance(&app_data.join("transcrip-it.lock")) {
                Ok(instance) => instance,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => std::process::exit(0),
                Err(error) => return Err(error.into()),
            };
            app.manage(instance);
            let database_path = app_data.join("transcrip-it.sqlite3");
            let database = Database::open(&database_path)?;
            recording::recover_completed_recording_metadata(&database)
                .map_err(std::io::Error::other)?;
            database.recover_interrupted_meetings()?;
            app.manage(database);
            app.manage(Recorder::new());
            app.manage(Transcriber::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            database_status,
            workspace_diagnostics,
            create_meeting,
            list_meetings,
            rename_meeting,
            replace_meeting_tags,
            archive_meeting,
            reopen_meeting,
            delete_meeting,
            recording::start_recording,
            recording::audio_devices,
            recording::recording_status,
            recording::recover_interrupted_recordings,
            recording::pause_recording,
            recording::resume_recording,
            recording::stop_recording,
            recording::play_recording_track,
            recording::playback_status,
            recording::stop_recording_playback,
            transcription::transcription_model_status,
            transcription::install_transcription_model,
            transcription::list_transcript,
            transcription::delete_transcript,
            transcription::rename_transcript_speaker,
            transcription::live_transcript_preview,
            transcription::search_transcripts,
            transcription::correct_transcript_segment,
            transcription::export_transcript,
            transcription::transcribe_meeting
        ])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        if error
            .to_string()
            .contains("Transcrip-it is already running")
        {
            return;
        }
        panic!("error while running tauri application: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn rejects_a_second_desktop_instance_lock() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "transcrip-it-instance-{}-{unique}.lock",
            std::process::id()
        ));
        let first = acquire_single_instance(&path).unwrap();
        let second = acquire_single_instance(&path).unwrap_err();
        assert_eq!(second.kind(), io::ErrorKind::AlreadyExists);
        drop(first);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn directory_size_counts_files_and_ignores_symlinks() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("transcrip-it-size-{unique}"));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("first"), [0_u8; 7]).unwrap();
        fs::write(root.join("nested/second"), [0_u8; 11]).unwrap();
        assert_eq!(directory_size(&root).unwrap(), 18);
        fs::remove_dir_all(root).unwrap();
    }
}
