//! Tauri-граница этапа 7: локальное извлечение текста и структуры, подборка
//! материалов и предпросмотр корпуса. Codex/Claude не вызываются; frontend
//! адресует материалы непрозрачными ссылками и не получает путей.

use std::sync::{atomic::AtomicBool, Arc};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_state::AppState,
    dto::{OperationKind, OperationResultDto},
    error::{AppError, ErrorCode},
    events::{
        emit_bridge_event, names, ExtractionProgressPayload, ExtractionUpdateReason,
        ExtractionUpdatedPayload, OperationCompletedPayload, OperationFailedPayload,
        OperationProgressPayload,
    },
    extraction::{
        CacheClearedDto, CorpusPreviewDto, CorpusSelectionDto, ExtractionBatchReport,
        ExtractionService, ExtractionSettingsDto, MaterialExtractionDto, MaterialExtractionItemDto,
        MaterialRef,
    },
};

async fn blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| AppError::new(ErrorCode::Unknown, "Фоновая задача прервана"))?
}

fn service(state: &AppState) -> ExtractionService<'_> {
    ExtractionService::new(
        state.database(),
        state.home_dir(),
        state.extraction_limits(),
        state.ocr_engine(),
    )
}

fn emit_updated<R: Runtime>(
    app: &AppHandle<R>,
    reason: ExtractionUpdateReason,
    materials: Vec<MaterialRef>,
) -> Result<(), AppError> {
    emit_bridge_event(
        app,
        names::EXTRACTION_UPDATED,
        ExtractionUpdatedPayload { reason, materials },
    )
}

#[tauri::command]
pub async fn list_material_extractions<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<MaterialExtractionItemDto>, AppError> {
    blocking(move || service(&app.state::<AppState>()).list()).await
}

#[tauri::command]
pub async fn get_material_extraction<R: Runtime>(
    app: AppHandle<R>,
    material: MaterialRef,
) -> Result<MaterialExtractionDto, AppError> {
    blocking(move || service(&app.state::<AppState>()).get(&material)).await
}

#[tauri::command]
pub async fn analyze_material<R: Runtime>(
    app: AppHandle<R>,
    material: MaterialRef,
    force: bool,
) -> Result<MaterialExtractionDto, AppError> {
    blocking(move || analyze_one(&app, &material, force)).await
}

pub fn analyze_one<R: Runtime>(
    app: &AppHandle<R>,
    material: &MaterialRef,
    force: bool,
) -> Result<MaterialExtractionDto, AppError> {
    let state = app.state::<AppState>();
    let result = service(&state).analyze(material, force, Arc::new(AtomicBool::new(false)))?;
    if emit_updated(
        app,
        ExtractionUpdateReason::Analyzed,
        vec![material.clone()],
    )
    .is_err()
    {
        return Err(AppError::new(
            ErrorCode::Unknown,
            "Текст извлечён и сохранён, но UI-событие не доставлено; обновите список материалов",
        ));
    }
    Ok(result)
}

#[tauri::command]
pub async fn analyze_materials<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    materials: Vec<MaterialRef>,
    force: bool,
) -> Result<OperationResultDto<ExtractionBatchReport>, AppError> {
    blocking(move || run_batch(&app, &operation_id, &materials, force)).await
}

pub fn run_batch<R: Runtime>(
    app: &AppHandle<R>,
    operation_id: &str,
    materials: &[MaterialRef],
    force: bool,
) -> Result<OperationResultDto<ExtractionBatchReport>, AppError> {
    crate::intake::validate_operation_id(operation_id)?;
    let state = app.state::<AppState>();
    let cancel = state.extraction_jobs().register(operation_id)?;
    let mut delivery_failed = false;
    let outcome =
        service(&state).run_batch(operation_id, materials, force, cancel, &mut |progress| {
            let fraction = if progress.total == 0 {
                0.0
            } else {
                progress.completed as f64 / progress.total as f64
            };
            let (material, name) = match progress.current {
                Some((material, name)) => (Some(material), Some(name)),
                None => (None, None),
            };
            delivery_failed |= emit_bridge_event(
                app,
                names::EXTRACTION_PROGRESS,
                ExtractionProgressPayload {
                    operation_id: operation_id.to_owned(),
                    completed: progress.completed,
                    total: progress.total,
                    material,
                    name: name.clone(),
                    outcome: progress.outcome,
                },
            )
            .is_err();
            delivery_failed |= emit_bridge_event(
                app,
                names::OPERATION_PROGRESS,
                OperationProgressPayload {
                    operation_id: operation_id.to_owned(),
                    kind: OperationKind::Extract,
                    progress: fraction,
                    message: name.unwrap_or_else(|| "Извлечение текста".to_owned()),
                },
            )
            .is_err();
        });
    state.extraction_jobs().finish(operation_id);
    let outcome = outcome?;

    // Повтор той же команды: сохранённый итог без событий.
    if outcome.report.replayed {
        return Ok(OperationResultDto {
            operation: outcome.operation,
            value: outcome.report,
        });
    }
    delivery_failed |= emit_updated(
        app,
        ExtractionUpdateReason::Analyzed,
        outcome.touched.clone(),
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
            "Извлечение сохранено, но UI-событие не доставлено; обновите список материалов",
        ));
    }
    Ok(OperationResultDto {
        operation: outcome.operation,
        value: outcome.report,
    })
}

