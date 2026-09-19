pub mod database;
pub mod engines;
mod recording;

use database::{CreateMeetingInput, Database, DatabaseStatus, MeetingRecord};
use recording::Recorder;
use tauri::Manager;

#[tauri::command]
fn database_status(database: tauri::State<'_, Database>) -> Result<DatabaseStatus, String> {
    database.status().map_err(|error| error.to_string())
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
    meeting_id: String,
) -> Result<bool, String> {
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
    tauri::Builder::default()
        .setup(|app| {
            let database_path = app.path().app_data_dir()?.join("transcrip-it.sqlite3");
            app.manage(Database::open(&database_path)?);
            app.manage(Recorder::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            database_status,
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
            recording::pause_recording,
            recording::resume_recording,
            recording::stop_recording,
            recording::play_recording_track
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
