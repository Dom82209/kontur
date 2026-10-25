//! Прикладной слой этапа 9: статусы провайдеров, подготовка передачи,
//! запуск официального CLI в фоне, отмена, ручная передача и вставка ответа.
//!
//! Правила:
//! - CLI используется только после явного подключения пользователем
//!   (`connect_provider`); найденный CLI сам по себе не включается;
//! - материалы уходят провайдеру только по явной команде `send` для уже
//!   подготовленного задания, текст которого пользователь видел целиком;
//! - ответ не применяется сам: он разбирается в черновик и ждёт
//!   подтверждения (`WorkflowService::confirm_ai_result`);
//! - учётные данные провайдеров не запрашиваются, не читаются и не хранятся.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::Database,
    dto::{AutomationRulesDto, ProviderId},
    error::{AppError, BackendError},
    events::names,
    repositories::load_automation_rules,
    workflow::{
        dto::{
            ProviderAdapterDto, ProviderStateDto, ProviderStatusDto, ReviewRunDto, ReviewRunStatus,
            RunKind, WorkflowRole,
        },
        repository,
        service::WorkflowEvent,
    },
};

use super::{
    adapters::{
        executable_name, probe, AdapterFactory, AdapterRequest, AiAdapter, CliAdapter, FailureKind,
        ProbeReport, RunControl,
    },
    context::{build_context, ContextInput, Section, SourceText, MAX_CONTEXT_CHARS},
    isolation::{
        IsolationReport, IsolationRunner, IsolationSelfTest, ProviderExecutionPolicy,
        ISOLATION_PROFILE_VERSION,
    },
    locate::{CliLocator, Located},
    model::{
        provider_label, AiIssueDto, AiJobDto, AiTask, JobStatus, LogLevel, ParseStatus,
        ProviderAvailability, ProviderCliDto, ProviderHandoffDto, ProviderIsolationDto,
        ResponseSource, Transport,
    },
    redact::redact,
    response::parse_response,
    store::{self, log_entry, FailureUpdate, NewJob, ProviderSetting, ResponseUpdate},
};

/// Предел ожидания ответа CLI.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Предел вставленного или импортированного ответа.
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Как пользователь хочет передать материалы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportPreference {
    /// CLI, если провайдер подключён; иначе ручная передача.
    Auto,
    Cli,
    ManualHandoff,
}

/// Получатель событий заданий. Production — `AppHandle`, тесты — запись.
pub trait AiEventSink: Send + Sync {
    fn emit(&self, event: WorkflowEvent);
}

fn event(name: &'static str, payload: impl Serialize) -> WorkflowEvent {
    WorkflowEvent {
        name,
        payload: serde_json::to_value(payload).unwrap_or(Value::Null),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiJobProgressPayload {
    pub job_id: String,
    pub run_id: String,
    pub homework_id: String,
    pub phase: &'static str,
    pub elapsed_ms: u64,
    pub output_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiJobUpdatedPayload {
    pub job_id: String,
    pub run_id: String,
    pub homework_id: String,
    pub status: JobStatus,
    pub parse_status: Option<ParseStatus>,
}

#[derive(Debug, Clone)]
struct CachedProbe {
    report: ProbeReport,
    checked_at: String,
}

/// Долгоживущее состояние провайдеров: поиск CLI, кэш проверки, флаги отмены
/// выполняющихся заданий. Только в памяти процесса.
pub struct ProviderRuntime {
    locator: CliLocator,
    work_root: PathBuf,
    probes: Mutex<HashMap<ProviderId, CachedProbe>>,
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    factory: Option<Arc<dyn AdapterFactory>>,
    timeout: Duration,
    apps_dir: PathBuf,
    runner: IsolationRunner,
    self_test: Mutex<Option<IsolationReport>>,
}

impl ProviderRuntime {
    pub fn new(app_data_dir: &Path, home: Option<&Path>) -> Self {
        Self {
            locator: CliLocator::system(home),
            work_root: app_data_dir.join("ai"),
            probes: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            factory: None,
            timeout: DEFAULT_TIMEOUT,
            apps_dir: PathBuf::from("/Applications"),
            runner: IsolationRunner::system(home),
            self_test: Mutex::new(None),
        }
    }

    /// Тесты: свой поиск CLI, фабрика fake-адаптеров, тайм-аут.
    pub fn for_tests(
        work_root: PathBuf,
        locator: CliLocator,
        factory: Option<Arc<dyn AdapterFactory>>,
        timeout: Duration,
    ) -> Self {
        let test_enabled = factory.is_some();
        let runner = IsolationRunner::system(locator.home());
        Self {
            apps_dir: work_root.join("Applications"),
            locator,
            work_root,
            probes: Mutex::new(HashMap::new()),
            running: Mutex::new(HashMap::new()),
            factory,
            timeout,
            runner,
            self_test: Mutex::new(Some(IsolationReport {
                platform: "macos".to_owned(),
                runner_available: true,
                profile_version: ISOLATION_PROFILE_VERSION,
                passed: test_enabled,
                checks: Vec::new(),
                checked_at: store::timestamp(),
                explanation: "Тестовый isolation runner".to_owned(),
            })),
        }
    }

    pub fn work_root(&self) -> &Path {
        &self.work_root
    }

    pub fn home(&self) -> Option<&Path> {
        self.locator.home()
    }

    fn prepare_work_root(&self) -> Result<PathBuf, AppError> {
        ensure_private_dir(&self.work_root)?;
        Ok(self.work_root.clone())
    }

    fn cached(&self, provider: ProviderId) -> Option<CachedProbe> {
        self.probes.lock().ok()?.get(&provider).cloned()
    }

    fn store_probe(&self, provider: ProviderId, report: ProbeReport) {
        if let Ok(mut probes) = self.probes.lock() {
            probes.insert(
                provider,
                CachedProbe {
                    report,
                    checked_at: store::timestamp(),
                },
            );
        }
    }

    fn cached_self_test(&self) -> Option<IsolationReport> {
        self.self_test.lock().ok()?.clone()
    }

    fn store_self_test(&self, report: IsolationReport) {
        if let Ok(mut current) = self.self_test.lock() {
            *current = Some(report);
        }
    }

    /// Отмечает, что CLI потребовал вход (по итогу реального запуска).
    fn mark_logged_out(&self, provider: ProviderId) {
        if let Ok(mut probes) = self.probes.lock() {
            if let Some(entry) = probes.get_mut(&provider) {
                entry.report.logged_in = Some(false);
            }
        }
    }

    fn register(&self, job_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut running) = self.running.lock() {
            running.insert(job_id.to_owned(), Arc::clone(&flag));
        }
        flag
    }

    fn unregister(&self, job_id: &str) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(job_id);
        }
    }

    fn signal_cancel(&self, job_id: &str) -> bool {
        self.running
            .lock()
            .ok()
            .and_then(|running| running.get(job_id).cloned())
            .map(|flag| flag.store(true, Ordering::SeqCst))
            .is_some()
    }
}

