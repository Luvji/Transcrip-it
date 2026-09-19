mod database;

use database::{Database, DatabaseStatus};
use tauri::Manager;

#[tauri::command]
fn database_status(database: tauri::State<'_, Database>) -> Result<DatabaseStatus, String> {
    database.status().map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let database_path = app.path().app_data_dir()?.join("transcrip-it.sqlite3");
            app.manage(Database::open(&database_path)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![database_status])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
