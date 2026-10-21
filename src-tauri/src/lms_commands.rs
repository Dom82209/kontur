//! Tauri-граница этапа 6: watcher загрузок, кандидаты, предпросмотр и импорт
//! экспорта CU LMS Enhancer, ссылки на курс и задание.
//!
//! Системный диалог и внешний браузер вызываются только из Rust через
//! официальные плагины; frontend не получает прав `dialog:*`, `opener:*`, `fs:*`.
//! Ссылка открывается во внешнем браузере пользователя только после повторной
//! проверки allowlist HTTPS-домена LMS. Вход в LMS, cookies и отправка работ
//! не автоматизируются.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    app_state::AppState,
    dto::{HomeworkDto, OperationKind, OperationResultDto, SubjectDto},
    error::{AppError, BackendError, ErrorCode},
    events::{
        emit_bridge_event, names, CuImportProgressPayload, DownloadsWatcherUpdatedPayload,
        InboxFileAddedPayload, InboxUpdateReason, InboxUpdatedPayload, OperationCompletedPayload,
        OperationFailedPayload, OperationProgressPayload,
    },
    intake::{
        validate_operation_id, FilePicker, InboxImportReport, InboxImporter, RootState,
        RootsService,
    },
    lms::{
        self, open_candidate, preview, store as lms_store, CandidateOrigin, CandidateSource,
        DownloadCandidateDto, DownloadsWatcherDto, ExportImport, ExportImportRequest, ExportPhase,
        ExportPreviewDto, ScannerConfig, SourceContext, WatcherHost, WatcherState, ZipLimits,
    },
    query_service::QueryService,
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

fn roots_service(state: &AppState) -> RootsService<'_> {
    RootsService::new(state.database(), state.home_dir())
}

// ───────────────────────── Watcher ─────────────────────────

struct AppWatcherHost<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> WatcherHost for AppWatcherHost<R> {
    fn with_state(
        &self,
        task: &mut lms::watcher::WatcherTask<'_>,
    ) -> Option<Result<lms::watcher::TickOutcome, AppError>> {
        let state = self.app.try_state::<AppState>()?;
        let home = state.home_dir();
        Some(task(state.database(), state.candidates(), home.as_deref()))
    }

    fn notify(&self) {
        if let Some(state) = self.app.try_state::<AppState>() {
            if let Ok(snapshot) = watcher_snapshot(&state) {
                let _ = emit_watcher(&self.app, snapshot);
            }
        }
    }
}

pub fn watcher_snapshot(state: &AppState) -> Result<DownloadsWatcherDto, AppError> {
    let settings = lms_store::read_watcher_settings(state.database())?;
    state
        .watcher()
        .snapshot(settings.enabled, state.candidates())
}

fn emit_watcher<R: Runtime>(
    app: &AppHandle<R>,
    watcher: DownloadsWatcherDto,
) -> Result<(), AppError> {
    emit_bridge_event(
        app,
        names::DOWNLOADS_WATCHER_UPDATED,
        DownloadsWatcherUpdatedPayload { watcher },
    )
}

fn downloads_folder(state: &AppState) -> Result<PathBuf, AppError> {
    let service = roots_service(state);
    let status = service.status()?;
    if status.downloads.state != RootState::Ok {
        return Err(BackendError::blocked(
            "Сначала выберите папку загрузок в «Настройки → Управляемые каталоги»",
        ));
    }
    service
        .config()?
        .downloads_path
        .map(PathBuf::from)
        .ok_or_else(|| BackendError::blocked("Папка загрузок не выбрана"))
}

/// Запускает watcher при старте, если пользователь включил его ранее.
pub fn resume_watcher<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Ok(settings) = lms_store::read_watcher_settings(state.database()) else {
        return;
    };
    if !settings.enabled {
        return;
    }
    match downloads_folder(&state) {
        Ok(folder) => {
            let host = Box::new(AppWatcherHost { app: app.clone() });
            if let Err(error) = state
                .watcher()
                .start(host, folder, ScannerConfig::default())
            {
                let _ = state
                    .watcher()
                    .mark(WatcherState::Error, Some(error.message));
            }
        }
        Err(error) => {
            let _ = state
                .watcher()
                .mark(WatcherState::NoFolder, Some(error.message));
        }
    }
}

