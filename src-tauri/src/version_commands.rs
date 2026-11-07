//! Этап 10: команды родословной, сравнения содержимого версий и плана исправлений.

use tauri::State;

use crate::{
    app_state::AppState,
    dto::HomeworkVersionDto,
    error::AppError,
    versions::{CorrectionPlanDto, CorrectionStatus, VersionComparisonDto, VersionService},
};

fn service<'a>(state: &'a State<'_, AppState>) -> VersionService<'a> {
    VersionService::new(state.database(), state.home_dir())
}

#[tauri::command]
pub fn get_version_lineage(
    state: State<'_, AppState>,
    homework_id: String,
    version_id: String,
) -> Result<Vec<HomeworkVersionDto>, AppError> {
    service(&state).lineage(&homework_id, &version_id)
}

#[tauri::command]
pub fn compare_version_contents(
    state: State<'_, AppState>,
    homework_id: String,
    base_version_id: String,
    target_version_id: String,
) -> Result<VersionComparisonDto, AppError> {
    service(&state).compare(&homework_id, &base_version_id, &target_version_id)
}

#[tauri::command]
pub fn get_version_comparison(
    state: State<'_, AppState>,
    comparison_id: String,
) -> Result<VersionComparisonDto, AppError> {
    service(&state).comparison(&comparison_id)
}

#[tauri::command]
pub fn get_correction_plan(
    state: State<'_, AppState>,
    homework_id: String,
) -> Result<Option<CorrectionPlanDto>, AppError> {
    service(&state).current_plan(&homework_id)
}

#[tauri::command]
pub fn build_correction_plan(
    state: State<'_, AppState>,
    homework_id: String,
) -> Result<CorrectionPlanDto, AppError> {
    service(&state).build_plan(&homework_id)
}

#[tauri::command]
pub fn set_correction_item_status(
    state: State<'_, AppState>,
    item_id: String,
    status: CorrectionStatus,
    resolved_in_version_id: Option<String>,
) -> Result<CorrectionPlanDto, AppError> {
    service(&state).set_item_status(&item_id, status, resolved_in_version_id.as_deref())
}
