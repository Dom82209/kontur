use tauri::State;

use crate::{
    app_state::AppState,
    domain::BackendHealth,
    dto::{AutomationRulesDto, HomeworkDto, InboxFileDto, OperationDto, SubjectDto, WeekDto},
    error::AppError,
    query_service::QueryService,
};

#[tauri::command]
pub fn backend_health(state: State<'_, AppState>) -> Result<BackendHealth, AppError> {
    let _safe_state_probe = (
        state.app_data_dir().is_absolute(),
        state.events().registered_services(),
    );
    QueryService::new(state.database()).health()
}

#[tauri::command]
pub fn get_subjects(state: State<'_, AppState>) -> Result<Vec<SubjectDto>, AppError> {
    QueryService::new(state.database()).subjects()
}

#[tauri::command]
pub fn get_weeks(state: State<'_, AppState>, subject_id: String) -> Result<Vec<WeekDto>, AppError> {
    QueryService::new(state.database()).weeks(&subject_id)
}

#[tauri::command]
pub fn get_week(
    state: State<'_, AppState>,
    subject_id: String,
    week_number: i64,
) -> Result<WeekDto, AppError> {
    QueryService::new(state.database()).week(&subject_id, week_number)
}

#[tauri::command]
pub fn get_homework_list(state: State<'_, AppState>) -> Result<Vec<HomeworkDto>, AppError> {
    QueryService::new(state.database()).homework_list()
}

#[tauri::command]
pub fn get_homework(
    state: State<'_, AppState>,
    homework_id: String,
) -> Result<HomeworkDto, AppError> {
    QueryService::new(state.database()).homework(&homework_id)
}

#[tauri::command]
pub fn get_inbox_files(state: State<'_, AppState>) -> Result<Vec<InboxFileDto>, AppError> {
    QueryService::new(state.database()).inbox_files()
}

#[tauri::command]
pub fn get_automation_rules(state: State<'_, AppState>) -> Result<AutomationRulesDto, AppError> {
    QueryService::new(state.database()).automation_rules()
}

#[tauri::command]
pub fn get_activity_log(state: State<'_, AppState>) -> Result<Vec<OperationDto>, AppError> {
    QueryService::new(state.database()).activity_log()
}