#[tauri::command]
pub fn get_downloads_watcher(
    state: tauri::State<'_, AppState>,
) -> Result<DownloadsWatcherDto, AppError> {
    watcher_snapshot(&state)
}

#[tauri::command]
pub async fn set_downloads_watcher<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<DownloadsWatcherDto, AppError> {
    blocking(move || set_watcher_enabled(&app, enabled)).await
}

fn set_watcher_enabled<R: Runtime>(
    app: &AppHandle<R>,
    enabled: bool,
) -> Result<DownloadsWatcherDto, AppError> {
    let state = app.state::<AppState>();
    if enabled {
        let folder = downloads_folder(&state)?;
        lms_store::write_watcher_settings(
            state.database(),
            lms_store::WatcherSettings { enabled: true },
        )?;
        let host = Box::new(AppWatcherHost { app: app.clone() });
        state
            .watcher()
            .start(host, folder, ScannerConfig::default())?;
    } else {
        lms_store::write_watcher_settings(
            state.database(),
            lms_store::WatcherSettings { enabled: false },
        )?;
        state.watcher().stop()?;
        state.candidates().clear_watcher()?;
    }
    let snapshot = watcher_snapshot(&state)?;
    let _ = emit_watcher(app, snapshot.clone());
    Ok(snapshot)
}

#[tauri::command]
pub async fn dismiss_download_candidate<R: Runtime>(
    app: AppHandle<R>,
    candidate_id: String,
) -> Result<DownloadsWatcherDto, AppError> {
    blocking(move || {
        let state = app.state::<AppState>();
        if let Some(candidate) = state.candidates().remove(&candidate_id)? {
            if let Some(sha256) = &candidate.sha256 {
                lms_store::mark_seen(
                    state.database(),
                    sha256,
                    &candidate.name,
                    candidate.size_bytes,
                    lms_store::SeenOutcome::Dismissed,
                    None,
                )?;
            }
        }
        let snapshot = watcher_snapshot(&state)?;
        let _ = emit_watcher(&app, snapshot.clone());
        Ok(snapshot)
    })
    .await
}

// ───────────────────────── Экспорт ЦУ ─────────────────────────

struct ExportDialog<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> FilePicker for ExportDialog<R> {
    fn pick_files(&self, _start: Option<&Path>) -> Option<Vec<PathBuf>> {
        None
    }

    fn pick_folder(&self, _title: &str, _start: Option<&Path>) -> Option<PathBuf> {
        None
    }

    fn pick_export(&self, start: Option<&Path>) -> Option<PathBuf> {
        let mut dialog = self
            .app
            .dialog()
            .file()
            .set_title("Импортировать экспорт ЦУ")
            .add_filter("Экспорт CU LMS Enhancer", &["zip", "pdf"]);
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        dialog
            .blocking_pick_file()
            .and_then(|path| path.into_path().ok())
    }
}

/// Регистрирует файл экспорта, выбранный в системном диалоге. Отмена — `None`.
pub fn register_picked_export(
    state: &AppState,
    picker: &dyn FilePicker,
) -> Result<Option<DownloadCandidateDto>, AppError> {
    let start = roots_service(state)
        .config()?
        .downloads_path
        .map(PathBuf::from)
        .filter(|path| path.is_dir());
    let Some(path) = picker.pick_export(start.as_deref()) else {
        return Ok(None);
    };
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| BackendError::not_found("Выбранный файл не найден"))?;
    if !path.is_absolute() || !metadata.is_file() {
        return Err(BackendError::blocked("Выберите обычный файл ZIP или PDF"));
    }
    let name = crate::intake::safe_inbox_name(path.file_name().unwrap_or_default())?;
    state.grants().grant([path.clone()]);
    let candidate = state.candidates().register(
        name,
        metadata.len(),
        CandidateOrigin::Manual,
        CandidateSource::External { path },
        None,
    )?;
    Ok(Some(DownloadCandidateDto::from(&candidate)))
}

#[tauri::command]
pub async fn pick_cu_export<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<DownloadCandidateDto>, AppError> {
    blocking(move || {
        let state = app.state::<AppState>();
        let picker = ExportDialog { app: app.clone() };
        let picked = register_picked_export(&state, &picker)?;
        if picked.is_some() {
            if let Ok(snapshot) = watcher_snapshot(&state) {
                let _ = emit_watcher(&app, snapshot);
            }
        }
        Ok(picked)
    })
    .await
}

