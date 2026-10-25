//! Tauri-граница этапа 8: рабочий цикл ДЗ. Команды тонкие: состояние и
//! переходы меняет только `WorkflowService`, события доставляются после commit.
//! Этап 9: запуски ждут задания передачи (`ai_commands`) или ручного ввода;
//! отмена запуска останавливает его выполняющееся задание.

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{
    app_state::AppState,
    domain::{HomeworkWorkflowStage, RequirementStatus},
    dto::{FileRefDto, HomeworkDto, HomeworkVersionDto, OperationResultDto},
    error::{AppError, ErrorCode},
    workflow::{
        bundle::VaultContentVerifier,
        dto::{
            DashboardDto, HomeworkIterationDto, HomeworkWorkflowDto, ReviewRunDto,
            RevisionPromptDto, RunKind, SourceBundleDto, StyleConfirmationDto, TeacherCommentDto,
            TeacherRequirementDto, VersionComparisonDto,
        },
        service::{
            CommandMeta, ManualAnalysisInput, ManualReviewInput, RunMode, TeacherCommentInput,
            TeacherRequirementInput, VersionInput, WorkflowEvent, WorkflowFailure, WorkflowResult,
            WorkflowService,
        },
    },
};

/// Получатель событий. Production — `AppHandle`, тесты подставляют свой.
pub trait EventSink {
    fn emit_event(&self, event: &WorkflowEvent) -> Result<(), ()>;
}

impl<R: Runtime> EventSink for AppHandle<R> {
    fn emit_event(&self, event: &WorkflowEvent) -> Result<(), ()> {
        self.emit(event.name, event.payload.clone()).map_err(|_| ())
    }
}

/// Доставляет события по порядку. Ошибка доставки не откатывает сохранённое:
/// клиент получает контролируемую ошибку с просьбой перечитать состояние.
pub fn deliver(sink: &dyn EventSink, events: &[WorkflowEvent]) -> Result<(), AppError> {
    let mut failed = false;
    for event in events {
        failed |= sink.emit_event(event).is_err();
    }
    if failed {
        Err(AppError::new(
            ErrorCode::Unknown,
            "Изменения сохранены, но UI-событие не доставлено — перечитайте состояние ДЗ",
        ))
    } else {
        Ok(())
    }
}

/// Итог команды для frontend: успех — значение с операцией; ошибка — после
/// доставки `operation:failed`.
pub fn finish<T>(
    sink: &dyn EventSink,
    result: WorkflowResult<T>,
) -> Result<OperationResultDto<T>, AppError> {
    match result {
        Ok(success) => {
            deliver(sink, &success.events)?;
            Ok(OperationResultDto {
                operation: success.operation,
                value: success.value,
            })
        }
        Err(WorkflowFailure { error, events, .. }) => {
            if !events.is_empty() && deliver(sink, &events).is_err() {
                return Err(AppError::new(
                    error.code,
                    format!("{} (UI-событие об ошибке не доставлено)", error.message),
                ));
            }
            Err(error)
        }
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

pub(crate) fn with_workflow<R: Runtime, T>(
    app: &AppHandle<R>,
    action: impl FnOnce(&WorkflowService<'_>) -> T,
) -> T {
    with_service(app, action)
}

fn with_service<R: Runtime, T>(
    app: &AppHandle<R>,
    action: impl FnOnce(&WorkflowService<'_>) -> T,
) -> T {
    let state = app.state::<AppState>();
    let verifier = VaultContentVerifier::new(state.database(), state.home_dir());
    let service = WorkflowService::new(state.database(), &verifier);
    action(&service)
}

fn meta(operation_id: String, expected_revision: Option<i64>) -> CommandMeta {
    CommandMeta {
        operation_id,
        expected_revision,
    }
}

// ───────────────────────── Чтение ─────────────────────────

#[tauri::command]
pub async fn get_workflow<R: Runtime>(
    app: AppHandle<R>,
    homework_id: String,
) -> Result<HomeworkWorkflowDto, AppError> {
    blocking(move || with_service(&app, |service| service.aggregate(&homework_id))).await
}

#[tauri::command]
pub async fn get_dashboard<R: Runtime>(app: AppHandle<R>) -> Result<DashboardDto, AppError> {
    blocking(move || {
        let state = app.state::<AppState>();
        let runtime = state.providers();
        let providers = crate::ai::AiService::new(state.database(), &runtime).statuses()?;
        with_service(&app, |service| service.dashboard(providers))
    })
    .await
}

#[tauri::command]
pub async fn get_iterations<R: Runtime>(
    app: AppHandle<R>,
    homework_id: String,
) -> Result<Vec<HomeworkIterationDto>, AppError> {
    blocking(move || with_service(&app, |service| service.iterations(&homework_id))).await
}

#[tauri::command]
pub async fn compare_versions<R: Runtime>(
    app: AppHandle<R>,
    from_version_id: String,
    to_version_id: String,
) -> Result<VersionComparisonDto, AppError> {
    blocking(move || {
        with_service(&app, |service| {
            service.compare_versions(&from_version_id, &to_version_id)
        })
    })
    .await
}

#[tauri::command]
pub async fn get_homework_candidates<R: Runtime>(
    app: AppHandle<R>,
    homework_id: String,
) -> Result<Vec<FileRefDto>, AppError> {
    blocking(move || with_service(&app, |service| service.candidates(&homework_id))).await
}

// ───────────────────────── Мутации ─────────────────────────

#[tauri::command]
pub async fn create_source_bundle<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    homework_id: String,
    purpose: RunKind,
    version_id: Option<String>,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<SourceBundleDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.create_source_bundle(
                &meta(operation_id, expected_revision),
                &homework_id,
                purpose,
                version_id.as_deref(),
            )
        });
        finish(&app, result)
    })
    .await
}