/// Просит остановить пакет. `false` — пакет уже завершён или не найден.
#[tauri::command]
pub fn cancel_extraction(
    state: tauri::State<'_, AppState>,
    operation_id: String,
) -> Result<bool, AppError> {
    Ok(state.extraction_jobs().cancel(&operation_id))
}

#[tauri::command]
pub async fn clear_extraction_cache<R: Runtime>(
    app: AppHandle<R>,
) -> Result<CacheClearedDto, AppError> {
    blocking(move || {
        let removed = service(&app.state::<AppState>()).clear_cache()?;
        let _ = emit_updated(&app, ExtractionUpdateReason::Cleared, Vec::new());
        Ok(CacheClearedDto { removed })
    })
    .await
}

#[tauri::command]
pub async fn get_extraction_settings<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ExtractionSettingsDto, AppError> {
    blocking(move || service(&app.state::<AppState>()).settings()).await
}

#[tauri::command]
pub async fn set_extraction_settings<R: Runtime>(
    app: AppHandle<R>,
    ocr_enabled: bool,
) -> Result<ExtractionSettingsDto, AppError> {
    blocking(move || service(&app.state::<AppState>()).set_ocr_enabled(ocr_enabled)).await
}

#[tauri::command]
pub async fn get_corpus_selection<R: Runtime>(
    app: AppHandle<R>,
    homework_id: Option<String>,
) -> Result<CorpusSelectionDto, AppError> {
    blocking(move || service(&app.state::<AppState>()).selection(homework_id.as_deref())).await
}

#[tauri::command]
pub async fn set_corpus_selection<R: Runtime>(
    app: AppHandle<R>,
    homework_id: Option<String>,
    materials: Vec<MaterialRef>,
) -> Result<CorpusSelectionDto, AppError> {
    blocking(move || {
        service(&app.state::<AppState>()).set_selection(homework_id.as_deref(), &materials)
    })
    .await
}