fn source_context(state: &AppState) -> Result<(Option<PathBuf>, Option<PathBuf>), AppError> {
    Ok((
        roots_service(state)
            .config()?
            .downloads_path
            .map(PathBuf::from),
        state.home_dir(),
    ))
}

pub fn preview_candidate(
    state: &AppState,
    candidate_id: &str,
    limits: &ZipLimits,
) -> Result<ExportPreviewDto, AppError> {
    let candidate = state.candidates().get(candidate_id)?;
    let (downloads_path, home) = source_context(state)?;
    let source = open_candidate(
        &candidate,
        &SourceContext {
            downloads_path,
            home: home.as_deref(),
            grants: state.grants(),
        },
    )?;
    preview(state.database(), &candidate, &source, limits)
}

#[tauri::command]
pub async fn preview_cu_export<R: Runtime>(
    app: AppHandle<R>,
    candidate_id: String,
) -> Result<ExportPreviewDto, AppError> {
    blocking(move || {
        preview_candidate(
            &app.state::<AppState>(),
            &candidate_id,
            &ZipLimits::default(),
        )
    })
    .await
}

#[tauri::command]
pub async fn import_cu_export<R: Runtime>(
    app: AppHandle<R>,
    operation_id: String,
    request: ExportImportRequest,
) -> Result<OperationResultDto<InboxImportReport>, AppError> {
    blocking(move || run_cu_import(&app, &operation_id, &request, ZipLimits::default())).await
}

pub fn run_cu_import<R: Runtime>(
    app: &AppHandle<R>,
    operation_id: &str,
    request: &ExportImportRequest,
    limits: ZipLimits,
) -> Result<OperationResultDto<InboxImportReport>, AppError> {
    validate_operation_id(operation_id)?;
    let state = app.state::<AppState>();
    let _serialized = state.lock_imports()?;
    let roots = roots_service(&state).open_import_roots()?;
    let importer = InboxImporter::new(state.database(), &roots, state.grants());

    // Повтор команды: сохранённый итог без файловых действий и событий.
    if let Some(replayed) = importer.replay_if_finished(operation_id)? {
        return Ok(OperationResultDto {
            operation: replayed.operation,
            value: replayed.report,
        });
    }

    let candidate = state.candidates().get(&request.candidate_id)?;
    let (downloads_path, home) = source_context(&state)?;
    let source = open_candidate(
        &candidate,
        &SourceContext {
            downloads_path,
            home: home.as_deref(),
            grants: state.grants(),
        },
    )?;
    let scratch = state.scratch_dir();
    let export = ExportImport {
        database: state.database(),
        importer: &importer,
        scratch_base: &scratch,
        limits,
    };
    let mut delivery_failed = false;
    let result = export.run(
        operation_id,
        &candidate,
        &source,
        request,
        &mut |progress| {
            let fraction = if progress.total == 0 {
                0.0
            } else {
                let base = progress.completed as f64 / progress.total as f64;
                match progress.phase {
                    ExportPhase::Inspect => 0.0,
                    ExportPhase::Extract => base,
                    ExportPhase::Import => (base + 0.5 / progress.total as f64).min(1.0),
                    ExportPhase::Cleanup => 1.0,
                }
            };
            let message = progress
                .entry_name
                .clone()
                .unwrap_or_else(|| phase_label(progress.phase).to_owned());
            delivery_failed |= emit_bridge_event(
                app,
                names::CU_IMPORT_PROGRESS,
                CuImportProgressPayload {
                    operation_id: operation_id.to_owned(),
                    phase: progress.phase,
                    completed: progress.completed,
                    total: progress.total,
                    entry_name: progress.entry_name.clone(),
                },
            )
            .is_err();
            delivery_failed |= emit_bridge_event(
                app,
                names::OPERATION_PROGRESS,
                OperationProgressPayload {
                    operation_id: operation_id.to_owned(),
                    kind: OperationKind::Import,
                    progress: fraction,
                    message,
                },
            )
            .is_err();
        },
    )?;
    let outcome = result.outcome;
    if outcome.failure.is_none() {
        let _ = state.candidates().remove(&candidate.id);
    }

    delivery_failed |= emit_import_events(app, &outcome);
    if let Ok(snapshot) = watcher_snapshot(&state) {
        delivery_failed |= emit_watcher(app, snapshot).is_err();
    }
    if delivery_failed {
        return Err(AppError::new(
            ErrorCode::Unknown,
            "Импорт сохранён, но UI-событие не доставлено; обновите «Входящие»",
        ));
    }
    Ok(OperationResultDto {
        operation: outcome.operation,
        value: outcome.report,
    })
}