/// Параметры запуска шага рабочего цикла.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRunRequest {
    pub homework_id: String,
    pub kind: RunKind,
    pub version_id: Option<String>,
    pub bundle_id: String,
    pub mode: RunMode,
}

#[tauri::command]
pub async fn start_workflow_run<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    request: StartRunRequest,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<ReviewRunDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.start_run(
                &meta(operation_id, expected_revision),
                &request.homework_id,
                request.kind,
                request.version_id.as_deref(),
                &request.bundle_id,
                request.mode,
            )
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn cancel_workflow_run<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    run_id: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<ReviewRunDto>, AppError> {
    blocking(move || {
        // Выполняющееся задание провайдера останавливается вместе с запуском.
        {
            let state = app.state::<AppState>();
            let runtime = state.providers();
            crate::ai::AiService::new(state.database(), &runtime).cancel_for_run(&run_id)?;
        }
        let result = with_service(&app, |service| {
            service.cancel_run(&meta(operation_id, expected_revision), &run_id)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn pin_workflow_run<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    run_id: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<ReviewRunDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.pin_run(&meta(operation_id, expected_revision), &run_id)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn save_manual_analysis<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    run_id: String,
    input: ManualAnalysisInput,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<ReviewRunDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.save_manual_analysis(&meta(operation_id, expected_revision), &run_id, &input)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn save_manual_review<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    run_id: String,
    input: ManualReviewInput,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<ReviewRunDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.save_manual_review(&meta(operation_id, expected_revision), &run_id, &input)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn create_revision_prompt<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    review_run_id: String,
    text: Option<String>,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<RevisionPromptDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.create_revision_prompt(
                &meta(operation_id, expected_revision),
                &review_run_id,
                text.as_deref(),
            )
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn update_prompt_draft<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    prompt_id: String,
    text: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<RevisionPromptDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.update_prompt_draft(&meta(operation_id, expected_revision), &prompt_id, &text)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn commit_prompt<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    prompt_id: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<RevisionPromptDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.commit_prompt(&meta(operation_id, expected_revision), &prompt_id)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn hand_off_prompt<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    prompt_id: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<RevisionPromptDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.hand_off_prompt(&meta(operation_id, expected_revision), &prompt_id)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn create_homework_version<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    homework_id: String,
    input: VersionInput,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<HomeworkVersionDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.create_version(&meta(operation_id, expected_revision), &homework_id, &input)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn select_homework_version<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    version_id: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<HomeworkDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.select_version(&meta(operation_id, expected_revision), &version_id)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn add_teacher_comments<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    homework_id: String,
    comments: Vec<TeacherCommentInput>,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<Vec<TeacherCommentDto>>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.add_teacher_comments(
                &meta(operation_id, expected_revision),
                &homework_id,
                &comments,
            )
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn add_teacher_requirement<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    homework_id: String,
    input: TeacherRequirementInput,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<TeacherRequirementDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.add_teacher_requirement(
                &meta(operation_id, expected_revision),
                &homework_id,
                &input,
            )
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn resolve_teacher_requirement<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    requirement_id: String,
    status: RequirementStatus,
    note: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<TeacherRequirementDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.resolve_requirement(
                &meta(operation_id, expected_revision),
                &requirement_id,
                status,
                &note,
            )
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn confirm_style_divergence<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    review_run_id: String,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<StyleConfirmationDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.confirm_style(&meta(operation_id, expected_revision), &review_run_id)
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn mark_ready_to_submit<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    homework_id: String,
    no_teacher_comments: bool,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<HomeworkDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.mark_ready(
                &meta(operation_id, expected_revision),
                &homework_id,
                no_teacher_comments,
            )
        });
        finish(&app, result)
    })
    .await
}

