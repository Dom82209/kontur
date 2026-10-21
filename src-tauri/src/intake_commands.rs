//! Tauri-граница этапа 5: управляемые каталоги, native picker/drop, импорт
//! во «Входящие», чтение итога операции и действия Finder.
//!
//! Системные диалоги и Finder вызываются из Rust через официальные
//! `tauri-plugin-dialog` и `tauri-plugin-opener`; frontend не получает прав на
//! эти плагины. Долгие действия выполняются в `spawn_blocking`, чтобы не
//! блокировать главный поток и доставку событий прогресса.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, DragDropEvent, Manager, Runtime, Window};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    app_state::AppState,
    dto::{OperationDto, OperationKind, OperationResultDto},
    error::{AppError, BackendError, ErrorCode},
    events::{
        emit_bridge_event, names, DragState, DragStatePayload, InboxFileAddedPayload,
        InboxUpdateReason, InboxUpdatedPayload, NativeDropPayload, OperationCompletedPayload,
        OperationFailedPayload, OperationProgressPayload,
    },
    file_service::PersistentPath,
    intake::{
        choose_root, pick_files_for_import, register_native_paths, stored_import_report,
        validate_operation_id, DroppedItemDto, FilePicker, ImportItemInput, InboxImportReport,
        InboxImporter, ManagedRootsDto, RootKind, RootSelectionDto, RootsService,
    },
    repositories::ReadRepository,
};

/// Официальный системный диалог (NSOpenPanel на macOS).
struct DialogPicker<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> FilePicker for DialogPicker<R> {
    fn pick_files(&self, start: Option<&Path>) -> Option<Vec<PathBuf>> {
        let mut dialog = self
            .app
            .dialog()
            .file()
            .set_title("Добавить файлы во «Входящие»");
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        dialog.blocking_pick_files().map(|paths| {
            paths
                .into_iter()
                .filter_map(|path| path.into_path().ok())
                .collect()
        })
    }

    fn pick_folder(&self, title: &str, start: Option<&Path>) -> Option<PathBuf> {
        let mut dialog = self.app.dialog().file().set_title(title);
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        dialog
            .blocking_pick_folder()
            .and_then(|path| path.into_path().ok())
    }
}

async fn blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| AppError::new(ErrorCode::Unknown, "Фоновая задача прервана"))?
}

fn roots_service(state: &AppState) -> RootsService<'_> {
    RootsService::new(state.database(), state.home_dir())
}

#[tauri::command]
pub async fn check_managed_roots<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ManagedRootsDto, AppError> {
    blocking(move || roots_service(&app.state::<AppState>()).status()).await
}

#[tauri::command]
pub async fn choose_managed_root<R: Runtime>(
    app: AppHandle<R>,
    kind: RootKind,
) -> Result<RootSelectionDto, AppError> {
    blocking(move || {
        let picker = DialogPicker { app: app.clone() };
        let state = app.state::<AppState>();
        choose_root(&roots_service(&state), &picker, kind)
    })
    .await
}

#[tauri::command]
pub async fn prepare_inbox_root<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ManagedRootsDto, AppError> {
    blocking(move || roots_service(&app.state::<AppState>()).prepare_inbox()).await
}

#[tauri::command]
pub async fn pick_import_files<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<DroppedItemDto>, AppError> {
    blocking(move || {
        let state = app.state::<AppState>();
        let start = roots_service(&state)
            .config()?
            .downloads_path
            .map(PathBuf::from)
            .filter(|path| path.is_dir());
        let picker = DialogPicker { app: app.clone() };
        Ok(pick_files_for_import(
            &picker,
            state.grants(),
            start.as_deref(),
        ))
    })
    .await
}

#[tauri::command]
pub async fn import_inbox_files<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    items: Vec<ImportItemInput>,
) -> Result<OperationResultDto<InboxImportReport>, AppError> {
    blocking(move || run_import(&app, &operation_id, &items)).await
}