fn phase_label(phase: ExportPhase) -> &'static str {
    match phase {
        ExportPhase::Inspect => "Проверка архива",
        ExportPhase::Extract => "Распаковка во временную папку",
        ExportPhase::Import => "Импорт во «Входящие»",
        ExportPhase::Cleanup => "Удаление временных файлов",
    }
}

/// `inbox:file-added` → `inbox:updated` → `operation:completed`/`operation:failed`.
/// Возвращает `true`, если хотя бы одно событие не доставлено.
fn emit_import_events<R: Runtime>(
    app: &AppHandle<R>,
    outcome: &crate::intake::ImportOutcome,
) -> bool {
    let mut failed = false;
    let added = outcome.added_files();
    if !added.is_empty() {
        failed |= emit_bridge_event(
            app,
            names::INBOX_FILE_ADDED,
            InboxFileAddedPayload { files: added },
        )
        .is_err();
    }
    failed |= emit_bridge_event(
        app,
        names::INBOX_UPDATED,
        InboxUpdatedPayload {
            reason: InboxUpdateReason::Import,
            file_ids: outcome.touched_file_ids(),
        },
    )
    .is_err();
    failed |= match &outcome.failure {
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
    failed
}

// ───────────────────────── Ссылки LMS ─────────────────────────

#[tauri::command]
pub fn set_subject_course_url(
    state: tauri::State<'_, AppState>,
    subject_id: String,
    url: Option<String>,
) -> Result<SubjectDto, AppError> {
    update_course_url(&state, &subject_id, url.as_deref())
}

pub fn update_course_url(
    state: &AppState,
    subject_id: &str,
    url: Option<&str>,
) -> Result<SubjectDto, AppError> {
    let normalized = url.map(lms::normalize_lms_url).transpose()?.flatten();
    lms_store::set_course_url(state.database(), subject_id, normalized.as_deref())?;
    QueryService::new(state.database())
        .subjects()?
        .into_iter()
        .find(|subject| subject.id == subject_id)
        .ok_or_else(|| BackendError::not_found("Предмет не найден"))
}

#[tauri::command]
pub fn set_homework_assignment_url(
    state: tauri::State<'_, AppState>,
    homework_id: String,
    url: Option<String>,
) -> Result<HomeworkDto, AppError> {
    update_assignment_url(&state, &homework_id, url.as_deref())
}

pub fn update_assignment_url(
    state: &AppState,
    homework_id: &str,
    url: Option<&str>,
) -> Result<HomeworkDto, AppError> {
    let normalized = url.map(lms::normalize_lms_url).transpose()?.flatten();
    lms_store::set_assignment_url(state.database(), homework_id, normalized.as_deref())?;
    QueryService::new(state.database()).homework(homework_id)
}

/// Ссылка курса: у предмета или из происхождения файла «Входящих».
pub fn resolve_course_url(
    state: &AppState,
    subject_id: Option<&str>,
    file_id: Option<&str>,
) -> Result<String, AppError> {
    let stored = match (subject_id, file_id) {
        (Some(subject_id), None) => lms_store::course_url(state.database(), subject_id)?,
        (None, Some(file_id)) => lms_store::inbox_provenance(state.database(), file_id)?
            .and_then(|provenance| provenance.course_url),
        _ => return Err(BackendError::blocked("Укажите либо предмет, либо файл")),
    };
    lms::require_lms_url(stored.as_deref(), "Ссылка на курс не задана")
}

/// Ссылка задания: у ДЗ или из происхождения файла «Входящих».
pub fn resolve_assignment_url(
    state: &AppState,
    homework_id: Option<&str>,
    file_id: Option<&str>,
) -> Result<String, AppError> {
    let stored = match (homework_id, file_id) {
        (Some(homework_id), None) => lms_store::assignment_url(state.database(), homework_id)?,
        (None, Some(file_id)) => lms_store::inbox_provenance(state.database(), file_id)?
            .and_then(|provenance| provenance.assignment_url),
        _ => return Err(BackendError::blocked("Укажите либо ДЗ, либо файл")),
    };
    lms::require_lms_url(stored.as_deref(), "Ссылка на задание не задана")
}

fn open_external_url<R: Runtime>(app: &AppHandle<R>, url: &str) -> Result<(), AppError> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| BackendError::io("Не удалось открыть ссылку во внешнем браузере"))
}

