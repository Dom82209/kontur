//! Tauri-граница этапа 5: семестр и точные сроки ДЗ. В vault ничего не пишется.

use tauri::State;

use crate::{
    app_state::AppState,
    dto::HomeworkDto,
    error::AppError,
    semester::{SemesterDto, SemesterInput, SemesterSaveResultDto, SemesterService},
};

#[tauri::command]
pub fn get_semester(state: State<'_, AppState>) -> Result<Option<SemesterDto>, AppError> {
    SemesterService::new(state.database()).get(time::OffsetDateTime::now_utc())
}

/// `expected_revision = null` — семестр ещё не был настроен.
#[tauri::command]
pub fn set_semester(
    state: State<'_, AppState>,
    input: SemesterInput,
    expected_revision: Option<i64>,
) -> Result<SemesterSaveResultDto, AppError> {
    SemesterService::new(state.database()).set(
        &input,
        expected_revision,
        time::OffsetDateTime::now_utc(),
    )
}

/// Точный срок (RFC 3339 с часовым поясом) или `null` — срок неизвестен.
#[tauri::command]
pub fn set_homework_deadline(
    state: State<'_, AppState>,
    homework_id: String,
    revision: i64,
    deadline: Option<String>,
) -> Result<HomeworkDto, AppError> {
    SemesterService::new(state.database()).set_homework_deadline(
        &homework_id,
        revision,
        deadline.as_deref(),
    )
}