fn ensure_private_dir(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(BackendError::blocked(
                "Служебный каталог AI повреждён (не каталог или ссылка)",
            ))
        }
        Ok(_) => {}
        Err(_) => {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .map_err(|error| {
                    BackendError::io(format!("Служебный каталог AI не создан: {}", error.kind()))
                })?;
        }
    }
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    Ok(())
}

pub fn handoff_target(provider: ProviderId) -> (&'static [&'static str], &'static str) {
    match provider {
        // Официальное приложение Claude и claude.ai.
        ProviderId::Claude => (&["Claude.app"], "https://claude.ai/new"),
        // Официальные приложения OpenAI (Codex, ChatGPT) и chatgpt.com.
        ProviderId::Codex => (&["Codex.app", "ChatGPT.app"], "https://chatgpt.com/"),
    }
}

pub struct AiService<'a> {
    database: &'a Database,
    runtime: &'a ProviderRuntime,
}

impl<'a> AiService<'a> {
    pub fn new(database: &'a Database, runtime: &'a ProviderRuntime) -> Self {
        Self { database, runtime }
    }

    // ───────────── Провайдеры ─────────────

    fn tasks_of(rules: &AutomationRulesDto, provider: ProviderId) -> Vec<AiTask> {
        AiTask::ALL
            .iter()
            .copied()
            .filter(|task| rules.workflow.tasks.provider(*task) == provider)
            .collect()
    }

    fn roles_of(rules: &AutomationRulesDto, provider: ProviderId) -> Vec<WorkflowRole> {
        [
            (WorkflowRole::Analyst, rules.workflow.roles.analyst),
            (WorkflowRole::Author, rules.workflow.roles.author),
            (WorkflowRole::Reviewer, rules.workflow.roles.reviewer),
        ]
        .into_iter()
        .filter(|(_, assigned)| *assigned == provider)
        .map(|(role, _)| role)
        .collect()
    }

    /// Проверка CLI (`--version`, `--help`, статус входа). Выполняется по
    /// явному действию и лениво для уже подключённого провайдера.
    fn run_probe(&self, provider: ProviderId) -> Result<Option<ProbeReport>, AppError> {
        match self.runtime.locator.find(executable_name(provider)) {
            Located::Found(exe) => {
                let report = probe(
                    provider,
                    &exe,
                    &self.runtime.prepare_work_root()?,
                    &self.runtime.runner,
                );
                self.runtime.store_probe(provider, report.clone());
                Ok(Some(report))
            }
            _ => Ok(None),
        }
    }

    fn project(
        &self,
        provider: ProviderId,
        setting: ProviderSetting,
        rules: &AutomationRulesDto,
    ) -> ProviderStatusDto {
        let located = self.runtime.locator.find(executable_name(provider));
        let probe = self.runtime.cached(provider);
        let persisted = self.database.with_connection(store::read_settings).ok();
        let stored_report = persisted
            .as_ref()
            .and_then(|settings| settings.isolation_report.clone());
        let report_for_policy = self.runtime.cached_self_test().or(stored_report);
        let capability = self.runtime.runner.capability();
        let consent_valid = setting.consent_profile_version == Some(ISOLATION_PROFILE_VERSION)
            && report_for_policy.as_ref().is_some_and(|report| {
                report.passed && report.profile_version == ISOLATION_PROFILE_VERSION
            })
            && capability.available;
        let provider_proven = provider == ProviderId::Codex || self.runtime.factory.is_some();
        let effective_enabled = setting.cli_enabled && consent_valid && provider_proven;
        let (apps, web_url) = handoff_target(provider);
        let app_installed = apps
            .iter()
            .any(|app| self.runtime.apps_dir.join(app).is_dir());
        let (detected, executable, rejection) = match &located {
            Located::Found(exe) => (true, Some(exe.display.clone()), None),
            Located::Missing => (false, None, None),
            Located::Rejected(reason) => (false, None, Some(reason.clone())),
        };
        let report = probe.as_ref().map(|cached| &cached.report);
        let cli = ProviderCliDto {
            enabled: effective_enabled,
            detected,
            executable,
            version: report.and_then(|report| report.version.clone()),
            supported: report.map(|report| report.supported),
            missing_flags: report
                .map(|report| report.missing_flags.clone())
                .unwrap_or_default(),
            logged_in: report.and_then(|report| report.logged_in),
            checked_at: probe.as_ref().map(|cached| cached.checked_at.clone()),
        };
        let login_hint = match provider {
            ProviderId::Codex => "codex login",
            ProviderId::Claude => "claude auth login",
        };
        let isolation_reason = if !capability.available {
            capability.reason.clone()
        } else if provider == ProviderId::Claude && self.runtime.factory.is_none() {
            Some("Claude остаётся в ручном режиме: не доказано одновременное отключение пользовательских settings, hooks, MCP и slash-команд".to_owned())
        } else if report_for_policy.as_ref().is_none_or(|report| {
            !report.passed || report.profile_version != ISOLATION_PROFILE_VERSION
        }) {
            Some("Сначала выполните проверку файловой изоляции".to_owned())
        } else if setting.consent_profile_version != Some(ISOLATION_PROFILE_VERSION) {
            Some("После проверки явно включите изолированный автоматический режим".to_owned())
        } else {
            None
        };
        let (availability, state, adapter, error) = if !effective_enabled {
            (
                ProviderAvailability::Manual,
                ProviderStateDto::Disconnected,
                ProviderAdapterDto::ManualHandoff,
                isolation_reason.clone(),
            )
        } else if !detected {
            (
                ProviderAvailability::Unavailable,
                ProviderStateDto::Error,
                ProviderAdapterDto::Unavailable,
                Some(match rejection {
                    Some(reason) => format!("CLI отклонён проверкой безопасности: {reason}"),
                    None => format!(
                        "CLI {} не найден в каталогах установки — шаги выполняются ручной передачей",
                        executable_name(provider)
                    ),
                }),
            )
        } else {
            match report {
                None => (
                    ProviderAvailability::Error,
                    ProviderStateDto::Error,
                    ProviderAdapterDto::Unavailable,
                    Some("CLI ещё не проверен — нажмите «Проверить»".to_owned()),
                ),
                Some(report) if !report.supported || report.version.is_none() => (
                    ProviderAvailability::Error,
                    ProviderStateDto::Error,
                    ProviderAdapterDto::Unavailable,
                    report.error.clone(),
                ),
                Some(report)
                    if report.logged_in != Some(true)
                        && (provider == ProviderId::Codex || self.runtime.factory.is_none()) =>
                {
                    (
                    ProviderAvailability::Error,
                    ProviderStateDto::ReauthRequired,
                    ProviderAdapterDto::Cli,
                    Some(format!(
                        "Войдите в официальном CLI: выполните `{login_hint}` в Терминале, затем «Проверить»"
                    )),
                )
                }
                Some(_) => (
                    ProviderAvailability::Connected,
                    ProviderStateDto::Connected,
                    ProviderAdapterDto::Cli,
                    None,
                ),
            }
        };
        let label = match provider {
            ProviderId::Codex => "Codex CLI",
            ProviderId::Claude => "Claude Code CLI",
        };
        ProviderStatusDto {
            provider,
            state,
            account_label: (availability == ProviderAvailability::Connected).then(|| {
                match &cli.version {
                    Some(version) => format!("{label} · {version}"),
                    None => label.to_owned(),
                }
            }),
            last_checked_at: cli.checked_at.clone().unwrap_or_else(store::timestamp),
            error_message: error.map(|message| redact(&message, self.runtime.home())),
            adapter,
            roles: Self::roles_of(rules, provider),
            availability,
            tasks: Self::tasks_of(rules, provider),
            cli,
            isolation: ProviderIsolationDto {
                capability,
                report: report_for_policy,
                consent_profile_version: setting.consent_profile_version,
                consent_valid,
                execution_policy: if effective_enabled {
                    ProviderExecutionPolicy::IsolatedAutomatic
                } else {
                    ProviderExecutionPolicy::ManualOnly
                },
                blocking_reason: isolation_reason,
            },
            handoff: ProviderHandoffDto {
                app_installed,
                web_url: web_url.to_owned(),
            },
        }
    }

