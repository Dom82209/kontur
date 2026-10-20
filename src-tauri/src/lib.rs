mod app_state;
mod commands;
pub mod db;
pub mod domain;
pub mod error;
mod events;
pub mod repositories;

use app_state::AppState;
use db::Database;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;

            let database = Database::open_file(app_data_dir.join("kontur.sqlite3"))?;
            database.migrate()?;
            app.manage(AppState::new(app_data_dir, database));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::backend_health])
        .run(tauri::generate_context!())
        .expect("error while running Kontur");
}
