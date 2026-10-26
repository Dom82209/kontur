use tauri::State;

use crate::{
    app_state::AppState,
    dto::{InboxFileDto, OperationResultDto, RouteDestinationDto, RoutePreviewDto, WeekDto},
    error::AppError,
    routing::RoutingService,
};

fn service(state: &AppState) -> RoutingService<'_> {
    RoutingService::new(state.database(), state.home_dir())
}

#[tauri::command]
pub fn create_week(
    state: State<'_, AppState>,
    subject_id: String,
    week_number: i64,
) -> Result<OperationResultDto<WeekDto>, AppError> {
    service(&state).create_week(&subject_id, week_number)
}

#[tauri::command]
pub fn classify_files(
    state: State<'_, AppState>,
    file_ids: Vec<String>,
) -> Result<OperationResultDto<Vec<InboxFileDto>>, AppError> {
    service(&state).classify_files(&file_ids)
}

#[tauri::command]
pub fn preview_route(
    state: State<'_, AppState>,
    file_id: String,
    destination: RouteDestinationDto,
) -> Result<RoutePreviewDto, AppError> {
    service(&state).preview(&file_id, &destination)
}

#[tauri::command]
pub fn route_files(
    state: State<'_, AppState>,
    file_ids: Vec<String>,
    destination: Option<RouteDestinationDto>,
) -> Result<OperationResultDto<Vec<RoutePreviewDto>>, AppError> {
    service(&state).route(&file_ids, destination.as_ref())
}

#[tauri::command]
pub fn undo_operation(
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<OperationResultDto<()>, AppError> {
    service(&state).undo(&operation_id)
}
