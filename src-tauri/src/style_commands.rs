//! Этап 9: команды профиля личного стиля. Источники — только явный выбор пользователя.

use tauri::State;

use crate::{
    app_state::AppState,
    error::AppError,
    style::{
        StyleCheckDto, StyleProfileRevisionDto, StyleProfileStateDto, StyleService, StyleSourceKind,
    },
};

fn service<'a>(state: &'a State<'_, AppState>) -> StyleService<'a> {
    StyleService::new(state.database(), state.home_dir())
}

#[tauri::command]
pub fn get_style_profile_state(
    state: State<'_, AppState>,
) -> Result<StyleProfileStateDto, AppError> {
    service(&state).state()
}

#[tauri::command]
pub fn get_style_profile_revision(
    state: State<'_, AppState>,
    revision_id: String,
) -> Result<Option<StyleProfileRevisionDto>, AppError> {
    service(&state).revision(&revision_id)
}

#[tauri::command]
pub fn add_style_source(
    state: State<'_, AppState>,
    relative_path: String,
    kind: StyleSourceKind,
    language: String,
) -> Result<StyleProfileStateDto, AppError> {
    service(&state).add_source(&relative_path, kind, &language)
}

#[tauri::command]
pub fn remove_style_source(
    state: State<'_, AppState>,
    source_id: String,
) -> Result<StyleProfileStateDto, AppError> {
    service(&state).remove_source(&source_id)
}

#[tauri::command]
pub fn refresh_style_sources(state: State<'_, AppState>) -> Result<StyleProfileStateDto, AppError> {
    service(&state).refresh_sources()
}

#[tauri::command]
pub fn rebuild_style_profile(
    state: State<'_, AppState>,
    language: String,
) -> Result<StyleProfileStateDto, AppError> {
    service(&state).rebuild(&language)
}

#[tauri::command]
pub fn run_style_check(
    state: State<'_, AppState>,
    homework_id: String,
) -> Result<StyleCheckDto, AppError> {
    service(&state).check(&homework_id)
}

#[tauri::command]
pub fn confirm_style_check(
    state: State<'_, AppState>,
    check_id: String,
    note: String,
) -> Result<StyleCheckDto, AppError> {
    service(&state).confirm(&check_id, &note)
}