#[tauri::command]
pub async fn transition_workflow<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    homework_id: String,
    to: HomeworkWorkflowStage,
    reason: Option<String>,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<HomeworkDto>, AppError> {
    blocking(move || {
        let result = with_service(&app, |service| {
            service.transition(
                &meta(operation_id, expected_revision),
                &homework_id,
                to,
                reason.as_deref(),
            )
        });
        finish(&app, result)
    })
    .await
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

    use super::{create_source_bundle, get_workflow, start_workflow_run, StartRunRequest};
    use crate::{
        app_state::AppState,
        db::Database,
        error::ErrorCode,
        events::names,
        intake::{RootKind, RootsService},
        ipc_commands::COMMANDS,
        workflow::{dto::RunKind, service::RunMode},
    };

    struct Fixture {
        app: tauri::App<tauri::test::MockRuntime>,
        vault: std::path::PathBuf,
        _tmp: TempDir,
    }

    const TASK: &str = "Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/Условие.txt";

    fn fixture() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let vault = root.join("vault");
        fs::create_dir_all(vault.join("Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        let content = "Постройте модель процесса закупки";
        fs::write(vault.join(TASK), content).unwrap();
        let sha = format!("{:x}", Sha256::digest(content.as_bytes()));
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        RootsService::new(&database, None)
            .set_root(RootKind::Vault, &vault)
            .unwrap();
        database
            .with_connection(|connection| {
                connection.execute_batch(&format!(
                    r#"
                    INSERT INTO subjects(id, name, short_name, folder_path) VALUES ('s1', 'ОБА', 'ОБА', 'Предметы/ОБА');
                    INSERT INTO weeks(id, subject_id, number, starts_at, ends_at, folder_path)
                      VALUES ('w7', 's1', 7, '2026-10-19T00:00:00Z', '2026-10-25T23:59:59Z', 'Предметы/ОБА/Неделя 07');
                    INSERT INTO homeworks(id, week_id, title, deadline, stage, workflow_stage, folder_path)
                      VALUES ('hw-1', 'w7', 'ДЗ 7', '2026-10-25T20:00:00Z', 'task', 'materials', 'Предметы/ОБА/Неделя 07/03 ДЗ');
                    INSERT INTO files(id, homework_id, category, name, kind, size_bytes, modified_at, relative_path)
                      VALUES ('task', 'hw-1', 'homework_task', 'Условие.txt', 'text', {size}, '2026-10-19T10:00:00Z', '{TASK}');
                    INSERT INTO extractions(id, sha256, extractor_version, options_key, format, status, metadata_json,
                        structure_json, blocks_truncated, text, text_chars, truncated, warnings_json, ocr_used,
                        source_size_bytes, duration_ms, created_at)
                      VALUES ('ext', '{sha}', 1, 'text', 'text', 'complete', '{{}}', '[]', 0, '{content}', 33, 0, '[]', 0,
                        {size}, 1, '2026-10-21T09:00:00Z');
                    INSERT INTO material_extractions(material_kind, material_id, sha256, extraction_id, analyzed_at)
                      VALUES ('file', 'task', '{sha}', 'ext', '2026-10-21T09:00:00Z');
                    INSERT INTO corpus_selections(id, homework_id, created_at, updated_at)
                      VALUES ('sel', 'hw-1', '2026-10-21T09:30:00Z', '2026-10-21T09:30:00Z');
                    INSERT INTO corpus_selection_items(selection_id, material_kind, material_id, sha256, position, added_at)
                      VALUES ('sel', 'file', 'task', '{sha}', 0, '2026-10-21T09:30:00Z');
                    "#,
                    size = content.len()
                ))?;
                Ok(())
            })
            .unwrap();
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(AppState::new(root.join("data"), None, database));
        Fixture {
            app,
            vault,
            _tmp: tmp,
        }
    }

    fn record(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> Arc<Mutex<Vec<(String, serde_json::Value)>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        for name in [
            names::HOMEWORK_UPDATED,
            names::HOMEWORK_READINESS_UPDATED,
            names::WORKFLOW_STAGE_CHANGED,
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
    fn ipc_creates_bundle_with_real_vault_verification_and_emits_after_commit() {
        let fixture = fixture();
        let log = record(&fixture.app);
        let handle = fixture.app.handle().clone();
        let bundle = tauri::async_runtime::block_on(create_source_bundle(
            handle.clone(),
            Uuid::new_v4().to_string(),
            "hw-1".into(),
            RunKind::AnalyzeMaterials,
            None,
            Some(0),
        ))
        .unwrap();
        assert!(bundle.value.complete);
        assert_eq!(bundle.value.items[0].file.relative_path, TASK);
        let run = tauri::async_runtime::block_on(start_workflow_run(
            handle.clone(),
            Uuid::new_v4().to_string(),
            StartRunRequest {
                homework_id: "hw-1".into(),
                kind: RunKind::AnalyzeMaterials,
                version_id: None,
                bundle_id: bundle.value.id.clone(),
                mode: RunMode::Provider,
            },
            Some(1),
        ))
        .unwrap();
        assert_eq!(
            serde_json::to_value(run.value.status).unwrap(),
            "waiting_for_provider"
        );
        let events = log.lock().unwrap().clone();
        let order: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            order,
            [
                names::HOMEWORK_UPDATED,
                names::HOMEWORK_READINESS_UPDATED,
                names::OPERATION_PROGRESS,
                names::OPERATION_COMPLETED,
                names::WORKFLOW_STAGE_CHANGED,
                names::HOMEWORK_UPDATED,
                names::HOMEWORK_READINESS_UPDATED,
                names::OPERATION_PROGRESS,
                names::OPERATION_COMPLETED,
            ]
        );
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(
            !serialized.contains(fixture.vault.to_str().unwrap()),
            "no absolute paths in events"
        );
        let workflow = tauri::async_runtime::block_on(get_workflow(handle, "hw-1".into())).unwrap();
        assert_eq!(
            serde_json::to_value(workflow.stage).unwrap(),
            "codex_analysis"
        );
        assert_eq!(workflow.revision, 2);
    }

    #[test]
    fn ipc_blocks_bundle_when_file_changed_on_disk_and_reports_failure() {
        let fixture = fixture();
        fs::write(fixture.vault.join(TASK), "Изменённое условие").unwrap();
        let log = record(&fixture.app);
        let error = tauri::async_runtime::block_on(create_source_bundle(
            fixture.app.handle().clone(),
            Uuid::new_v4().to_string(),
            "hw-1".into(),
            RunKind::AnalyzeMaterials,
            None,
            None,
        ))
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Blocked);
        assert!(!error.message.contains(fixture.vault.to_str().unwrap()));
        let events = log.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, names::OPERATION_FAILED);
        assert_eq!(events[0].1["error"]["code"], "blocked");
        let bundles: i64 = fixture
            .app
            .state::<AppState>()
            .database()
            .with_connection(|connection| {
                Ok(connection
                    .query_row("SELECT count(*) FROM source_bundles", [], |row| row.get(0))?)
            })
            .unwrap();
        assert_eq!(bundles, 0);
    }

    #[test]
    fn stale_ipc_revision_is_conflict() {
        let fixture = fixture();
        let error = tauri::async_runtime::block_on(create_source_bundle(
            fixture.app.handle().clone(),
            Uuid::new_v4().to_string(),
            "hw-1".into(),
            RunKind::AnalyzeMaterials,
            None,
            Some(5),
        ))
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
    }

    #[test]
    fn every_ipc_command_is_registered_in_the_handler_exactly_once() {
        let source = include_str!("lib.rs");
        let start = source.find("generate_handler![").unwrap();
        let end = start + source[start..].find("])").unwrap();
        let registered: Vec<String> = source[start + "generate_handler![".len()..end]
            .split(',')
            .map(|entry| entry.trim())
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.rsplit("::").next().unwrap().to_owned())
            .collect();
        let mut expected: Vec<String> = COMMANDS
            .iter()
            .map(|command| (*command).to_owned())
            .collect();
        let mut actual = registered.clone();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
        let unique: std::collections::HashSet<_> = registered.iter().collect();
        assert_eq!(unique.len(), registered.len(), "no duplicate registration");
        assert_eq!(COMMANDS.len(), 78);
    }
}
