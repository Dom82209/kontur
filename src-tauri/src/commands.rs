use tauri::{AppHandle, Runtime, State};

use crate::{
    app_state::AppState,
    domain::BackendHealth,
    dto::{
        AutomationRulesDto, ClassificationPatchDto, HomeworkDto, InboxFileDto, OperationDto,
        OperationResultDto, SubjectDto, WeekDto,
    },
    error::{AppError, ErrorCode},
    events::{emit_bridge_event, names, OperationFailedPayload},
    mutation_service::{MutationEvent, MutationFailure, MutationService},
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

#[tauri::command]
pub fn update_automation_rules<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    rules: AutomationRulesDto,
) -> Result<OperationResultDto<AutomationRulesDto>, AppError> {
    match MutationService::new(state.database()).update_automation_rules(rules) {
        Ok(success) => {
            emit_success_events(&app, &success.events)?;
            Ok(OperationResultDto {
                operation: success.operation,
                value: success.value,
            })
        }
        Err(failure) => Err(emit_failure_or_original(&app, failure)),
    }
}

#[tauri::command]
pub fn update_classification<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    file_id: String,
    patch: ClassificationPatchDto,
) -> Result<InboxFileDto, AppError> {
    match MutationService::new(state.database()).update_classification(&file_id, patch) {
        Ok(success) => {
            emit_success_events(&app, &success.events)?;
            Ok(success.value)
        }
        Err(failure) => Err(emit_failure_or_original(&app, failure)),
    }
}

fn emit_success_events<R: Runtime>(
    app: &AppHandle<R>,
    events: &[MutationEvent],
) -> Result<(), AppError> {
    let mut delivery_failed = false;
    for event in events {
        let result = match event {
            MutationEvent::InboxClassificationUpdated(payload) => {
                emit_bridge_event(app, names::INBOX_CLASSIFICATION_UPDATED, payload.clone())
            }
            MutationEvent::OperationCompleted(payload) => {
                emit_bridge_event(app, names::OPERATION_COMPLETED, payload.clone())
            }
        };
        delivery_failed |= result.is_err();
    }
    if delivery_failed {
        Err(AppError::new(
            ErrorCode::Unknown,
            "Данные сохранены, но UI-событие не доставлено; обновите данные повторным запросом",
        ))
    } else {
        Ok(())
    }
}

fn emit_failure_or_original<R: Runtime>(app: &AppHandle<R>, failure: MutationFailure) -> AppError {
    let Some(operation) = failure.operation else {
        return failure.error;
    };
    let delivery = emit_bridge_event(
        app,
        names::OPERATION_FAILED,
        OperationFailedPayload {
            operation: *operation,
            error: failure.error.clone(),
        },
    );
    if delivery.is_err() {
        AppError::new(
            failure.error.code,
            "Операция не выполнена; UI-событие об ошибке не доставлено",
        )
    } else {
        failure.error
    }
}