fn run_import<R: Runtime>(
    app: &AppHandle<R>,
    operation_id: &str,
    items: &[ImportItemInput],
) -> Result<OperationResultDto<InboxImportReport>, AppError> {
    validate_operation_id(operation_id)?;
    let state = app.state::<AppState>();
    let _serialized = state.lock_imports()?;
    let roots = roots_service(&state).open_import_roots()?;
    let importer = InboxImporter::new(state.database(), &roots, state.grants());
    let mut progress_delivery_failed = false;
    let outcome = importer.run(operation_id, items, &mut |progress| {
        let delivered = emit_bridge_event(
            app,
            names::OPERATION_PROGRESS,
            OperationProgressPayload {
                operation_id: progress.operation_id,
                kind: OperationKind::Import,
                progress: progress.progress,
                message: progress.message,
            },
        );
        progress_delivery_failed |= delivered.is_err();
    })?;

    if !outcome.report.replayed {
        let mut delivery_failed = progress_delivery_failed;
        let added = outcome.added_files();
        if !added.is_empty() {
            delivery_failed |= emit_bridge_event(
                app,
                names::INBOX_FILE_ADDED,
                InboxFileAddedPayload { files: added },
            )
            .is_err();
        }
        delivery_failed |= emit_bridge_event(
            app,
            names::INBOX_UPDATED,
            InboxUpdatedPayload {
                reason: InboxUpdateReason::Import,
                file_ids: outcome.touched_file_ids(),
            },
        )
        .is_err();
        delivery_failed |= match &outcome.failure {
            Some(error) => emit_bridge_event(
                app,
                names::OPERATION_FAILED,
                OperationFailedPayload {
                    operation: outcome.operation.clone(),
                    error: error.clone(),
                },
            ),
            None => emit_bridge_event(
                app,
                names::OPERATION_COMPLETED,
                OperationCompletedPayload {
                    operation: outcome.operation.clone(),
                },
            ),
        }
        .is_err();
        if delivery_failed {
            return Err(AppError::new(
                ErrorCode::Unknown,
                "Импорт сохранён, но UI-событие не доставлено; обновите «Входящие»",
            ));
        }
    }

    Ok(OperationResultDto {
        operation: outcome.operation,
        value: outcome.report,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecordDto {
    pub operation: OperationDto,
    pub import_report: Option<InboxImportReport>,
}

#[tauri::command]
pub fn get_operation_result(
    state: tauri::State<'_, AppState>,
    operation_id: String,
) -> Result<OperationRecordDto, AppError> {
    let operation = ReadRepository::new(state.database()).operation(&operation_id)?;
    let import_report = stored_import_report(state.database(), &operation_id)?;
    Ok(OperationRecordDto {
        operation,
        import_report,
    })
}

#[tauri::command]
pub async fn open_inbox_file<R: Runtime>(
    app: AppHandle<R>,
    file_id: String,
) -> Result<(), AppError> {
    blocking(move || {
        let path = inbox_file_path(&app, &file_id)?;
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
        if name.as_deref().is_some_and(crate::intake::is_risky_to_open) {
            return Err(BackendError::blocked(
                "Исполняемые файлы и скрипты не открываются из «Контура»; используйте «Показать в Finder»",
            ));
        }
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|_| BackendError::io("Не удалось открыть файл в приложении по умолчанию"))
    })
    .await
}

#[tauri::command]
pub async fn reveal_inbox_file<R: Runtime>(
    app: AppHandle<R>,
    file_id: String,
) -> Result<(), AppError> {
    blocking(move || {
        let path = inbox_file_path(&app, &file_id)?;
        reveal(&app, &path)
    })
    .await
}

#[tauri::command]
pub async fn reveal_in_finder<R: Runtime>(
    app: AppHandle<R>,
    relative_path: String,
) -> Result<(), AppError> {
    blocking(move || {
        let relative = PersistentPath::new(&relative_path)?;
        let state = app.state::<AppState>();
        let path = roots_service(&state).resolve_for_os(&relative, false)?;
        reveal(&app, &path)
    })
    .await
}

fn inbox_file_path<R: Runtime>(app: &AppHandle<R>, file_id: &str) -> Result<PathBuf, AppError> {
    let state = app.state::<AppState>();
    let relative = crate::intake::inbox_relative_path(state.database(), file_id)?;
    let relative = PersistentPath::new(relative)
        .map_err(|_| BackendError::database("Небезопасный путь записи «Входящих»"))?;
    roots_service(&state).resolve_for_os(&relative, true)
}

fn reveal<R: Runtime>(app: &AppHandle<R>, path: &Path) -> Result<(), AppError> {
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|_| BackendError::io("Не удалось показать файл в Finder"))
}

/// Native drag-and-drop окна: пути регистрируются как разрешённые до того, как
/// frontend о них узнает, затем отправляется `intake:native-drop`.
pub fn handle_window_drag_drop<R: Runtime>(window: &Window<R>, event: &DragDropEvent) {
    match event {
        DragDropEvent::Enter { .. } => {
            let _ = emit_bridge_event(
                window.app_handle(),
                names::INTAKE_DRAG_STATE,
                DragStatePayload {
                    state: DragState::Enter,
                },
            );
        }
        DragDropEvent::Leave => {
            let _ = emit_bridge_event(
                window.app_handle(),
                names::INTAKE_DRAG_STATE,
                DragStatePayload {
                    state: DragState::Leave,
                },
            );
        }
        DragDropEvent::Drop { paths, .. } => {
            let state = window.state::<AppState>();
            let items = register_native_paths(state.grants(), paths.clone());
            let _ = emit_bridge_event(
                window.app_handle(),
                names::INTAKE_DRAG_STATE,
                DragStatePayload {
                    state: DragState::Leave,
                },
            );
            let _ = emit_bridge_event(
                window.app_handle(),
                names::INTAKE_NATIVE_DROP,
                NativeDropPayload { items },
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use tauri::{Listener, Manager};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::run_import;
    use crate::{
        app_state::AppState,
        db::Database,
        error::ErrorCode,
        events::names,
        intake::{ImportItemInput, ImportOrigin, RootKind, RootsService},
    };

    fn app_with_vault() -> (
        tauri::App<tauri::test::MockRuntime>,
        TempDir,
        std::path::PathBuf,
    ) {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        fs::create_dir_all(root.join("vault")).unwrap();
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        RootsService::new(&database, None)
            .set_root(RootKind::Vault, &root.join("vault"))
            .unwrap();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(AppState::new(root.join("data"), None, database));
        (app, tmp, root)
    }

    fn record_events(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> Arc<Mutex<Vec<(String, serde_json::Value)>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        for name in [
            names::OPERATION_PROGRESS,
            names::INBOX_FILE_ADDED,
            names::INBOX_UPDATED,
            names::OPERATION_COMPLETED,
            names::OPERATION_FAILED,
        ] {
            let log = Arc::clone(&log);
            app.listen_any(name, move |event| {
                let payload = serde_json::from_str(event.payload()).unwrap();
                log.lock().unwrap().push((name.to_owned(), payload));
            });
        }
        log
    }

    #[test]
    fn import_command_emits_progress_then_inbox_events_then_completion() {
        let (app, _tmp, root) = app_with_vault();
        let source = root.join("Лекция.pdf");
        fs::write(&source, b"pdf").unwrap();
        app.state::<AppState>().grants().grant([source.clone()]);
        let log = record_events(&app);
        let operation_id = Uuid::new_v4().to_string();
        let items = [ImportItemInput {
            source_path: source.to_string_lossy().into_owned(),
            origin: ImportOrigin::Picker,
        }];

        let result = run_import(app.handle(), &operation_id, &items).unwrap();
        assert_eq!(result.value.imported, 1);

        let events = log.lock().unwrap().clone();
        let order: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            order,
            vec![
                names::OPERATION_PROGRESS,
                names::OPERATION_PROGRESS,
                names::INBOX_FILE_ADDED,
                names::INBOX_UPDATED,
                names::OPERATION_COMPLETED,
            ]
        );
        assert_eq!(events[1].1["operationId"], operation_id.as_str());
        assert_eq!(events[1].1["kind"], "import");
        assert_eq!(events[1].1["progress"], 1.0);
        assert_eq!(events[2].1["files"][0]["name"], "Лекция.pdf");
        assert_eq!(events[3].1["reason"], "import");
        assert_eq!(events[4].1["operation"]["status"], "completed");
        let serialized =
            serde_json::to_string(&events.iter().map(|(_, v)| v).collect::<Vec<_>>()).unwrap();
        assert!(
            !serialized.contains(root.to_str().unwrap()),
            "no absolute paths in events"
        );

        // Повтор команды: тот же итог, без новых событий и файловых действий.
        let replay = run_import(app.handle(), &operation_id, &items).unwrap();
        assert!(replay.value.replayed);
        assert_eq!(log.lock().unwrap().len(), events.len());
    }

    #[test]
    fn import_command_emits_failed_when_every_file_fails() {
        let (app, _tmp, root) = app_with_vault();
        let missing = root.join("нет.pdf");
        app.state::<AppState>().grants().grant([missing.clone()]);
        let log = record_events(&app);
        let result = run_import(
            app.handle(),
            &Uuid::new_v4().to_string(),
            &[ImportItemInput {
                source_path: missing.to_string_lossy().into_owned(),
                origin: ImportOrigin::Drop,
            }],
        )
        .unwrap();
        assert_eq!(result.value.failed, 1);
        let events = log.lock().unwrap().clone();
        let (last_name, last_payload) = events.last().unwrap();
        assert_eq!(last_name, names::OPERATION_FAILED);
        assert_eq!(last_payload["error"]["code"], "not_found");
        assert!(events
            .iter()
            .all(|(name, _)| name != names::INBOX_FILE_ADDED));
    }

    #[test]
    fn import_command_without_vault_returns_structured_error_and_no_events() {
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        app.manage(AppState::new(std::env::temp_dir(), None, database));
        let log = record_events(&app);
        let error = run_import(
            app.handle(),
            &Uuid::new_v4().to_string(),
            &[ImportItemInput {
                source_path: "/nonexistent/a.pdf".to_owned(),
                origin: ImportOrigin::Drop,
            }],
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Blocked);
        assert!(log.lock().unwrap().is_empty());
    }
}