    pub fn status(&self, provider: ProviderId) -> Result<ProviderStatusDto, AppError> {
        let (settings, rules) = self.database.with_connection(|connection| {
            Ok((
                store::read_settings(connection)?,
                load_automation_rules(connection)?,
            ))
        })?;
        let setting = settings.get(provider);
        if setting.cli_enabled
            && setting.consent_profile_version == Some(ISOLATION_PROFILE_VERSION)
            && self.runtime.cached(provider).is_none()
        {
            // Подключённый ранее CLI проверяется один раз за сеанс приложения.
            self.run_probe(provider)?;
        }
        Ok(self.project(provider, setting, &rules))
    }

    pub fn statuses(&self) -> Result<Vec<ProviderStatusDto>, AppError> {
        [ProviderId::Codex, ProviderId::Claude]
            .into_iter()
            .map(|provider| self.status(provider))
            .collect()
    }

    /// Явная повторная проверка CLI.
    pub fn check(&self, provider: ProviderId) -> Result<ProviderStatusDto, AppError> {
        if !self.runtime.runner.capability().available {
            return Err(BackendError::blocked(
                "OS isolation недоступна; используйте ручную передачу",
            ));
        }
        self.run_probe(provider)?;
        self.status(provider)
    }

    pub fn check_isolation(&self) -> Result<IsolationReport, AppError> {
        ensure_private_dir(self.runtime.work_root())?;
        let report = IsolationSelfTest::new(&self.runtime.runner).run();
        self.runtime.store_self_test(report.clone());
        self.database.with_transaction(|transaction| {
            let mut settings = store::read_settings(transaction)?;
            settings.isolation_report = Some(report.clone());
            if !report.passed || report.profile_version != ISOLATION_PROFILE_VERSION {
                for provider in [ProviderId::Codex, ProviderId::Claude] {
                    let mut setting = settings.get(provider);
                    setting.cli_enabled = false;
                    setting.consent_profile_version = None;
                    settings.set(provider, setting);
                }
            }
            store::write_settings(transaction, &settings)
        })?;
        Ok(report)
    }

    /// Явное подключение официального CLI пользователем. Включается только
    /// найденный и поддерживаемый CLI; иначе — ошибка без изменения настроек.
    pub fn connect(&self, provider: ProviderId) -> Result<ProviderStatusDto, AppError> {
        let name = provider_label(provider);
        if provider == ProviderId::Claude && self.runtime.factory.is_none() {
            return Err(BackendError::blocked(
                "Claude доступен только вручную: установленная версия не доказала отключение settings, hooks, MCP и slash-команд",
            ));
        }
        let settings = self.database.with_connection(store::read_settings)?;
        let isolation_ok = settings.isolation_report.as_ref().is_some_and(|report| {
            report.passed && report.profile_version == ISOLATION_PROFILE_VERSION
        }) || self.runtime.factory.is_some();
        if !isolation_ok
            || !self.runtime.runner.capability().available && self.runtime.factory.is_none()
        {
            return Err(BackendError::blocked(
                "Сначала выполните успешную проверку файловой изоляции",
            ));
        }
        let Some(report) = self.run_probe(provider)? else {
            return Err(match self.runtime.locator.find(executable_name(provider)) {
                Located::Rejected(reason) => BackendError::blocked(format!(
                    "{name}: CLI найден, но отклонён проверкой безопасности ({reason})"
                )),
                _ => AppError::new(
                    crate::error::ErrorCode::ProviderUnavailable,
                    format!(
                        "{name}: официальный CLI не найден. Установите его и выполните вход в нём самом — \
                         до этого шаги выполняются ручной передачей"
                    ),
                ),
            });
        };
        if !report.supported {
            return Err(BackendError::blocked(format!(
                "{name}: {}",
                report
                    .error
                    .unwrap_or_else(|| "версия CLI не поддерживается".to_owned())
            )));
        }
        if provider == ProviderId::Codex && report.logged_in != Some(true) {
            return Err(BackendError::blocked(format!(
                "{name}: учётные данные не подтверждены внутри OS isolation. Выполните `codex login` в Терминале и повторите проверку; если ошибка сохраняется, используйте ручную передачу"
            )));
        }
        self.database.with_transaction(|transaction| {
            let mut settings = store::read_settings(transaction)?;
            settings.set(
                provider,
                ProviderSetting {
                    cli_enabled: true,
                    consent_profile_version: Some(ISOLATION_PROFILE_VERSION),
                },
            );
            store::write_settings(transaction, &settings)
        })?;
        self.status(provider)
    }

