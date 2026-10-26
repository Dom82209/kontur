use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_dialog::DialogExt;

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticExportDto {
    cancelled: bool,
    file_name: Option<String>,
}

fn diagnostic_timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

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

fn diagnostic_report(state: &AppState) -> Result<Value, AppError> {
    let health = QueryService::new(state.database()).health()?;
    let roots = crate::intake::RootsService::new(state.database(), state.home_dir()).status()?;
    let (counts, recent_operations, ai_history) = state.database().with_connection(|connection| {
        let mut counts = serde_json::Map::new();
        for table in [
            "subjects",
            "weeks",
            "homeworks",
            "homework_versions",
            "inbox_files",
            "operations",
            "teacher_comments",
            "teacher_requirements",
            "submission_packages",
            "submission_receipts",
            "ai_jobs",
        ] {
            let count: i64 = connection.query_row(
                &format!("SELECT count(*) FROM {table}"),
                [],
                |row| row.get(0),
            )?;
            counts.insert(table.to_owned(), json!(count));
        }

        let mut operations_query = connection.prepare(
            "SELECT kind, status, started_at, completed_at FROM operations ORDER BY started_at DESC, id DESC LIMIT 20",
        )?;
        let recent_operations = operations_query
            .query_map([], |row| {
                Ok(json!({
                    "kind": row.get::<_, String>(0)?,
                    "status": row.get::<_, String>(1)?,
                    "startedAt": row.get::<_, String>(2)?,
                    "completedAt": row.get::<_, Option<String>>(3)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut ai_query = connection.prepare(
            "SELECT provider, transport, status, created_at, finished_at FROM ai_jobs ORDER BY created_at DESC, id DESC LIMIT 20",
        )?;
        let ai_history = ai_query
            .query_map([], |row| {
                Ok(json!({
                    "provider": row.get::<_, String>(0)?,
                    "transport": row.get::<_, String>(1)?,
                    "status": row.get::<_, String>(2)?,
                    "createdAt": row.get::<_, String>(3)?,
                    "finishedAt": row.get::<_, Option<String>>(4)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((Value::Object(counts), recent_operations, ai_history))
    })?;

    Ok(json!({
        "formatVersion": 1,
        "generatedAt": diagnostic_timestamp(),
        "application": {
            "version": health.app_version,
            "platform": health.platform,
            "schemaVersion": health.schema_version,
            "databaseReady": health.ready && health.database_exists,
        },
        "managedRoots": {
            "vault": roots.vault.state,
            "inbox": roots.inbox.state,
            "backup": roots.backup.state,
            "downloads": roots.downloads.state,
            "importReady": roots.import_ready,
        },
        "recordCounts": counts,
        "recentOperations": recent_operations,
        "aiHistory": ai_history,
        "privacy": {
            "documentContentsIncluded": false,
            "absolutePathsIncluded": false,
            "credentialsIncluded": false,
            "cookiesIncluded": false,
            "tokensIncluded": false,
        }
    }))
}

#[tauri::command]
pub fn export_diagnostic_report<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<DiagnosticExportDto, AppError> {
    let Some(selected) = app
        .dialog()
        .file()
        .set_title("Экспорт диагностики «Контура»")
        .set_file_name("kontur-diagnostic.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
    else {
        return Ok(DiagnosticExportDto {
            cancelled: true,
            file_name: None,
        });
    };
    let path = selected
        .into_path()
        .map_err(|_| crate::error::BackendError::blocked("Выбранный путь недоступен"))?;
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(crate::error::BackendError::blocked(
            "Диагностический отчёт нельзя записать по символической ссылке",
        ));
    }
    let bytes = serde_json::to_vec_pretty(&diagnostic_report(&state)?)
        .map_err(|error| crate::error::BackendError::database(error.to_string()))?;
    std::fs::write(&path, bytes).map_err(|_| {
        crate::error::BackendError::io("Не удалось сохранить диагностический отчёт")
    })?;
    Ok(DiagnosticExportDto {
        cancelled: false,
        file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_report_excludes_contents_paths_and_secrets() {
        let temp = tempfile::TempDir::new().unwrap();
        let database = crate::db::Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO operations(id, kind, status, title, detail, paths_json) VALUES ('operation-1', 'route', 'completed', 'Личное название', 'SECRET-DOCUMENT-TEXT', '[\"Предметы/Секрет/файл.pdf\"]')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let state = AppState::new(temp.path().to_path_buf(), None, database);
        let report = diagnostic_report(&state).unwrap();
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("SECRET-DOCUMENT-TEXT"));
        assert!(!encoded.contains("Личное название"));
        assert!(!encoded.contains("Секрет"));
        assert!(!encoded.contains(temp.path().to_string_lossy().as_ref()));
        assert_eq!(report["privacy"]["credentialsIncluded"], false);
        assert_eq!(report["privacy"]["documentContentsIncluded"], false);
    }
}
