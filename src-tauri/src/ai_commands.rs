//! Tauri-граница этапа 9: провайдеры и задания передачи материалов.
//!
//! Команды тонкие: правила — в `ai::AiService`, подтверждение результата —
//! в `WorkflowService::confirm_ai_result`. Отправка в CLI выполняется в
//! фоновом потоке после явной команды `send_ai_job`; прогресс приходит
//! событиями `ai-job:progress`, итог — `ai-job:updated`. Диалог выбора файла
//! ответа и открытие официального приложения/сайта вызываются из Rust.

use std::{
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    ai::{
        isolation::IsolationReport,
        model::{AiJobDto, AiTask, ResponseSource},
        service::{handoff_target, MAX_RESPONSE_BYTES},
        AiEventSink, AiService, TransportPreference,
    },
    app_state::AppState,
    dto::{OperationResultDto, ProviderId},
    error::{AppError, BackendError, ErrorCode},
    workflow::{
        dto::ProviderStatusDto,
        service::{AiConfirmInput, AiConfirmationDto, CommandMeta, WorkflowEvent},
    },
    workflow_commands::finish,
};

impl<R: Runtime> AiEventSink for AppHandle<R> {
    fn emit(&self, event: WorkflowEvent) {
        let _ = Emitter::emit(self, event.name, event.payload);
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

fn with_ai<R: Runtime, T>(app: &AppHandle<R>, action: impl FnOnce(&AiService<'_>) -> T) -> T {
    let state = app.state::<AppState>();
    let runtime = state.providers();
    let service = AiService::new(state.database(), &runtime);
    action(&service)
}

fn emit_status<R: Runtime>(app: &AppHandle<R>, status: &ProviderStatusDto) {
    AiEventSink::emit(app, AiService::status_event(status));
}

fn emit_job<R: Runtime>(app: &AppHandle<R>, job: &AiJobDto) {
    AiEventSink::emit(app, AiService::updated_event(job));
}

// ───────────────────────── Провайдеры ─────────────────────────

#[tauri::command]
pub async fn get_provider_statuses<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Vec<ProviderStatusDto>, AppError> {
    blocking(move || with_ai(&app, |service| service.statuses())).await
}

#[tauri::command]
pub async fn check_provider<R: Runtime>(
    app: AppHandle<R>,
    provider: ProviderId,
) -> Result<ProviderStatusDto, AppError> {
    blocking(move || {
        let status = with_ai(&app, |service| service.check(provider))?;
        emit_status(&app, &status);
        Ok(status)
    })
    .await
}

/// Реальный macOS self-test тем же runner и профилем, что production-запуски.
#[tauri::command]
pub async fn run_ai_isolation_self_test<R: Runtime>(
    app: AppHandle<R>,
) -> Result<IsolationReport, AppError> {
    blocking(move || {
        let report = with_ai(&app, |service| service.check_isolation())?;
        for status in with_ai(&app, |service| service.statuses())? {
            emit_status(&app, &status);
        }
        Ok(report)
    })
    .await
}

/// Явное подключение официального CLI пользователем.
#[tauri::command]
pub async fn connect_provider<R: Runtime>(
    app: AppHandle<R>,
    provider: ProviderId,
) -> Result<ProviderStatusDto, AppError> {
    blocking(move || {
        let status = with_ai(&app, |service| service.connect(provider))?;
        emit_status(&app, &status);
        Ok(status)
    })
    .await
}

#[tauri::command]
pub async fn disconnect_provider<R: Runtime>(
    app: AppHandle<R>,
    provider: ProviderId,
) -> Result<ProviderStatusDto, AppError> {
    blocking(move || {
        let status = with_ai(&app, |service| service.disconnect(provider))?;
        emit_status(&app, &status);
        Ok(status)
    })
    .await
}

/// Открывает официальное приложение провайдера (если установлено) или его
/// сайт во внешнем браузере. Адреса фиксированы; WebView URL не передаёт.
#[tauri::command]
pub fn open_provider_app<R: Runtime>(
    app: AppHandle<R>,
    provider: ProviderId,
) -> Result<(), AppError> {
    let (apps, web_url) = handoff_target(provider);
    for name in apps {
        let path = Path::new("/Applications").join(name);
        if path.is_dir() {
            return app
                .opener()
                .open_path(path.to_string_lossy(), None::<&str>)
                .map_err(|_| BackendError::io("Не удалось открыть официальное приложение"));
        }
    }
    app.opener()
        .open_url(web_url, None::<&str>)
        .map_err(|_| BackendError::io("Не удалось открыть сайт провайдера во внешнем браузере"))
}

// ───────────────────────── Задания ─────────────────────────

#[tauri::command]
pub async fn list_ai_jobs<R: Runtime>(
    app: AppHandle<R>,
    homework_id: String,
) -> Result<Vec<AiJobDto>, AppError> {
    blocking(move || with_ai(&app, |service| service.jobs(&homework_id))).await
}

#[tauri::command]
pub async fn get_ai_job<R: Runtime>(
    app: AppHandle<R>,
    job_id: String,
) -> Result<AiJobDto, AppError> {
    blocking(move || with_ai(&app, |service| service.job(&job_id))).await
}

/// Подготовка: полный текст и состав передачи. Ничего не отправляет.
#[tauri::command]
pub async fn prepare_ai_job<R: Runtime>(
    app: AppHandle<R>,
    run_id: String,
    task: AiTask,
    transport: TransportPreference,
) -> Result<AiJobDto, AppError> {
    blocking(move || {
        let job = with_ai(&app, |service| service.prepare(&run_id, task, transport))?;
        emit_job(&app, &job);
        Ok(job)
    })
    .await
}

/// Явная отправка подготовленного задания в официальный CLI. Возвращается
/// сразу (`running`); процесс выполняется в фоновом потоке.
#[tauri::command]
pub async fn send_ai_job<R: Runtime>(
    app: AppHandle<R>,
    job_id: String,
) -> Result<AiJobDto, AppError> {
    blocking(move || {
        let plan = with_ai(&app, |service| service.begin_send(&job_id))?;
        let job = plan.job.clone();
        emit_job(&app, &job);
        let worker = app.clone();
        std::thread::Builder::new()
            .name("kontur-ai-job".to_owned())
            .spawn(move || {
                let state = worker.state::<AppState>();
                let runtime = state.providers();
                AiService::new(state.database(), &runtime).execute(plan, &worker);
            })
            .map_err(|_| BackendError::io("Не удалось запустить фоновую задачу"))?;
        Ok(job)
    })
    .await
}

#[tauri::command]
pub async fn cancel_ai_job<R: Runtime>(
    app: AppHandle<R>,
    job_id: String,
) -> Result<bool, AppError> {
    blocking(move || {
        let cancelled = with_ai(&app, |service| service.cancel(&job_id))?;
        if let Ok(job) = with_ai(&app, |service| service.job(&job_id)) {
            emit_job(&app, &job);
        }
        Ok(cancelled)
    })
    .await
}

/// Ответ, вставленный пользователем после ручной передачи.
#[tauri::command]
pub async fn submit_ai_response<R: Runtime>(
    app: AppHandle<R>,
    job_id: String,
    text: String,
) -> Result<AiJobDto, AppError> {
    blocking(move || {
        let job = with_ai(&app, |service| {
            service.submit_response(&job_id, &text, ResponseSource::Paste, None)
        })?;
        emit_job(&app, &job);
        Ok(job)
    })
    .await
}

/// Выбор файла ответа (txt/md/json) в системном диалоге. Отмена — `null`.
pub trait ResponseFilePicker {
    fn pick(&self) -> Option<PathBuf>;
}

struct DialogResponsePicker<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> ResponseFilePicker for DialogResponsePicker<R> {
    fn pick(&self) -> Option<PathBuf> {
        self.app
            .dialog()
            .file()
            .set_title("Импортировать ответ провайдера")
            .add_filter("Текст ответа", &["txt", "md", "json"])
            .blocking_pick_file()
            .and_then(|path| path.into_path().ok())
    }
}

/// Читает выбранный пользователем файл ответа: без следования symlink,
/// только обычный файл ≤ 2 МБ в UTF-8. Путь не сохраняется — только имя.
pub fn read_response_file(path: &Path) -> Result<(String, String), AppError> {
    let flags = (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .map_err(|_| BackendError::blocked("Файл ответа не открыт (ссылка или нет доступа)"))?;
    let metadata = file
        .metadata()
        .map_err(|_| BackendError::io("Не удалось прочитать атрибуты файла"))?;
    if !metadata.is_file() {
        return Err(BackendError::blocked("Выбран не обычный файл"));
    }
    if metadata.len() > MAX_RESPONSE_BYTES as u64 {
        return Err(BackendError::blocked("Файл ответа больше 2 МБ"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BackendError::io("Файл ответа не прочитан"))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(BackendError::blocked("Файл ответа больше 2 МБ"));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| BackendError::blocked("Файл ответа не в кодировке UTF-8"))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ответ.txt".to_owned());
    Ok((text, name))
}

pub fn import_response_with(
    service: &AiService<'_>,
    picker: &dyn ResponseFilePicker,
    job_id: &str,
) -> Result<Option<AiJobDto>, AppError> {
    // Задание проверяется до диалога: закрытому заданию файл не нужен.
    service.job(job_id)?;
    let Some(path) = picker.pick() else {
        return Ok(None);
    };
    let (text, name) = read_response_file(&path)?;
    service
        .submit_response(job_id, &text, ResponseSource::File, Some(&name))
        .map(Some)
}

#[tauri::command]
pub async fn import_ai_response_file<R: Runtime>(
    app: AppHandle<R>,
    job_id: String,
) -> Result<Option<AiJobDto>, AppError> {
    blocking(move || {
        let picker = DialogResponsePicker { app: app.clone() };
        let job = with_ai(&app, |service| {
            import_response_with(service, &picker, &job_id)
        })?;
        if let Some(job) = &job {
            emit_job(&app, job);
        }
        Ok(job)
    })
    .await
}

#[tauri::command]
pub async fn discard_ai_job<R: Runtime>(
    app: AppHandle<R>,
    job_id: String,
) -> Result<AiJobDto, AppError> {
    blocking(move || {
        let job = with_ai(&app, |service| service.discard(&job_id))?;
        emit_job(&app, &job);
        Ok(job)
    })
    .await
}

/// Подтверждение пользователем проверенного (и, возможно, исправленного)
/// результата. Только после него ответ провайдера меняет рабочий цикл.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn confirm_ai_result<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    job_id: String,
    supporting_job_id: Option<String>,
    input: AiConfirmInput,
    expected_revision: Option<i64>,
) -> Result<OperationResultDto<AiConfirmationDto>, AppError> {
    blocking(move || {
        let result = crate::workflow_commands::with_workflow(&app, |service| {
            service.confirm_ai_result(
                &CommandMeta {
                    operation_id,
                    expected_revision,
                },
                &job_id,
                supporting_job_id.as_deref(),
                &input,
            )
        });
        let output = finish(&app, result)?;
        emit_job(&app, &output.value.job);
        Ok(output)
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use tauri::{Listener, Manager};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{
        confirm_ai_result, get_ai_job, import_response_with, prepare_ai_job, read_response_file,
        send_ai_job, submit_ai_response, ResponseFilePicker,
    };
    use crate::{
        ai::{
            adapters::{
                AdapterFactory, AdapterRequest, AdapterResponse, AdapterResult, AiAdapter,
                RunControl,
            },
            locate::CliLocator,
            model::{AiTask, JobStatus, ParseStatus},
            AiService, ProviderRuntime, TransportPreference,
        },
        app_state::AppState,
        dto::ProviderId,
        error::ErrorCode,
        events::names,
        workflow::{
            dto::{ReviewRunStatus, RunKind},
            service::{AiConfirmInput, CommandMeta, ManualAnalysisInput, RunMode, WorkflowService},
            tests::{seeded, FakeVerifier},
        },
    };

    struct Answer;

    impl AiAdapter for Answer {
        fn label(&self) -> String {
            "Fake CLI".into()
        }
        fn execute(
            &self,
            _request: &AdapterRequest<'_>,
            _control: RunControl<'_>,
        ) -> AdapterResult {
            Ok(AdapterResponse {
                text: "{\"summary\": \"Итог\", \"promptText\": \"Промт\"}".into(),
                truncated: false,
                exit_code: Some(0),
                stderr_excerpt: String::new(),
                duration_ms: 1,
                notes: Vec::new(),
            })
        }
    }

    struct Factory;

    impl AdapterFactory for Factory {
        fn adapter(
            &self,
            _provider: ProviderId,
        ) -> Result<Arc<dyn AiAdapter>, crate::error::AppError> {
            Ok(Arc::new(Answer))
        }
    }

    const FAKE_CODEX: &str = "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'codex-cli 0.99.0';;\n  login) exit 0;;\n  exec) echo 'Usage: codex exec [OPTIONS]'; echo 'Prompt from stdin'; echo 'read-only'; for f in --sandbox --skip-git-repo-check --output-last-message --ephemeral --ignore-user-config --ignore-rules --strict-config; do echo \"  $f <X>\"; done;;\nesac\n";

    fn app() -> (tauri::App<tauri::test::MockRuntime>, TempDir) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let bin = root.join("prefix/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("codex"), FAKE_CODEX).unwrap();
        fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
        let database = seeded();
        let runtime = ProviderRuntime::for_tests(
            root.join("data/ai"),
            CliLocator::with_dirs(vec![bin], vec![root.join("prefix")], None),
            Some(Arc::new(Factory)),
            Duration::from_secs(5),
        );
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(AppState::new(root.join("data"), None, database).with_providers(runtime));
        (app, tmp)
    }

    fn start_analysis(app: &tauri::App<tauri::test::MockRuntime>) -> String {
        let state = app.state::<AppState>();
        let verifier = FakeVerifier::default();
        let service = WorkflowService::new(state.database(), &verifier);
        let meta = || CommandMeta {
            operation_id: Uuid::new_v4().to_string(),
            expected_revision: None,
        };
        let bundle = service
            .create_source_bundle(&meta(), "hw-1", RunKind::AnalyzeMaterials, None)
            .unwrap()
            .value
            .id;
        service
            .start_run(
                &meta(),
                "hw-1",
                RunKind::AnalyzeMaterials,
                None,
                &bundle,
                RunMode::Provider,
            )
            .unwrap()
            .value
            .id
    }

    #[test]
    fn ipc_prepare_send_and_confirm_emit_job_events_in_order() {
        let (app, _tmp) = app();
        {
            let state = app.state::<AppState>();
            let runtime = state.providers();
            AiService::new(state.database(), &runtime)
                .connect(ProviderId::Codex)
                .unwrap();
        }
        let run = start_analysis(&app);
        let log = Arc::new(Mutex::new(Vec::new()));
        for name in [
            names::AI_JOB_PROGRESS,
            names::AI_JOB_UPDATED,
            names::REVIEW_COMPLETED,
            names::OPERATION_COMPLETED,
        ] {
            let log = Arc::clone(&log);
            app.listen_any(name, move |event| {
                log.lock().unwrap().push((name, event.payload().to_owned()));
            });
        }
        let handle = app.handle().clone();
        let job = tauri::async_runtime::block_on(prepare_ai_job(
            handle.clone(),
            run.clone(),
            AiTask::MaterialAnalysis,
            TransportPreference::Auto,
        ))
        .unwrap();
        assert_eq!(job.status, JobStatus::Prepared);
        let sent =
            tauri::async_runtime::block_on(send_ai_job(handle.clone(), job.id.clone())).unwrap();
        assert_eq!(sent.status, JobStatus::Running);
        let started = Instant::now();
        let done = loop {
            let current =
                tauri::async_runtime::block_on(get_ai_job(handle.clone(), job.id.clone())).unwrap();
            if current.status != JobStatus::Running {
                break current;
            }
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(done.status, JobStatus::ResponseReady);
        assert_eq!(done.parse_status, Some(ParseStatus::Valid));
        let confirmed = tauri::async_runtime::block_on(confirm_ai_result(
            handle.clone(),
            Uuid::new_v4().to_string(),
            job.id.clone(),
            None,
            AiConfirmInput::Analysis(ManualAnalysisInput {
                summary: "Итог".into(),
                prompt_text: "Промт".into(),
            }),
            None,
        ))
        .unwrap();
        assert_eq!(confirmed.value.run.status, ReviewRunStatus::Completed);
        assert_eq!(confirmed.value.job.status, JobStatus::Confirmed);
        let events = log.lock().unwrap().clone();
        let order: Vec<&str> = events.iter().map(|(name, _)| *name).collect();
        let first_progress = order
            .iter()
            .position(|name| *name == names::AI_JOB_PROGRESS)
            .unwrap();
        let review = order
            .iter()
            .position(|name| *name == names::REVIEW_COMPLETED)
            .unwrap();
        assert!(
            order[..first_progress].contains(&names::AI_JOB_UPDATED),
            "prepare/send emit updates first"
        );
        assert!(
            first_progress < review,
            "result applies only after the job finished and was confirmed"
        );
        assert_eq!(order.last(), Some(&names::AI_JOB_UPDATED));
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(
            !serialized.contains("Постройте модель процесса"),
            "events carry ids, not material text"
        );
    }

    #[test]
    fn ipc_rejects_cli_send_for_manual_jobs_and_accepts_pasted_answers() {
        let (app, _tmp) = app();
        let run = start_analysis(&app);
        let handle = app.handle().clone();
        let job = tauri::async_runtime::block_on(prepare_ai_job(
            handle.clone(),
            run,
            AiTask::MaterialAnalysis,
            TransportPreference::Auto,
        ))
        .unwrap();
        let error = tauri::async_runtime::block_on(send_ai_job(handle.clone(), job.id.clone()))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Blocked);
        let pasted = tauri::async_runtime::block_on(submit_ai_response(
            handle,
            job.id,
            "{\"summary\": \"вставлено\", \"promptText\": \"промт\"}".into(),
        ))
        .unwrap();
        assert_eq!(pasted.status, JobStatus::ResponseReady);
    }

    struct Picker(Option<PathBuf>);

    impl ResponseFilePicker for Picker {
        fn pick(&self) -> Option<PathBuf> {
            self.0.clone()
        }
    }

    #[test]
    fn response_file_import_reads_only_regular_small_utf8_files() {
        let (app, tmp) = app();
        let run = start_analysis(&app);
        let state = app.state::<AppState>();
        let runtime = state.providers();
        let service = AiService::new(state.database(), &runtime);
        let job = service
            .prepare(
                &run,
                AiTask::MaterialAnalysis,
                TransportPreference::ManualHandoff,
            )
            .unwrap();
        assert_eq!(
            import_response_with(&service, &Picker(None), &job.id).unwrap(),
            None,
            "cancelled dialog"
        );

        let dir = tmp.path().join("downloads");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("ответ.md"), "\u{feff}{\"summary\": \"из файла\"}").unwrap();
        std::os::unix::fs::symlink(dir.join("ответ.md"), dir.join("link.md")).unwrap();
        fs::write(
            dir.join("big.txt"),
            vec![b'a'; super::MAX_RESPONSE_BYTES + 1],
        )
        .unwrap();
        fs::write(dir.join("bin.txt"), [0xff_u8, 0xfe, 0x00]).unwrap();
        assert_eq!(
            read_response_file(&dir.join("link.md")).unwrap_err().code,
            ErrorCode::Blocked
        );
        assert_eq!(
            read_response_file(&dir.join("big.txt")).unwrap_err().code,
            ErrorCode::Blocked
        );
        assert_eq!(
            read_response_file(&dir.join("bin.txt")).unwrap_err().code,
            ErrorCode::Blocked
        );
        assert_eq!(
            read_response_file(&dir).unwrap_err().code,
            ErrorCode::Blocked
        );

        let imported = import_response_with(&service, &Picker(Some(dir.join("ответ.md"))), &job.id)
            .unwrap()
            .unwrap();
        assert_eq!(imported.response_file_name.as_deref(), Some("ответ.md"));
        assert_eq!(imported.draft.unwrap()["summary"], "из файла");
        let stored: String = state
            .database()
            .with_connection(|connection| {
                Ok(connection.query_row(
                    "SELECT log_json FROM ai_jobs WHERE id = ?1",
                    [&job.id],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert!(
            !stored.contains(tmp.path().to_str().unwrap()),
            "the path of the file is not stored"
        );
    }
}