    pub fn disconnect(&self, provider: ProviderId) -> Result<ProviderStatusDto, AppError> {
        self.database.with_transaction(|transaction| {
            let mut settings = store::read_settings(transaction)?;
            settings.set(
                provider,
                ProviderSetting {
                    cli_enabled: false,
                    consent_profile_version: None,
                },
            );
            store::write_settings(transaction, &settings)
        })?;
        self.status(provider)
    }

    pub fn status_event(status: &ProviderStatusDto) -> WorkflowEvent {
        event(
            names::PROVIDER_STATUS_CHANGED,
            serde_json::json!({ "status": status }),
        )
    }

    // ───────────── Задания ─────────────

    pub fn jobs(&self, homework_id: &str) -> Result<Vec<AiJobDto>, AppError> {
        self.database.with_connection(|connection| {
            repository::homework_core(connection, homework_id)?;
            store::jobs_for_homework(connection, homework_id)
        })
    }

    pub fn job(&self, job_id: &str) -> Result<AiJobDto, AppError> {
        self.database
            .with_connection(|connection| store::job(connection, job_id))
    }

    /// Подготовка передачи: строит ровно тот текст, который получит провайдер,
    /// и сохраняет его неизменяемым. Ничего не отправляет.
    pub fn prepare(
        &self,
        run_id: &str,
        task: AiTask,
        preference: TransportPreference,
    ) -> Result<AiJobDto, AppError> {
        let rules = self.database.with_connection(load_automation_rules)?;
        let provider = rules.workflow.tasks.provider(task);
        let status = self.status(provider)?;
        let connected = status.availability == ProviderAvailability::Connected;
        let transport = match preference {
            TransportPreference::ManualHandoff => Transport::ManualHandoff,
            TransportPreference::Cli if !connected => {
                return Err(AppError::new(
                    crate::error::ErrorCode::ProviderUnavailable,
                    format!(
                        "{} CLI не подключён — используйте ручную передачу",
                        provider_label(provider)
                    ),
                ))
            }
            TransportPreference::Cli => Transport::Cli,
            TransportPreference::Auto if connected => Transport::Cli,
            TransportPreference::Auto => Transport::ManualHandoff,
        };
        let include_prompt = task == AiTask::MaterialAnalysis
            && rules.workflow.tasks.initial_prompt == rules.workflow.tasks.material_analysis;
        let job_id = Uuid::new_v4().to_string();
        self.database.with_transaction(|transaction| {
            let run = repository::run(transaction, run_id)?;
            validate_task(transaction, &run, task)?;
            if store::running_job_for_run(transaction, run_id)?.is_some() {
                return Err(BackendError::conflict(
                    "По этому запуску уже выполняется задание — дождитесь или остановите его",
                ));
            }
            let input = gather_context(transaction, &run, task, include_prompt)?;
            let built = build_context(&input);
            let sha = format!("{:x}", Sha256::digest(built.prompt.as_bytes()));
            // Прежние неподтверждённые передачи той же задачи заменяются новой.
            let mut statement = transaction.prepare(
                "SELECT id FROM ai_jobs WHERE run_id = ?1 AND task = ?2 AND status IN ('prepared', 'response_ready')",
            )?;
            let previous: Vec<String> = statement
                .query_map(params![run_id, task.as_db_str()], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            drop(statement);
            for id in previous {
                store::discard_job(transaction, &id)?;
            }
            let included = built.sources.iter().filter(|source| source.chars_included > 0).count();
            let mut log = vec![log_entry(
                LogLevel::Info,
                format!(
                    "Подготовлена передача «{}» для {}: {} источников, {} символов (лимит {}); способ — {}",
                    task.label(),
                    provider_label(provider),
                    included,
                    built.prompt.chars().count(),
                    input.limit_chars,
                    match transport {
                        Transport::Cli => "официальный CLI после подтверждения",
                        Transport::ManualHandoff => "ручная передача",
                    }
                ),
            )];
            if built.truncated {
                log.push(log_entry(
                    LogLevel::Warn,
                    "Часть материалов не вошла в лимит объёма — см. состав передачи",
                ));
            }
            store::insert_job(
                transaction,
                &NewJob {
                    id: &job_id,
                    homework_id: &run.homework_id,
                    run_id,
                    task,
                    provider,
                    transport,
                    adapter_label: None,
                    include_prompt,
                    prompt_text: &built.prompt,
                    prompt_sha256: &sha,
                    context: &built.sources,
                    context_truncated: built.truncated,
                    limit_chars: input.limit_chars,
                    issues: &built.warnings,
                    log: &log,
                },
            )?;
            if run.status.is_open() {
                store::set_run_progress(
                    transaction,
                    run_id,
                    match transport {
                        Transport::Cli => "waiting_for_provider",
                        Transport::ManualHandoff => "manual_handoff",
                    },
                    "Передача подготовлена — проверьте текст и подтвердите отправку",
                )?;
            }
            Ok(())
        })?;
        self.job(&job_id)
    }

    /// Ответ, вставленный пользователем или импортированный из файла.
    pub fn submit_response(
        &self,
        job_id: &str,
        text: &str,
        source: ResponseSource,
        file_name: Option<&str>,
    ) -> Result<AiJobDto, AppError> {
        if source == ResponseSource::Cli {
            return Err(BackendError::blocked("Ответ CLI сохраняет только backend"));
        }
        if text.len() > MAX_RESPONSE_BYTES {
            return Err(BackendError::blocked(
                "Ответ больше 2 МБ — сократите его или передайте файлом поменьше",
            ));
        }
        let file_name = file_name.map(safe_file_name);
        self.database.with_transaction(|transaction| {
            let (job, include_prompt) = store::job_with_flags(transaction, job_id)?;
            if !matches!(job.status, JobStatus::Prepared | JobStatus::ResponseReady) {
                return Err(BackendError::conflict(
                    "Задание уже закрыто или выполняется — подготовьте передачу заново",
                ));
            }
            let parsed = parse_response(job.task, text, false, include_prompt);
            let message = match source {
                ResponseSource::File => format!(
                    "Ответ импортирован из файла «{}»: {} символов; разбор — {}",
                    file_name.as_deref().unwrap_or("файл"),
                    text.chars().count(),
                    parse_label(parsed.status)
                ),
                _ => format!(
                    "Ответ вставлен вручную: {} символов; разбор — {}",
                    text.chars().count(),
                    parse_label(parsed.status)
                ),
            };
            store::save_response(
                transaction,
                job_id,
                &ResponseUpdate {
                    text,
                    source,
                    file_name: file_name.as_deref(),
                    truncated: false,
                    parse_status: parsed.status,
                    draft: &parsed.draft,
                    issues: &parsed.issues,
                    exit_code: None,
                    duration_ms: None,
                    error: None,
                    log: &[log_entry(
                        LogLevel::Info,
                        redact(&message, self.runtime.home()),
                    )],
                },
            )?;
            store::set_run_progress(
                transaction,
                &job.run_id,
                "manual_handoff",
                "Ответ получен — проверьте и подтвердите результат",
            )?;
            Ok(())
        })?;
        self.job(job_id)
    }

    pub fn discard(&self, job_id: &str) -> Result<AiJobDto, AppError> {
        self.database.with_transaction(|transaction| {
            let job = store::job(transaction, job_id)?;
            store::discard_job(transaction, job_id)?;
            store::set_run_progress(
                transaction,
                &job.run_id,
                "waiting_for_provider",
                "Ответ отклонён — подготовьте передачу заново или введите результат вручную",
            )
        })?;
        self.job(job_id)
    }

    /// prepared → running. Возвращает адаптер и флаг отмены для фонового запуска.
    pub fn begin_send(&self, job_id: &str) -> Result<SendPlan, AppError> {
        let job = self.job(job_id)?;
        if job.status != JobStatus::Prepared {
            return Err(BackendError::conflict("Задание уже отправлено или закрыто"));
        }
        if job.transport != Transport::Cli {
            return Err(BackendError::blocked(
                "Это задание ручной передачи: скопируйте промт и вставьте ответ",
            ));
        }
        let status = self.status(job.provider)?;
        if status.availability != ProviderAvailability::Connected {
            return Err(AppError::new(
                crate::error::ErrorCode::ProviderUnavailable,
                status.error_message.unwrap_or_else(|| {
                    format!("{} CLI не подключён", provider_label(job.provider))
                }),
            ));
        }
        let adapter = self.adapter(job.provider)?;
        let label = adapter.label();
        let prompt = self.database.with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT prompt_text FROM ai_jobs WHERE id = ?1",
                [job_id],
                |row| row.get::<_, String>(0),
            )?)
        })?;
        self.database.with_transaction(|transaction| {
            let run = repository::run(transaction, &job.run_id)?;
            if job.task != AiTask::RevisionPrompt && !run.status.is_open() {
                return Err(BackendError::conflict("Запуск уже завершён или отменён"));
            }
            store::mark_running(
                transaction,
                job_id,
                &job.run_id,
                &label,
                log_entry(
                    LogLevel::Info,
                    format!(
                        "Отправлено пользователем в {label}: {} символов через stdin, рабочий каталог — пустой каталог задания",
                        job.prompt_chars
                    ),
                ),
            )
        })?;
        let cancel = self.runtime.register(job_id);
        Ok(SendPlan {
            job: self.job(job_id)?,
            adapter,
            prompt,
            cancel,
        })
    }

    fn adapter(&self, provider: ProviderId) -> Result<Arc<dyn AiAdapter>, AppError> {
        if let Some(factory) = &self.runtime.factory {
            return factory.adapter(provider);
        }
        let Located::Found(executable) = self.runtime.locator.find(executable_name(provider))
        else {
            return Err(AppError::new(
                crate::error::ErrorCode::ProviderUnavailable,
                format!("{} CLI не найден", provider_label(provider)),
            ));
        };
        let cached = self.runtime.cached(provider);
        let report = cached.as_ref().map(|cached| &cached.report);
        Ok(Arc::new(CliAdapter {
            provider,
            executable,
            version: report.and_then(|report| report.version.clone()),
            optional_flags: report
                .map(|report| report.optional_flags.clone())
                .unwrap_or_default(),
            home: self.runtime.home().map(Path::to_path_buf),
            runner: self.runtime.runner.clone(),
        }))
    }

    /// Тело фонового запуска: процесс CLI → разбор → `response_ready` или
    /// ошибка. Ответ не применяется до подтверждения пользователем.
    pub fn execute(&self, plan: SendPlan, sink: &dyn AiEventSink) {
        let SendPlan {
            job,
            adapter,
            prompt,
            cancel,
        } = plan;
        let progress = |phase: &'static str, elapsed_ms: u64, output_bytes: u64| {
            sink.emit(event(
                names::AI_JOB_PROGRESS,
                AiJobProgressPayload {
                    job_id: job.id.clone(),
                    run_id: job.run_id.clone(),
                    homework_id: job.homework_id.clone(),
                    phase,
                    elapsed_ms,
                    output_bytes,
                },
            ))
        };
        progress("started", 0, 0);
        let operation_id = Uuid::new_v4().to_string();
        let _ = self.database.with_transaction(|transaction| {
            start_operation(
                transaction,
                &operation_id,
                &format!("{}: {}", provider_label(job.provider), job.task.label()),
                &job.homework_id,
            )
        });

        let workdir = ensure_private_dir(self.runtime.work_root())
            .ok()
            .and_then(|()| {
                tempfile::Builder::new()
                    .prefix(".kontur-ai-")
                    .tempdir_in(self.runtime.work_root())
                    .ok()
            });
        let result = match &workdir {
            Some(dir) => {
                let mut on_tick = |tick: super::process::Tick| {
                    progress(
                        "running",
                        tick.elapsed.as_millis() as u64,
                        tick.stdout_bytes,
                    )
                };
                adapter.execute(
                    &AdapterRequest {
                        prompt: &prompt,
                        workdir: dir.path(),
                        timeout: self.runtime.timeout,
                    },
                    RunControl {
                        cancel: &cancel,
                        on_tick: &mut on_tick,
                    },
                )
            }
            None => Err(super::adapters::AdapterFailure {
                kind: FailureKind::Spawn,
                message: "Не удалось создать рабочий каталог задания".to_owned(),
                exit_code: None,
                stderr_excerpt: String::new(),
                duration_ms: 0,
                partial_text: None,
            }),
        };
        // Рабочий каталог удаляется сразу: в нём нет ничего, кроме вывода CLI.
        drop(workdir);
        progress("parsing", 0, 0);

        let home = self.runtime.home();
        let outcome = self.database.with_transaction(|transaction| {
            let (current, include_prompt) = store::job_with_flags(transaction, &job.id)?;
            match result {
                Ok(response) => {
                    let parsed = parse_response(job.task, &response.text, response.truncated, include_prompt);
                    let mut log = vec![log_entry(
                        LogLevel::Info,
                        format!(
                            "Процесс завершён с кодом 0 за {} с; ответ {} символов; разбор — {}",
                            response.duration_ms / 1000,
                            response.text.chars().count(),
                            parse_label(parsed.status)
                        ),
                    )];
                    log.extend(response.notes.iter().map(|note| log_entry(LogLevel::Warn, note.clone())));
                    if !response.stderr_excerpt.trim().is_empty() {
                        log.push(log_entry(
                            LogLevel::Info,
                            format!("stderr (с редакцией): {}", response.stderr_excerpt.trim()),
                        ));
                    }
                    store::save_response(
                        transaction,
                        &current.id,
                        &ResponseUpdate {
                            text: &response.text,
                            source: ResponseSource::Cli,
                            file_name: None,
                            truncated: response.truncated,
                            parse_status: parsed.status,
                            draft: &parsed.draft,
                            issues: &parsed.issues,
                            exit_code: response.exit_code,
                            duration_ms: Some(response.duration_ms),
                            error: None,
                            log: &log,
                        },
                    )?;
                    store::set_run_progress(
                        transaction,
                        &job.run_id,
                        "waiting_for_provider",
                        "Ответ получен — проверьте и подтвердите результат",
                    )?;
                    finish_operation(transaction, &operation_id, true)?;
                    Ok(None)
                }
                Err(failure) => {
                    let mut log = vec![log_entry(
                        if failure.kind == FailureKind::Cancelled { LogLevel::Info } else { LogLevel::Error },
                        format!(
                            "{} (код выхода {}, {} с)",
                            failure.message,
                            failure.exit_code.map_or("—".to_owned(), |code| code.to_string()),
                            failure.duration_ms / 1000
                        ),
                    )];
                    if !failure.stderr_excerpt.trim().is_empty() {
                        log.push(log_entry(
                            LogLevel::Info,
                            format!("stderr (с редакцией): {}", failure.stderr_excerpt.trim()),
                        ));
                    }
                    match (&failure.kind, &failure.partial_text) {
                        (FailureKind::Cancelled, _) | (_, None) => {
                            store::fail_job(
                                transaction,
                                &current.id,
                                &FailureUpdate {
                                    status: if failure.kind == FailureKind::Cancelled {
                                        JobStatus::Cancelled
                                    } else {
                                        JobStatus::Failed
                                    },
                                    code: failure.kind.code(),
                                    message: &failure.message,
                                    exit_code: failure.exit_code,
                                    duration_ms: Some(failure.duration_ms),
                                    log: &log,
                                },
                            )?;
                            store::set_run_progress(
                                transaction,
                                &job.run_id,
                                "waiting_for_provider",
                                if failure.kind == FailureKind::Cancelled {
                                    "Запуск остановлен — подготовьте передачу заново или введите результат вручную"
                                } else {
                                    "Запуск не удался — повторите, передайте вручную или введите результат сами"
                                },
                            )?;
                        }
                        (_, Some(partial)) => {
                            // Частичный вывод сохраняется: пользователь решает, использовать ли его.
                            let mut parsed = parse_response(job.task, partial, true, include_prompt);
                            if parsed.status == ParseStatus::Valid {
                                parsed.status = ParseStatus::Partial;
                            }
                            parsed.issues.insert(
                                0,
                                AiIssueDto::new(
                                    "process_failed",
                                    format!("{} — ответ может быть неполным", failure.message),
                                ),
                            );
                            store::save_response(
                                transaction,
                                &current.id,
                                &ResponseUpdate {
                                    text: partial,
                                    source: ResponseSource::Cli,
                                    file_name: None,
                                    truncated: true,
                                    parse_status: parsed.status,
                                    draft: &parsed.draft,
                                    issues: &parsed.issues,
                                    exit_code: failure.exit_code,
                                    duration_ms: Some(failure.duration_ms),
                                    error: Some((failure.kind.code(), &failure.message)),
                                    log: &log,
                                },
                            )?;
                            store::set_run_progress(
                                transaction,
                                &job.run_id,
                                "waiting_for_provider",
                                "Получен частичный ответ — проверьте его или повторите запуск",
                            )?;
                        }
                    }
                    finish_operation(transaction, &operation_id, false)?;
                    Ok(Some(failure.kind))
                }
            }
        });
        self.runtime.unregister(&job.id);
        if let Ok(Some(FailureKind::AuthRequired)) = outcome {
            self.runtime.mark_logged_out(job.provider);
            if let Ok(status) = self.status(job.provider) {
                sink.emit(Self::status_event(&status));
            }
        }
        if let Err(error) = &outcome {
            // Запись итога не удалась (например, база недоступна): журнал без секретов.
            let message = redact(&error.message, home);
            let _ = self.database.with_transaction(|transaction| {
                store::fail_job(
                    transaction,
                    &job.id,
                    &FailureUpdate {
                        status: JobStatus::Failed,
                        code: "save_failed",
                        message: "Результат запуска не сохранён",
                        exit_code: None,
                        duration_ms: None,
                        log: &[log_entry(LogLevel::Error, message.clone())],
                    },
                )
            });
        }
        let current = self.job(&job.id).ok();
        sink.emit(event(
            names::AI_JOB_UPDATED,
            AiJobUpdatedPayload {
                job_id: job.id.clone(),
                run_id: job.run_id.clone(),
                homework_id: job.homework_id.clone(),
                status: current.as_ref().map_or(JobStatus::Failed, |job| job.status),
                parse_status: current.and_then(|job| job.parse_status),
            },
        ));
    }

    /// Остановка выполняющегося задания. `false` — задание не выполняется.
    pub fn cancel(&self, job_id: &str) -> Result<bool, AppError> {
        let job = self.job(job_id)?;
        if job.status != JobStatus::Running {
            return Ok(false);
        }
        if self.runtime.signal_cancel(job_id) {
            return Ok(true);
        }
        // Процесса уже нет (например, после перезапуска): закрыть задание честно.
        self.database.with_transaction(|transaction| {
            store::fail_job(
                transaction,
                job_id,
                &FailureUpdate {
                    status: JobStatus::Cancelled,
                    code: "cancelled",
                    message: "Запуск остановлен пользователем",
                    exit_code: None,
                    duration_ms: None,
                    log: &[log_entry(LogLevel::Info, "Запуск остановлен пользователем")],
                },
            )?;
            store::set_run_progress(
                transaction,
                &job.run_id,
                "waiting_for_provider",
                "Запуск остановлен",
            )
        })?;
        Ok(true)
    }

    /// При отмене запуска рабочего цикла останавливаются его задания.
    pub fn cancel_for_run(&self, run_id: &str) -> Result<(), AppError> {
        let ids: Vec<(String, String)> = self.database.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, status FROM ai_jobs WHERE run_id = ?1 AND status IN ('prepared', 'running', 'response_ready')",
            )?;
            let rows = statement
                .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        for (id, status) in ids {
            if status == "running" {
                self.cancel(&id)?;
            } else {
                self.database
                    .with_transaction(|transaction| store::discard_job(transaction, &id))?;
            }
        }
        Ok(())
    }

    pub fn updated_event(job: &AiJobDto) -> WorkflowEvent {
        event(
            names::AI_JOB_UPDATED,
            AiJobUpdatedPayload {
                job_id: job.id.clone(),
                run_id: job.run_id.clone(),
                homework_id: job.homework_id.clone(),
                status: job.status,
                parse_status: job.parse_status,
            },
        )
    }
}

