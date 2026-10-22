mod app_state;
mod commands;
pub mod db;
pub mod domain;
pub mod dto;
pub mod error;
pub mod events;
pub mod extraction;
mod extraction_commands;
pub mod file_service;
pub mod intake;
mod intake_commands;
#[cfg(test)]
mod ipc_commands;
pub mod lms;
mod lms_commands;
pub mod mutation_service;
pub mod query_service;
pub mod repositories;
pub mod workflow;
mod workflow_commands;

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
            // Watcher загрузок запускается только если пользователь включил его ранее.
            lms_commands::resume_watcher(app.handle());
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
            intake_commands::reveal_in_finder,
            lms_commands::get_downloads_watcher,
            lms_commands::set_downloads_watcher,
            lms_commands::dismiss_download_candidate,
            lms_commands::pick_cu_export,
            lms_commands::preview_cu_export,
            lms_commands::import_cu_export,
            lms_commands::set_subject_course_url,
            lms_commands::set_homework_assignment_url,
            lms_commands::open_course,
            lms_commands::open_assignment,
            extraction_commands::list_material_extractions,
            extraction_commands::get_material_extraction,
            extraction_commands::analyze_material,
            extraction_commands::analyze_materials,
            extraction_commands::cancel_extraction,
            extraction_commands::clear_extraction_cache,
            extraction_commands::get_extraction_settings,
            extraction_commands::set_extraction_settings,
            extraction_commands::get_corpus_selection,
            extraction_commands::set_corpus_selection,
            extraction_commands::preview_corpus,
            workflow_commands::get_workflow,
            workflow_commands::get_dashboard,
            workflow_commands::get_iterations,
            workflow_commands::compare_versions,
            workflow_commands::get_homework_candidates,
            workflow_commands::create_source_bundle,
            workflow_commands::start_workflow_run,
            workflow_commands::cancel_workflow_run,
            workflow_commands::pin_workflow_run,
            workflow_commands::save_manual_analysis,
            workflow_commands::save_manual_review,
            workflow_commands::create_revision_prompt,
            workflow_commands::update_prompt_draft,
            workflow_commands::commit_prompt,
            workflow_commands::hand_off_prompt,
            workflow_commands::create_homework_version,
            workflow_commands::select_homework_version,
            workflow_commands::add_teacher_comments,
            workflow_commands::add_teacher_requirement,
            workflow_commands::resolve_teacher_requirement,
            workflow_commands::confirm_style_divergence,
            workflow_commands::mark_ready_to_submit,
            workflow_commands::transition_workflow
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
