/// Все IPC-команды приложения. Для каждой tauri-build генерирует права
/// `allow-<command>`/`deny-<command>`; окно получает только перечисленные в
/// `capabilities/default.json`. Команда, не попавшая в capability, недоступна frontend.
pub const COMMANDS: &[&str] = &[
    "backend_health",
    "get_subjects",
    "get_weeks",
    "get_week",
    "get_homework_list",
    "get_homework",
    "get_inbox_files",
    "get_automation_rules",
    "get_activity_log",
    "update_automation_rules",
    "update_classification",
    "check_managed_roots",
    "choose_managed_root",
    "prepare_inbox_root",
    "pick_import_files",
    "import_inbox_files",
    "get_operation_result",
    "open_inbox_file",
    "reveal_inbox_file",
    "reveal_in_finder",
];