pub struct SendPlan {
    pub job: AiJobDto,
    pub adapter: Arc<dyn AiAdapter>,
    pub prompt: String,
    pub cancel: Arc<AtomicBool>,
}

fn parse_label(status: ParseStatus) -> &'static str {
    match status {
        ParseStatus::Valid => "полный",
        ParseStatus::Partial => "частичный",
        ParseStatus::Corrupt => "не распознан",
        ParseStatus::Empty => "пустой",
    }
}

/// Имя импортированного файла — только последний компонент, без управляющих символов.
fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.trim().is_empty() {
        "ответ.txt".to_owned()
    } else {
        clean
    }
}

fn start_operation(
    connection: &Connection,
    id: &str,
    title: &str,
    homework_id: &str,
) -> Result<(), AppError> {
    connection.execute(
        "INSERT INTO operations(id, kind, status, progress, undoable, started_at, title, detail, paths_json, homework_id)
         VALUES (?1, 'ai_run', 'running', 0, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?2,
                 'Официальный CLI провайдера', '[]', ?3)",
        params![id, title, homework_id],
    )?;
    Ok(())
}

fn finish_operation(connection: &Connection, id: &str, ok: bool) -> Result<(), AppError> {
    connection.execute(
        "UPDATE operations SET status = ?2, progress = 1, completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                detail = ?3
         WHERE id = ?1",
        params![
            id,
            if ok { "completed" } else { "failed" },
            if ok {
                "Ответ получен — ждёт подтверждения пользователем"
            } else {
                "Запуск не завершён — см. журнал задания"
            }
        ],
    )?;
    Ok(())
}

