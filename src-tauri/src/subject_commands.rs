//! Tauri-граница этапа 3: управление предметами. Ни одна команда не пишет в vault.

use tauri::State;

use crate::{
    app_state::AppState,
    dto::{HomeworkDto, SubjectDto},
    error::AppError,
    homework_registry::{HomeworkRegistry, NewHomeworkInput},
    subjects::{SubjectInput, SubjectMergePreviewDto, SubjectMergeResultDto, SubjectService},
};

#[tauri::command]
pub fn create_subject(
    state: State<'_, AppState>,
    input: SubjectInput,
) -> Result<SubjectDto, AppError> {
    SubjectService::new(state.database()).create(&input)
}

#[tauri::command]
pub fn update_subject(
    state: State<'_, AppState>,
    subject_id: String,
    revision: i64,
    input: SubjectInput,
) -> Result<SubjectDto, AppError> {
    SubjectService::new(state.database()).update(&subject_id, revision, &input)
}

#[tauri::command]
pub fn set_subject_archived(
    state: State<'_, AppState>,
    subject_id: String,
    revision: i64,
    archived: bool,
) -> Result<SubjectDto, AppError> {
    SubjectService::new(state.database()).set_archived(&subject_id, revision, archived)
}

#[tauri::command]
pub fn preview_subject_merge(
    state: State<'_, AppState>,
    source_id: String,
    target_id: String,
) -> Result<SubjectMergePreviewDto, AppError> {
    SubjectService::new(state.database()).preview_merge(&source_id, &target_id)
}

/// Слияние меняет принадлежность недель, поэтому идёт под общим замком мутаций.
#[tauri::command]
pub fn merge_subjects(
    state: State<'_, AppState>,
    source_id: String,
    target_id: String,
    fingerprint: String,
) -> Result<SubjectMergeResultDto, AppError> {
    let _guard = state.lock_file_mutations()?;
    SubjectService::new(state.database()).merge(&source_id, &target_id, &fingerprint)
}

/// Этап 4: ещё одно ДЗ недели (только в базе, без записи в vault).
#[tauri::command]
pub fn create_homework(
    state: State<'_, AppState>,
    input: NewHomeworkInput,
) -> Result<HomeworkDto, AppError> {
    HomeworkRegistry::new(state.database()).create(&input)
}