#[tauri::command]
pub fn open_course<R: Runtime>(
    app: AppHandle<R>,
    subject_id: Option<String>,
    file_id: Option<String>,
) -> Result<(), AppError> {
    let url = resolve_course_url(
        &app.state::<AppState>(),
        subject_id.as_deref(),
        file_id.as_deref(),
    )?;
    open_external_url(&app, &url)
}

#[tauri::command]
pub fn open_assignment<R: Runtime>(
    app: AppHandle<R>,
    homework_id: Option<String>,
    file_id: Option<String>,
) -> Result<(), AppError> {
    let url = resolve_assignment_url(
        &app.state::<AppState>(),
        homework_id.as_deref(),
        file_id.as_deref(),
    )?;
    open_external_url(&app, &url)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
    };

    use tauri::{Listener, Manager};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{
        register_picked_export, resolve_assignment_url, resolve_course_url, run_cu_import,
        set_watcher_enabled, update_assignment_url, update_course_url, watcher_snapshot,
    };
    use crate::{
        app_state::AppState,
        db::Database,
        dto::{MaterialCategory, ProvenanceSource, ProvenanceVia},
        error::ErrorCode,
        events::names,
        intake::{
            FilePicker, ImportItemInput, ImportOrigin, InboxImporter, RootKind, RootsService,
        },
        lms::{
            zip_safe::test_zip::{build, RawEntry},
            ExportImportEntry, ExportImportRequest, WatcherState, ZipLimits,
        },
        repositories::ReadRepository,
    };

    struct Picker(Option<PathBuf>);

    impl FilePicker for Picker {
        fn pick_files(&self, _start: Option<&Path>) -> Option<Vec<PathBuf>> {
            None
        }
        fn pick_folder(&self, _title: &str, _start: Option<&Path>) -> Option<PathBuf> {
            None
        }
        fn pick_export(&self, _start: Option<&Path>) -> Option<PathBuf> {
            self.0.clone()
        }
    }

    fn app(with_downloads: bool) -> (tauri::App<tauri::test::MockRuntime>, TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        for dir in ["vault", "downloads", "external", "data"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
            .with_connection(|connection| {
                connection.execute_batch(
                    "INSERT INTO subjects(id, name, short_name, folder_path)
                     VALUES ('oba', 'Основы бизнес-аналитики', 'ОБА', 'Предметы/ОБА');",
                )?;
                Ok(())
            })
            .unwrap();
        let roots = RootsService::new(&database, None);
        roots
            .set_root(RootKind::Vault, &root.join("vault"))
            .unwrap();
        if with_downloads {
            roots
                .set_root(RootKind::Downloads, &root.join("downloads"))
                .unwrap();
        }
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        app.manage(AppState::new(root.join("data"), None, database));
        (app, tmp, root)
    }

    fn record(
        app: &tauri::App<tauri::test::MockRuntime>,
    ) -> Arc<Mutex<Vec<(String, serde_json::Value)>>> {
        let log = Arc::new(Mutex::new(Vec::new()));
        for name in [
            names::CU_IMPORT_PROGRESS,
            names::OPERATION_PROGRESS,
            names::INBOX_FILE_ADDED,
            names::INBOX_UPDATED,
            names::OPERATION_COMPLETED,
            names::OPERATION_FAILED,
            names::DOWNLOADS_WATCHER_UPDATED,
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
    fn cu_import_command_emits_phases_then_inbox_events_then_completion() {
        let (app, _tmp, root) = app(false);
        let archive = root.join("external/Основы бизнес-аналитики.zip");
        fs::write(
            &archive,
            build(
                &[
                    RawEntry::file("Неделя 4/Лекции/Лекция 4.pdf", b"lecture"),
                    RawEntry::file("Неделя 4/ДЗ/data.csv", b"a,b"),
                ],
                false,
            ),
        )
        .unwrap();
        let state = app.state::<AppState>();
        let candidate = register_picked_export(&state, &Picker(Some(archive.clone())))
            .unwrap()
            .unwrap();
        let preview =
            super::preview_candidate(&state, &candidate.id, &ZipLimits::default()).unwrap();
        let request = ExportImportRequest {
            candidate_id: candidate.id.clone(),
            archive_sha256: preview.archive_sha256.clone(),
            entries: preview
                .entries
                .iter()
                .map(|entry| ExportImportEntry {
                    entry_id: entry.entry_id.clone(),
                    subject_id: entry.classification.subject_id.clone(),
                    week_number: entry.classification.week_number,
                    category: entry.classification.category,
                    edited: false,
                })
                .collect(),
            course_url: Some("https://my.centraluniversity.ru/learn/courses/1".to_owned()),
            assignment_url: None,
        };
        let log = record(&app);
        let operation_id = Uuid::new_v4().to_string();
        let result =
            run_cu_import(app.handle(), &operation_id, &request, ZipLimits::default()).unwrap();
        assert_eq!(result.value.imported, 2);

        let events = log.lock().unwrap().clone();
        let order: Vec<&str> = events
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| *name != names::CU_IMPORT_PROGRESS && *name != names::OPERATION_PROGRESS)
            .collect();
        assert_eq!(
            order,
            vec![
                names::INBOX_FILE_ADDED,
                names::INBOX_UPDATED,
                names::OPERATION_COMPLETED,
                names::DOWNLOADS_WATCHER_UPDATED,
            ]
        );
        let phases: Vec<&str> = events
            .iter()
            .filter(|(name, _)| name == names::CU_IMPORT_PROGRESS)
            .map(|(_, payload)| payload["phase"].as_str().unwrap())
            .collect();
        assert_eq!(phases.first(), Some(&"inspect"));
        assert!(phases.contains(&"extract") && phases.contains(&"import"));
        assert_eq!(phases.last(), Some(&"cleanup"));
        assert!(events
            .iter()
            .filter(|(name, _)| name == names::OPERATION_PROGRESS)
            .all(
                |(_, payload)| payload["operationId"] == operation_id.as_str()
                    && payload["kind"] == "import"
            ));
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(
            !serialized.contains(root.to_str().unwrap()),
            "no absolute paths in events"
        );
        // Кандидат использован и снят с списка.
        assert!(watcher_snapshot(&state).unwrap().candidates.is_empty());

        let replay =
            run_cu_import(app.handle(), &operation_id, &request, ZipLimits::default()).unwrap();
        assert!(replay.value.replayed);
        assert_eq!(
            log.lock().unwrap().len(),
            events.len(),
            "replay emits nothing"
        );
    }

    #[test]
    fn picked_export_is_granted_and_cancel_registers_nothing() {
        let (app, _tmp, root) = app(false);
        let state = app.state::<AppState>();
        assert!(register_picked_export(&state, &Picker(None))
            .unwrap()
            .is_none());
        assert!(watcher_snapshot(&state).unwrap().candidates.is_empty());
        let file = root.join("external/export.pdf");
        fs::write(&file, b"%PDF").unwrap();
        let picked = register_picked_export(&state, &Picker(Some(file.clone())))
            .unwrap()
            .unwrap();
        assert_eq!(picked.name, "export.pdf");
        assert!(state.grants().is_granted(&file));
        let serialized = serde_json::to_string(&picked).unwrap();
        assert!(!serialized.contains(root.to_str().unwrap()));
        let directory = root.join("external");
        assert_eq!(
            register_picked_export(&state, &Picker(Some(directory)))
                .unwrap_err()
                .code,
            ErrorCode::Blocked
        );
    }

    #[test]
    fn watcher_requires_explicit_enable_and_a_downloads_folder() {
        let (app, _tmp, _root) = app(false);
        let state = app.state::<AppState>();
        let snapshot = watcher_snapshot(&state).unwrap();
        assert!(!snapshot.enabled);
        assert_eq!(snapshot.state, WatcherState::Disabled);
        let error = set_watcher_enabled(app.handle(), true).unwrap_err();
        assert_eq!(error.code, ErrorCode::Blocked);
        assert!(!watcher_snapshot(&state).unwrap().enabled);
        assert!(!state.watcher().is_running());

        let (app, _tmp, _root) = self::app(true);
        let state = app.state::<AppState>();
        let enabled = set_watcher_enabled(app.handle(), true).unwrap();
        assert!(enabled.enabled);
        assert_eq!(enabled.state, WatcherState::Watching);
        assert!(state.watcher().is_running());
        let disabled = set_watcher_enabled(app.handle(), false).unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.state, WatcherState::Disabled);
        assert!(!state.watcher().is_running());
    }

    #[test]
    fn lms_links_are_allowlisted_on_save_and_on_open() {
        let (app, _tmp, _root) = app(false);
        let state = app.state::<AppState>();
        assert_eq!(
            resolve_course_url(&state, Some("oba"), None)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            update_course_url(&state, "oba", Some("https://evil.example/x"))
                .unwrap_err()
                .code,
            ErrorCode::Blocked
        );
        let subject = update_course_url(
            &state,
            "oba",
            Some("https://my.centraluniversity.ru/learn/courses/view/actual/7"),
        )
        .unwrap();
        assert_eq!(
            subject.course_url.as_deref(),
            Some("https://my.centraluniversity.ru/learn/courses/view/actual/7")
        );
        assert_eq!(
            resolve_course_url(&state, Some("oba"), None).unwrap(),
            "https://my.centraluniversity.ru/learn/courses/view/actual/7"
        );
        assert_eq!(
            resolve_course_url(&state, Some("oba"), Some("f"))
                .unwrap_err()
                .code,
            ErrorCode::Blocked
        );
        assert_eq!(
            resolve_course_url(&state, None, None).unwrap_err().code,
            ErrorCode::Blocked
        );
        // Очистка ссылки.
        assert!(update_course_url(&state, "oba", Some(""))
            .unwrap()
            .course_url
            .is_none());
        assert_eq!(
            update_assignment_url(&state, "missing", None)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
        assert_eq!(
            resolve_assignment_url(&state, Some("missing"), None)
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn assignment_link_from_import_provenance_opens_only_after_revalidation() {
        let (app, _tmp, root) = app(false);
        let state = app.state::<AppState>();
        let pdf = root.join("external/ДЗ 5.pdf");
        fs::write(&pdf, b"%PDF hw").unwrap();
        let candidate = register_picked_export(&state, &Picker(Some(pdf)))
            .unwrap()
            .unwrap();
        let preview =
            super::preview_candidate(&state, &candidate.id, &ZipLimits::default()).unwrap();
        let request = ExportImportRequest {
            candidate_id: candidate.id,
            archive_sha256: preview.archive_sha256,
            entries: vec![ExportImportEntry {
                entry_id: "file".to_owned(),
                subject_id: Some("oba".to_owned()),
                week_number: Some(5),
                category: MaterialCategory::HomeworkTask,
                edited: true,
            }],
            course_url: None,
            assignment_url: Some(
                "https://my.centraluniversity.ru/learn/courses/1/tasks/99".to_owned(),
            ),
        };
        let result = run_cu_import(
            app.handle(),
            &Uuid::new_v4().to_string(),
            &request,
            ZipLimits::default(),
        )
        .unwrap();
        let file_id = match &result.value.items[0] {
            crate::intake::ImportItemReport::Imported { file, .. } => file.id.clone(),
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            resolve_assignment_url(&state, None, Some(&file_id)).unwrap(),
            "https://my.centraluniversity.ru/learn/courses/1/tasks/99"
        );
        assert_eq!(
            resolve_course_url(&state, None, Some(&file_id))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn manual_picker_import_records_manual_provenance() {
        let (app, _tmp, root) = app(false);
        let state = app.state::<AppState>();
        let file = root.join("external/notes.md");
        fs::write(&file, b"# notes").unwrap();
        state.grants().grant([file.clone()]);
        let roots = RootsService::new(state.database(), None)
            .open_import_roots()
            .unwrap();
        InboxImporter::new(state.database(), &roots, state.grants())
            .run(
                &Uuid::new_v4().to_string(),
                &[ImportItemInput {
                    source_path: file.to_string_lossy().into_owned(),
                    origin: ImportOrigin::Picker,
                }],
                &mut |_| {},
            )
            .unwrap();
        let files = ReadRepository::new(state.database()).inbox_files().unwrap();
        let provenance = files[0].provenance.as_ref().unwrap();
        assert_eq!(provenance.source, ProvenanceSource::Manual);
        assert_eq!(provenance.via, ProvenanceVia::Picker);
        assert!(provenance.archive_name.is_none());
    }
}