// ───────────────────────── Состав передачи ─────────────────────────

fn validate_task(
    connection: &Connection,
    run: &ReviewRunDto,
    task: AiTask,
) -> Result<(), AppError> {
    let open = run.status.is_open();
    let ok = match task {
        AiTask::MaterialAnalysis | AiTask::InitialPrompt => {
            run.kind == RunKind::AnalyzeMaterials && open
        }
        AiTask::SolutionReview => {
            matches!(run.kind, RunKind::ReviewVersion | RunKind::RevisionPrompt) && open
        }
        AiTask::TeacherReview => run.kind == RunKind::ReviewTeacher && open,
        AiTask::RevisionPrompt => {
            run.kind != RunKind::AnalyzeMaterials
                && run.status == ReviewRunStatus::Completed
                && run.result.is_some()
        }
    };
    if !ok {
        return Err(BackendError::blocked(match task {
            AiTask::RevisionPrompt => {
                "Промт исправлений готовится по завершённой проверке".to_owned()
            }
            _ => format!(
                "Задача «{}» недоступна для этого запуска (он завершён или другого вида)",
                task.label()
            ),
        }));
    }
    let homework = repository::homework_core(connection, &run.homework_id)?;
    if homework.workflow_stage == crate::domain::HomeworkWorkflowStage::Submitted {
        return Err(BackendError::immutable(
            "Работа отправлена — снимок неизменяем",
        ));
    }
    Ok(())
}

