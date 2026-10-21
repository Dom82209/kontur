mod app_state;
mod commands;
pub mod db;
pub mod domain;
pub mod dto;
pub mod error;
pub mod events;
pub mod file_service;
pub mod intake;
mod intake_commands;
#[cfg(test)]
mod ipc_commands;
pub mod mutation_service;
pub mod query_service;
pub mod repositories;

use app_state::AppState;
use db::Database;
use tauri::{Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Плагины вызываются только из Rust-команд; frontend-права на них не выдаются.
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let home_dir = app.path().home_dir().ok();

            let database = Database::open_file(app_data_dir.join("kontur.sqlite3"))?;
            database.migrate()?;
            app.manage(AppState::new(app_data_dir, home_dir, database));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(drag) = event {
                intake_commands::handle_window_drag_drop(window, drag);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::backend_health,
            commands::get_subjects,
            commands::get_weeks,
            commands::get_week,
            commands::get_homework_list,
            commands::get_homework,
            commands::get_inbox_files,
            commands::get_automation_rules,
            commands::get_activity_log,
            commands::update_automation_rules,
            commands::update_classification,
            intake_commands::check_managed_roots,
            intake_commands::choose_managed_root,
            intake_commands::prepare_inbox_root,
            intake_commands::pick_import_files,
            intake_commands::import_inbox_files,
            intake_commands::get_operation_result,
            intake_commands::open_inbox_file,
            intake_commands::reveal_inbox_file,
            intake_commands::reveal_in_finder
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kontur");
}

#[cfg(test)]
mod capability_tests {
    use super::ipc_commands::COMMANDS;

    #[test]
    fn capability_allows_exactly_registered_commands_and_no_plugins() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let permissions: Vec<String> = capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        let mut expected: Vec<String> = vec![
            "core:event:default".to_owned(),
            "core:window:allow-start-dragging".to_owned(),
        ];
        expected.extend(
            COMMANDS
                .iter()
                .map(|command| format!("allow-{}", command.replace('_', "-"))),
        );
        assert_eq!(permissions, expected);
        assert!(permissions
            .iter()
            .all(|permission| !permission.starts_with("dialog:")
                && !permission.starts_with("opener:")
                && !permission.starts_with("fs:")));
    }
}
