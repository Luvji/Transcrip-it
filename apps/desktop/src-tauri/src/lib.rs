pub mod database;
pub mod engines;

use database::{CreateMeetingInput, Database, DatabaseStatus, MeetingRecord};
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
    database: tauri::State<'_, Database>,
    meeting_id: String,
) -> Result<bool, String> {
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
            delete_meeting
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