fn extraction_text(
    connection: &Connection,
    extraction_id: &str,
) -> Result<Option<(String, String, String)>, AppError> {
    Ok(connection
        .query_row(
            "SELECT text, sha256, status FROM extractions WHERE id = ?1",
            [extraction_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?)
}

fn gather_context(
    connection: &Connection,
    run: &ReviewRunDto,
    task: AiTask,
    include_prompt: bool,
) -> Result<ContextInput, AppError> {
    let homework = repository::homework_dto(connection, &run.homework_id)?;
    let subject: String = connection
        .query_row(
            "SELECT name FROM subjects WHERE id = ?1",
            [&homework.subject_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_default();
    let bundle_id = run
        .source_bundle_id
        .as_deref()
        .ok_or_else(|| BackendError::blocked("У запуска нет снимка источников"))?;
    let (bundle, changes) = repository::bundle(connection, bundle_id)?;
    if task != AiTask::RevisionPrompt && changes.iter().any(|changed| *changed) {
        return Err(BackendError::blocked(
            "Материалы изменились после снимка — создайте набор источников заново",
        ));
    }
    let mut sources = Vec::new();
    for item in bundle.items.iter().filter(|item| item.included) {
        let missing = || {
            BackendError::blocked(format!(
                "«{}»: результат извлечения текста удалён или устарел — извлеките текст и создайте набор заново",
                item.file.name
            ))
        };
        let extraction_id = item.extraction_id.as_deref().ok_or_else(missing)?;
        let (text, sha, status) =
            extraction_text(connection, extraction_id)?.ok_or_else(missing)?;
        if item.sha256.as_deref() != Some(sha.as_str())
            || !matches!(status.as_str(), "complete" | "partial")
        {
            return Err(missing());
        }
        sources.push(SourceText {
            source_id: Some(item.id.clone()),
            name: item.file.name.clone(),
            kind: item.kind.as_db_str().to_owned(),
            sha256: item.sha256.clone(),
            text,
        });
    }

    let version = run
        .version_id
        .as_deref()
        .and_then(|id| homework.versions.iter().find(|version| version.id == id));
    if matches!(
        task,
        AiTask::SolutionReview | AiTask::TeacherReview | AiTask::RevisionPrompt
    ) {
        let version = version.ok_or_else(|| BackendError::database("review run has no version"))?;
        let mut ids: Vec<&str> = version.files.iter().map(|file| file.id.as_str()).collect();
        if !ids.contains(&version.source_file_id.as_str()) {
            ids.insert(0, version.source_file_id.as_str());
        }
        let mut found = false;
        for file_id in ids {
            let row: Option<(String, String, String, String)> = connection
                .query_row(
                    "SELECT e.text, e.sha256, e.status, coalesce(f.name, ?2)
                     FROM material_extractions m
                     JOIN extractions e ON e.id = m.extraction_id
                     LEFT JOIN files f ON f.id = m.material_id
                     WHERE m.material_kind = 'file' AND m.material_id = ?1",
                    params![file_id, version.file_name],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            if let Some((text, sha, status, name)) = row {
                if matches!(status.as_str(), "complete" | "partial") {
                    found = true;
                    sources.insert(
                        0,
                        SourceText {
                            source_id: None,
                            name,
                            kind: "solution".to_owned(),
                            sha256: Some(sha),
                            text,
                        },
                    );
                }
            }
        }
        if !found && task != AiTask::RevisionPrompt {
            return Err(BackendError::blocked(format!(
                "Текст версии {} не извлечён — откройте «Материалы» и извлеките текст файла «{}»",
                version.label, version.file_name
            )));
        }
    }

    let mut sections = Vec::new();
    match task {
        AiTask::InitialPrompt => {
            let summary: Option<String> = connection
                .query_row(
                    "SELECT json_extract(draft_json, '$.summary') FROM ai_jobs
                     WHERE run_id = ?1 AND task = 'material_analysis' AND status = 'response_ready'
                     ORDER BY created_at DESC LIMIT 1",
                    [&run.id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            let summary = summary
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| {
                    BackendError::blocked(
                        "Сначала получите итог анализа материалов — промт строится по нему",
                    )
                })?;
            sections.push(Section {
                title: "Итог анализа материалов (черновик, ещё не подтверждён)".to_owned(),
                body: summary,
            });
        }
        AiTask::TeacherReview | AiTask::RevisionPrompt => {
            let comments = repository::teacher_comments(connection, &run.homework_id)?;
            if !comments.is_empty() {
                let body = comments
                    .iter()
                    .enumerate()
                    .map(|(index, comment)| {
                        format!(
                            "{}. Вопрос: {}\n   Ответ преподавателя: {}",
                            index + 1,
                            comment.question,
                            comment.answer
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                sections.push(Section {
                    title: "Комментарии преподавателя".to_owned(),
                    body,
                });
            } else if task == AiTask::TeacherReview {
                return Err(BackendError::blocked(
                    "Добавьте комментарии преподавателя — без них проверку по ним не провести",
                ));
            }
            if task == AiTask::RevisionPrompt {
                sections.push(Section {
                    title: "Проблемы проверки, которые нужно устранить".to_owned(),
                    body: findings_text(run),
                });
            }
        }
        _ => {}
    }

    Ok(ContextInput {
        task,
        homework_title: homework.title.clone(),
        subject,
        version_label: version.map(|version| version.label.clone()),
        include_prompt,
        style_threshold: run.style_threshold,
        sections,
        sources,
        limit_chars: MAX_CONTEXT_CHARS,
    })
}

fn findings_text(run: &ReviewRunDto) -> String {
    let Some(result) = &run.result else {
        return String::new();
    };
    let mut lines = vec![format!("Итог проверки: {}", result.summary)];
    let mut push = |title: &str, items: Vec<String>| {
        if !items.is_empty() {
            lines.push(format!("\n{title}"));
            lines.extend(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| format!("{}. {text}", index + 1)),
            );
        }
    };
    let describe = |text: &str, location: Option<&str>| match location {
        Some(location) => format!("{text} ({location})"),
        None => text.to_owned(),
    };
    push(
        "Критические ошибки:",
        result
            .critical
            .iter()
            .map(|item| describe(&item.text, item.location.as_deref()))
            .collect(),
    );
    push(
        "Замечания:",
        result
            .remarks
            .iter()
            .map(|item| describe(&item.text, item.location.as_deref()))
            .collect(),
    );
    push(
        "Утверждения без опоры на материалы:",
        result
            .unsupported_claims
            .iter()
            .map(|claim| describe(&claim.text, Some(&claim.location)))
            .collect(),
    );
    push(
        "Не хватает обязательных файлов или ссылок:",
        result
            .missing
            .iter()
            .filter(|item| item.required)
            .map(|item| item.label.clone())
            .collect(),
    );
    push(
        "Невыполненные требования преподавателя:",
        result
            .teacher_requirements
            .iter()
            .filter(|item| item.status == crate::domain::RequirementStatus::NotDone)
            .map(|item| item.text.clone())
            .collect(),
    );
    lines.join("\n")
}