#[tauri::command]
pub async fn preview_corpus<R: Runtime>(
    app: AppHandle<R>,
    homework_id: Option<String>,
) -> Result<CorpusPreviewDto, AppError> {
    blocking(move || service(&app.state::<AppState>()).preview_corpus(homework_id.as_deref())).await
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Mutex},
    };

    use sha2::{Digest, Sha256};
    use tauri::{Listener, Manager};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{analyze_one, run_batch};
    use crate::{
        app_state::AppState,
        db::Database,
        error::ErrorCode,
        events::names,
        extraction::{ExtractionLimits, MaterialKind, MaterialRef},
        intake::{RootKind, RootsService},
    };

    fn app() -> (
        tauri::App<tauri::test::MockRuntime>,
        TempDir,
        std::path::PathBuf,
    ) {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let vault = root.join("vault");
        fs::create_dir_all(vault.join("Предметы/00 Входящие")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        RootsService::new(&database, None)
            .set_root(RootKind::Vault, &vault)
            .unwrap();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(
            AppState::new(root.join("data"), None, database)
                .with_extraction_limits(ExtractionLimits::default())
                .with_ocr_engine(None),
        );
        (app, tmp, vault)
    }

    fn add(
        app: &tauri::App<tauri::test::MockRuntime>,
        vault: &std::path::Path,
        name: &str,
        bytes: &[u8],
    ) -> MaterialRef {
        let relative = format!("Предметы/00 Входящие/{name}");
        fs::write(vault.join(&relative), bytes).unwrap();
        let id = Uuid::new_v4().to_string();
        app.state::<AppState>()
            .database()
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO inbox_files(id, name, kind, size_bytes, added_at, origin, status, relative_path, sha256)
                     VALUES (?1, ?2, 'text', ?3, '2026-10-22T10:00:00Z', 'drop', 'new', ?4, ?5)",
                    rusqlite::params![
                        id,
                        name,
                        bytes.len() as i64,
                        relative,
                        format!("{:x}", Sha256::digest(bytes))
                    ],
                )?;
                Ok(())
            })
            .unwrap();
        MaterialRef {
            kind: MaterialKind::Inbox,
            id,
        }
    }

    fn record(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> Arc<Mutex<Vec<(String, serde_json::Value)>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        for name in [
            names::EXTRACTION_PROGRESS,
            names::EXTRACTION_UPDATED,
            names::OPERATION_PROGRESS,
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
    fn batch_command_emits_progress_then_update_then_completion_and_replays_silently() {
        let (app, _tmp, vault) = app();
        let first = add(&app, &vault, "a.md", b"# A");
        let second = add(&app, &vault, "b.txt", b"B");
        let log = record(&app);
        let operation_id = Uuid::new_v4().to_string();
        let result =
            run_batch(app.handle(), &operation_id, &[first.clone(), second], false).unwrap();
        assert_eq!(result.value.extracted, 2);

        let events = log.lock().unwrap().clone();
        let order: Vec<&str> = events
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| {
                *name != names::EXTRACTION_PROGRESS && *name != names::OPERATION_PROGRESS
            })
            .collect();
        assert_eq!(
            order,
            vec![names::EXTRACTION_UPDATED, names::OPERATION_COMPLETED]
        );
        let progress: Vec<(u64, u64)> = events
            .iter()
            .filter(|(name, _)| name == names::EXTRACTION_PROGRESS)
            .map(|(_, payload)| {
                (
                    payload["completed"].as_u64().unwrap(),
                    payload["total"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(progress, vec![(0, 2), (1, 2), (2, 2)]);
        assert!(events
            .iter()
            .filter(|(name, _)| name == names::OPERATION_PROGRESS)
            .all(|(_, payload)| payload["kind"] == "extract"
                && payload["operationId"] == operation_id.as_str()));
        let updated = events
            .iter()
            .find(|(name, _)| name == names::EXTRACTION_UPDATED)
            .unwrap();
        assert_eq!(updated.1["reason"], "analyzed");
        assert_eq!(updated.1["materials"].as_array().unwrap().len(), 2);
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(
            !serialized.contains(vault.to_str().unwrap()),
            "no absolute paths in events"
        );
        assert!(!app
            .state::<AppState>()
            .extraction_jobs()
            .is_running(&operation_id));

        let replay = run_batch(app.handle(), &operation_id, &[first], false).unwrap();
        assert!(replay.value.replayed);
        assert_eq!(
            log.lock().unwrap().len(),
            events.len(),
            "replay emits nothing"
        );
    }

    #[test]
    fn single_analysis_emits_update_and_cancel_of_unknown_batch_is_false() {
        let (app, _tmp, vault) = app();
        let material = add(&app, &vault, "a.md", b"# A");
        let log = record(&app);
        let result = analyze_one(app.handle(), &material, false).unwrap();
        assert_eq!(result.text, "# A");
        let events = log.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, names::EXTRACTION_UPDATED);
        assert!(!app
            .state::<AppState>()
            .extraction_jobs()
            .cancel(&Uuid::new_v4().to_string()));
    }

    #[test]
    fn failing_batch_emits_operation_failed() {
        let (app, _tmp, _vault) = app();
        let log = record(&app);
        let missing = MaterialRef {
            kind: MaterialKind::Inbox,
            id: "missing".into(),
        };
        let result =
            run_batch(app.handle(), &Uuid::new_v4().to_string(), &[missing], false).unwrap();
        assert_eq!(result.value.failed, 1);
        let events = log.lock().unwrap().clone();
        let failed = events
            .iter()
            .find(|(name, _)| name == names::OPERATION_FAILED)
            .unwrap();
        assert_eq!(failed.1["operation"]["kind"], "extract");
        assert_eq!(failed.1["error"]["code"], "io_error");
        assert_eq!(
            run_batch(app.handle(), "bad-id", &[], false)
                .unwrap_err()
                .code,
            ErrorCode::Blocked
        );
    }
}
