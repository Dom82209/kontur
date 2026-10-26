//! Сценарные тесты этапа 9. Сетевых вызовов нет: адаптеры — fake-реализации
//! в памяти или фиктивный CLI-скрипт в TempDir. In-memory SQLite.

use std::{
    collections::VecDeque,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tempfile::TempDir;
use uuid::Uuid;

use crate::{
    db::Database,
    dto::ProviderId,
    error::ErrorCode,
    events::names,
    workflow::{
        dto::{PromptKind, PromptStatus, Provenance, ReviewRunStatus, RunKind},
        repository,
        service::{
            AiConfirmInput, CommandMeta, ManualAnalysisInput, ManualReviewInput, RunMode,
            VersionInput, WorkflowEvent, WorkflowService,
        },
        tests::{seeded, FakeVerifier},
    },
};

use super::{
    adapters::{
        AdapterFactory, AdapterFailure, AdapterRequest, AdapterResponse, AdapterResult, AiAdapter,
        FailureKind, RunControl,
    },
    locate::CliLocator,
    model::{AiTask, JobStatus, ParseStatus, ProviderAvailability, ResponseSource, Transport},
    process::Tick,
    service::{AiEventSink, AiService, ProviderRuntime, TransportPreference},
    store,
};

const HW: &str = "hw-1";

fn op() -> CommandMeta {
    CommandMeta {
        operation_id: Uuid::new_v4().to_string(),
        expected_revision: None,
    }
}

// ───────────────────────── Fake-адаптер ─────────────────────────

enum Behavior {
    Respond(String),
    Fail {
        kind: FailureKind,
        partial: Option<String>,
        stderr: String,
    },
    /// Ждёт отмены, сообщая прогресс.
    Hang,
}

#[derive(Default)]
struct FakeAdapter {
    script: Mutex<VecDeque<Behavior>>,
    prompts: Mutex<Vec<String>>,
    calls: AtomicUsize,
}

impl FakeAdapter {
    fn push(&self, behavior: Behavior) {
        self.script.lock().unwrap().push_back(behavior);
    }
}

impl AiAdapter for FakeAdapter {
    fn label(&self) -> String {
        "Fake CLI · test".to_owned()
    }

    fn execute(&self, request: &AdapterRequest<'_>, control: RunControl<'_>) -> AdapterResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(request.prompt.to_owned());
        assert!(
            request.workdir.is_dir(),
            "isolated workdir exists during the run"
        );
        assert_eq!(
            fs::read_dir(request.workdir).unwrap().count(),
            0,
            "workdir starts empty"
        );
        let behavior = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Behavior::Respond("{}".into()));
        match behavior {
            Behavior::Respond(text) => Ok(AdapterResponse {
                text,
                truncated: false,
                exit_code: Some(0),
                stderr_excerpt: String::new(),
                duration_ms: 5,
                notes: Vec::new(),
            }),
            Behavior::Fail {
                kind,
                partial,
                stderr,
            } => Err(AdapterFailure {
                kind,
                message: "CLI завершился с кодом 1".into(),
                exit_code: Some(1),
                stderr_excerpt: super::redact::redact(&stderr, Some(Path::new("/Users/student"))),
                duration_ms: 7,
                partial_text: partial,
            }),
            Behavior::Hang => {
                let started = Instant::now();
                while !control.cancel.load(Ordering::SeqCst) {
                    (control.on_tick)(Tick {
                        elapsed: started.elapsed(),
                        stdout_bytes: 10,
                        stderr_bytes: 0,
                    });
                    assert!(
                        started.elapsed() < Duration::from_secs(10),
                        "cancel never arrived"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(AdapterFailure {
                    kind: FailureKind::Cancelled,
                    message: "Запуск остановлен пользователем".into(),
                    exit_code: None,
                    stderr_excerpt: String::new(),
                    duration_ms: started.elapsed().as_millis() as u64,
                    partial_text: None,
                })
            }
        }
    }
}

struct FakeFactory(Arc<FakeAdapter>);

impl AdapterFactory for FakeFactory {
    fn adapter(&self, _provider: ProviderId) -> Result<Arc<dyn AiAdapter>, crate::error::AppError> {
        Ok(Arc::clone(&self.0) as Arc<dyn AiAdapter>)
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<WorkflowEvent>>);

impl AiEventSink for Recorder {
    fn emit(&self, event: WorkflowEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Recorder {
    fn names(&self) -> Vec<&'static str> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.name)
            .collect()
    }
}

/// Фиктивный официальный CLI (только проверка возможностей: версия, флаги, вход).
const FAKE_CODEX: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "codex-cli 0.99.0";;
  login) exit 0;;
  exec) echo "Usage: codex exec [OPTIONS]"; echo "Prompt from stdin"; echo "  --sandbox <MODE> read-only"; echo "  --ephemeral"; echo "  --ignore-user-config"; echo "  --ignore-rules"; echo "  --strict-config"; echo "  --skip-git-repo-check"; echo "  --output-last-message <FILE>";;
esac
"#;

const FAKE_CLAUDE: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "2.1.0 (Claude Code)";;
  --help) echo "  -p, --print"; echo "  --output-format <f>"; echo "  --max-turns <n>"; echo "  --tools <t>";;
esac
"#;

struct Env {
    database: Database,
    verifier: FakeVerifier,
    runtime: ProviderRuntime,
    fake: Arc<FakeAdapter>,
    home: PathBuf,
    bin: PathBuf,
    _tmp: TempDir,
}

fn install(bin: &Path, name: &str, body: &str) {
    let path = bin.join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn env() -> Env {
    let tmp = TempDir::new().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let bin = root.join("prefix/bin");
    let home = root.join("home");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&home).unwrap();
    let fake = Arc::new(FakeAdapter::default());
    let runtime = ProviderRuntime::for_tests(
        root.join("data/ai"),
        CliLocator::with_dirs(
            vec![bin.clone()],
            vec![root.join("prefix")],
            Some(home.clone()),
        ),
        Some(Arc::new(FakeFactory(Arc::clone(&fake)))),
        Duration::from_secs(5),
    );
    Env {
        database: seeded(),
        verifier: FakeVerifier::default(),
        runtime,
        fake,
        home,
        bin,
        _tmp: tmp,
    }
}

impl Env {
    fn ai(&self) -> AiService<'_> {
        AiService::new(&self.database, &self.runtime)
    }

    fn workflow(&self) -> WorkflowService<'_> {
        WorkflowService::new(&self.database, &self.verifier)
    }

    fn connect_codex(&self) {
        install(&self.bin, "codex", FAKE_CODEX);
        fs::write(self.home.join("logged-in"), "").unwrap();
        let status = self.ai().connect(ProviderId::Codex).unwrap();
        assert_eq!(
            status.availability,
            ProviderAvailability::Connected,
            "{status:?}"
        );
    }

    fn start(&self, kind: RunKind, version: Option<&str>) -> String {
        let bundle = self
            .workflow()
            .create_source_bundle(&op(), HW, kind, version)
            .unwrap()
            .value
            .id;
        let run = self
            .workflow()
            .start_run(&op(), HW, kind, version, &bundle, RunMode::Provider)
            .unwrap()
            .value;
        assert_eq!(run.status, ReviewRunStatus::WaitingForProvider);
        run.id
    }

    fn run(&self, run_id: &str) -> crate::workflow::dto::ReviewRunDto {
        self.database
            .with_connection(|connection| repository::run(connection, run_id))
            .unwrap()
    }

    fn send_and_wait(&self, job_id: &str, sink: &Recorder) -> crate::ai::model::AiJobDto {
        let plan = self.ai().begin_send(job_id).unwrap();
        assert_eq!(plan.job.status, JobStatus::Running);
        self.ai().execute(plan, sink);
        self.ai().job(job_id).unwrap()
    }

    /// Версия v01 с извлечённым текстом решения → этап «Проверка».
    fn to_review(&self) -> String {
        let analysis = self.start(RunKind::AnalyzeMaterials, None);
        let saved = self
            .workflow()
            .save_manual_analysis(
                &op(),
                &analysis,
                &ManualAnalysisInput {
                    summary: "Разобрано".into(),
                    prompt_text: "Постройте модель".into(),
                },
            )
            .unwrap()
            .value;
        let prompt = saved.result.unwrap().prompt.unwrap().id;
        self.workflow().commit_prompt(&op(), &prompt).unwrap();
        self.workflow().hand_off_prompt(&op(), &prompt).unwrap();
        let version = self
            .workflow()
            .create_version(
                &op(),
                HW,
                &VersionInput {
                    file_id: "draft-1".into(),
                    attachment_file_ids: Vec::new(),
                    author: None,
                    prompt_id: None,
                    note: None,
                },
            )
            .unwrap()
            .value;
        version.id
    }

    fn extract_version_text(&self) {
        self.database
            .with_connection(|connection| {
                connection.execute_batch(&format!(
                    "INSERT INTO extractions(id, sha256, extractor_version, options_key, format, status, metadata_json,
                        structure_json, blocks_truncated, text, text_chars, truncated, warnings_json, ocr_used,
                        source_size_bytes, duration_ms, created_at)
                     VALUES ('ext-draft', '{sha}', 1, 'docx', 'docx', 'complete', '{{}}', '[]', 0,
                        'Модель процесса: старт → заявка → оплата', 40, 0, '[]', 0, 512, 2, '2026-10-21T09:00:00Z');
                     INSERT INTO material_extractions(material_kind, material_id, sha256, extraction_id, analyzed_at)
                     VALUES ('file', 'draft-1', '{sha}', 'ext-draft', '2026-10-21T09:00:00Z');",
                    sha = "d".repeat(64)
                ))?;
                Ok(())
            })
            .unwrap();
    }
}

const ANALYSIS: &str = r#"Готово:
```json
{"summary": "Нужно построить BPMN по условию [S1], опираясь на лекцию [S2].", "promptText": "Постройте BPMN-модель процесса закупки."}
```"#;

fn review_json(verdict: &str) -> String {
    json!({
        "verdict": verdict,
        "summary": "Проверено по условию и лекции",
        "correctness": if verdict == "pass" { "pass" } else { "fail" },
        "taskFit": "pass",
        "courseGrounding": "pass",
        "teacher": "na",
        "styleScore": 3,
        "critical": if verdict == "pass" { json!([]) } else { json!([{"text": "Нет шлюза оплаты", "location": "схема"}]) },
        "remarks": [],
        "unsupportedClaims": [],
        "missing": []
    })
    .to_string()
}

// ───────────────────────── Провайдеры ─────────────────────────

#[test]
fn detected_cli_is_never_enabled_without_explicit_connect() {
    let env = env();
    let statuses = env.ai().statuses().unwrap();
    assert!(statuses
        .iter()
        .all(|status| status.availability == ProviderAvailability::Manual && !status.cli.detected));

    install(&env.bin, "codex", FAKE_CODEX);
    install(&env.bin, "claude", FAKE_CLAUDE);
    let codex = env.ai().status(ProviderId::Codex).unwrap();
    assert!(codex.cli.detected);
    assert!(!codex.cli.enabled);
    assert_eq!(
        codex.availability,
        ProviderAvailability::Manual,
        "detection does not connect"
    );
    assert_eq!(codex.cli.version, None, "no probe before explicit action");

    // Fake CLI сообщает успешный вход; подключение всё равно происходит только
    // после явного действия пользователя.
    let status = env.ai().connect(ProviderId::Codex).unwrap();
    assert!(status.cli.enabled);
    assert_eq!(status.availability, ProviderAvailability::Connected);
    assert_eq!(
        status.account_label.as_deref(),
        Some("Codex CLI · codex-cli 0.99.0")
    );
    assert_eq!(status.cli.executable.as_deref(), Some("codex"));

    // Claude CLI найден, но не подключён — остаётся ручная передача.
    let claude = env.ai().status(ProviderId::Claude).unwrap();
    assert!(claude.cli.detected && !claude.cli.enabled);
    assert_eq!(claude.availability, ProviderAvailability::Manual);
    assert_eq!(claude.handoff.web_url, "https://claude.ai/new");
    let claude = env.ai().connect(ProviderId::Claude).unwrap();
    assert_eq!(
        claude.availability,
        ProviderAvailability::Connected,
        "{claude:?}"
    );
    assert_eq!(
        claude.cli.logged_in, None,
        "no auth subcommand in this version"
    );

    let codex = env.ai().disconnect(ProviderId::Codex).unwrap();
    assert_eq!(codex.availability, ProviderAvailability::Manual);
    let settings = env
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT value_json FROM app_settings WHERE key = 'ai_providers'",
                [],
                |row| row.get::<_, String>(0),
            )?)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&settings).unwrap(),
        json!({
            "codex": {"cliEnabled": false, "consentProfileVersion": null},
            "claude": {"cliEnabled": true, "consentProfileVersion": 1},
            "isolationReport": null
        }),
        "only consent state is stored — no paths, tokens or accounts"
    );
}

#[test]
fn legacy_enabled_setting_has_no_versioned_consent_and_stays_manual() {
    let env = env();
    install(&env.bin, "codex", FAKE_CODEX);
    env.database
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO app_settings(key, value_json, created_at, updated_at) VALUES ('ai_providers', ?1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
                [r#"{"codex":{"cliEnabled":true},"claude":{"cliEnabled":false}}"#],
            )?;
            Ok(())
        })
        .unwrap();
    let status = env.ai().status(ProviderId::Codex).unwrap();
    assert!(!status.cli.enabled);
    assert!(!status.isolation.consent_valid);
    assert_eq!(status.availability, ProviderAvailability::Manual);
}

#[test]
fn claude_is_manual_only_without_proof_that_settings_and_hooks_are_disabled() {
    let tmp = TempDir::new().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let bin = root.join("prefix/bin");
    fs::create_dir_all(&bin).unwrap();
    install(&bin, "claude", FAKE_CLAUDE);
    let runtime = ProviderRuntime::for_tests(
        root.join("data/ai"),
        CliLocator::with_dirs(vec![bin], vec![root.join("prefix")], None),
        None,
        Duration::from_secs(5),
    );
    let database = seeded();
    let error = AiService::new(&database, &runtime)
        .connect(ProviderId::Claude)
        .unwrap_err();
    assert_eq!(error.code, crate::error::ErrorCode::Blocked);
    assert!(error.message.contains("settings") && error.message.contains("hooks"));
}

#[test]
fn connect_fails_honestly_without_cli_or_with_unsupported_version() {
    let env = env();
    let error = env.ai().connect(ProviderId::Codex).unwrap_err();
    assert_eq!(error.code, ErrorCode::ProviderUnavailable);
    install(&env.bin, "codex", "#!/bin/sh\necho 'codex 0.1'\n");
    let error = env.ai().connect(ProviderId::Codex).unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(error.message.contains("--sandbox"));
    assert!(!env.ai().status(ProviderId::Codex).unwrap().cli.enabled);
}

#[test]
fn codex_automatic_mode_requires_credentials_proven_inside_isolation() {
    let env = env();
    install(
        &env.bin,
        "codex",
        &FAKE_CODEX.replace("login) exit 0", "login) exit 9"),
    );
    let error = env.ai().connect(ProviderId::Codex).unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(error.message.contains("учётные данные"));
    let status = env.ai().status(ProviderId::Codex).unwrap();
    assert!(!status.cli.enabled);
    assert_eq!(status.cli.logged_in, Some(false));
    assert_eq!(status.availability, ProviderAvailability::Manual);
}

#[test]
fn tasks_choose_providers_independently_of_stage_names() {
    let env = env();
    env.database
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO app_settings(key, value_json) VALUES ('automation_rules',
                 '{\"workflow\":{\"roles\":{\"analyst\":\"claude\",\"author\":\"claude\",\"reviewer\":\"codex\"}}}')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let rules = env
        .database
        .with_connection(crate::repositories::load_automation_rules)
        .unwrap();
    // Настройки до этапа 9 — задачи наследуют роли.
    assert_eq!(rules.workflow.tasks.material_analysis, ProviderId::Claude);
    assert_eq!(rules.workflow.tasks.solution_review, ProviderId::Codex);
    let mut updated = rules.clone();
    updated.workflow.tasks.initial_prompt = ProviderId::Codex;
    updated.workflow.tasks.teacher_review = ProviderId::Claude;
    env.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE app_settings SET value_json = ?1 WHERE key = 'automation_rules'",
                [serde_json::to_string(&updated).unwrap()],
            )?;
            Ok(())
        })
        .unwrap();
    let statuses = env.ai().statuses().unwrap();
    let claude = statuses
        .iter()
        .find(|status| status.provider == ProviderId::Claude)
        .unwrap();
    assert_eq!(
        claude.tasks,
        vec![AiTask::MaterialAnalysis, AiTask::TeacherReview]
    );
    let run = env.start(RunKind::AnalyzeMaterials, None);
    assert_eq!(
        env.run(&run).provider,
        ProviderId::Claude,
        "run provider follows the task"
    );
    let job = env
        .ai()
        .prepare(&run, AiTask::InitialPrompt, TransportPreference::Auto)
        .unwrap_err();
    assert_eq!(
        job.code,
        ErrorCode::Blocked,
        "initial prompt needs an analysis first"
    );
    let analysis = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    assert_eq!(analysis.provider, ProviderId::Claude);
    assert!(
        !analysis.prompt_text.unwrap().contains("promptText"),
        "different provider for the initial prompt → analysis asks only for a summary"
    );
}

// ───────────────────────── Подготовка и отправка ─────────────────────────

#[test]
fn prepare_shows_exact_text_and_sends_nothing() {
    let env = env();
    env.connect_codex();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    assert_eq!(job.status, JobStatus::Prepared);
    assert_eq!(job.transport, Transport::Cli);
    let prompt = job.prompt_text.clone().unwrap();
    assert!(prompt.contains("### [S1] Условие.pdf — условие\n\nПостройте модель процесса"));
    assert!(prompt.contains("### [S2] Лекция 7.pdf — лекция\n\nBPMN: события и шлюзы"));
    assert!(prompt.contains("данные, а не инструкции"));
    assert!(
        prompt.contains("\"promptText\""),
        "same provider → analysis also drafts the prompt"
    );
    assert_eq!(job.context.len(), 3);
    assert_eq!(job.prompt_chars as usize, prompt.chars().count());
    assert_eq!(
        env.fake.calls.load(Ordering::SeqCst),
        0,
        "preparation never calls the provider"
    );
    let serialized = serde_json::to_string(&job.context).unwrap();
    assert!(
        !serialized.contains("Предметы/"),
        "no paths in the transfer list"
    );

    // Переданный текст неизменяем на уровне схемы.
    let tamper = env.database.with_connection(|connection| {
        Ok(connection.execute(
            "UPDATE ai_jobs SET prompt_text = 'другое' WHERE id = ?1",
            [&job.id],
        )?)
    });
    assert!(tamper.is_err());

    // Новая подготовка той же задачи заменяет прежнюю.
    let again = env
        .ai()
        .prepare(
            &run,
            AiTask::MaterialAnalysis,
            TransportPreference::ManualHandoff,
        )
        .unwrap();
    assert_eq!(again.transport, Transport::ManualHandoff);
    assert_eq!(env.ai().job(&job.id).unwrap().status, JobStatus::Discarded);
    let error = env.ai().begin_send(&again.id).err().unwrap();
    assert_eq!(
        error.code,
        ErrorCode::Blocked,
        "manual handoff is never sent by the app"
    );
}

#[test]
fn cli_response_waits_for_confirmation_and_then_applies_with_provenance() {
    let env = env();
    env.connect_codex();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    env.fake.push(Behavior::Respond(ANALYSIS.into()));
    let sink = Recorder::default();
    let done = env.send_and_wait(&job.id, &sink);
    assert_eq!(
        env.fake.prompts.lock().unwrap()[0],
        job.prompt_text.clone().unwrap(),
        "exactly the previewed text"
    );
    assert_eq!(done.status, JobStatus::ResponseReady);
    assert_eq!(done.parse_status, Some(ParseStatus::Valid));
    assert_eq!(done.response_source, Some(ResponseSource::Cli));
    assert_eq!(done.adapter_label.as_deref(), Some("Fake CLI · test"));
    assert_eq!(
        done.draft.as_ref().unwrap()["promptText"],
        "Постройте BPMN-модель процесса закупки."
    );
    let names = sink.names();
    assert_eq!(names.first(), Some(&names::AI_JOB_PROGRESS));
    assert_eq!(names.last(), Some(&names::AI_JOB_UPDATED));

    // Ответ получен, но ничего не применено: запуск открыт, промта нет.
    let current = env.run(&run);
    assert_eq!(current.status, ReviewRunStatus::WaitingForProvider);
    assert!(current.result.is_none());
    let revision_before = env
        .database
        .with_connection(|connection| repository::homework_core(connection, HW))
        .unwrap()
        .revision;

    let draft = done.draft.unwrap();
    let confirmed = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &job.id,
            None,
            &AiConfirmInput::Analysis(ManualAnalysisInput {
                summary: draft["summary"].as_str().unwrap().into(),
                prompt_text: draft["promptText"].as_str().unwrap().into(),
            }),
        )
        .unwrap();
    let outcome = confirmed.value;
    assert_eq!(outcome.run.status, ReviewRunStatus::Completed);
    assert_eq!(outcome.run.provenance, Provenance::Provider);
    assert_eq!(outcome.job.status, JobStatus::Confirmed);
    assert_eq!(outcome.job.edited, Some(false));
    let prompt = outcome.prompt.unwrap();
    assert_eq!(prompt.provenance, Provenance::Provider);
    assert_eq!(
        prompt.status,
        PromptStatus::Draft,
        "the user still commits the prompt"
    );
    assert_eq!(serde_json::to_value(prompt.created_by).unwrap(), "codex");
    let events: Vec<_> = confirmed.events.iter().map(|event| event.name).collect();
    assert!(events.contains(&names::REVIEW_COMPLETED));
    assert!(events.contains(&names::HOMEWORK_UPDATED));
    let revision_after = env
        .database
        .with_connection(|connection| repository::homework_core(connection, HW))
        .unwrap()
        .revision;
    assert_eq!(
        revision_after,
        revision_before + 1,
        "only the confirmation changes the revision"
    );

    // Подтверждённое задание неизменяемо, повторно его не подтвердить.
    let error = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &job.id,
            None,
            &AiConfirmInput::Analysis(ManualAnalysisInput {
                summary: "x".into(),
                prompt_text: "y".into(),
            }),
        )
        .err()
        .unwrap();
    assert_eq!(error.error.code, ErrorCode::Conflict);
    let aggregate = env.workflow().aggregate(HW).unwrap();
    assert_eq!(aggregate.ai_jobs.len(), 1);
    assert!(
        aggregate.ai_jobs[0].prompt_text.is_none(),
        "aggregate carries no texts"
    );
}

#[test]
fn partial_and_corrupt_responses_need_manual_completion() {
    let env = env();
    env.connect_codex();
    let version = env.to_review();
    env.extract_version_text();
    let run = env.start(RunKind::ReviewVersion, Some(&version));
    let job = env
        .ai()
        .prepare(&run, AiTask::SolutionReview, TransportPreference::Auto)
        .unwrap();
    assert!(job
        .prompt_text
        .as_deref()
        .unwrap()
        .contains("— проверяемое решение\n\nМодель процесса"));
    env.fake.push(Behavior::Respond(
        "{\"verdict\": \"needs_fixes\", \"summary\": \"Ошибка\", \"styleScore\": 99}".into(),
    ));
    let sink = Recorder::default();
    let partial = env.send_and_wait(&job.id, &sink);
    assert_eq!(partial.parse_status, Some(ParseStatus::Partial));
    assert!(partial
        .issues
        .iter()
        .any(|issue| issue.code == "missing_field"));
    assert_eq!(partial.draft.as_ref().unwrap()["styleScore"], Value::Null);

    // Неполный черновик дополняется вручную; отметка о правке сохраняется.
    let input = ManualReviewInput {
        verdict: crate::workflow::dto::ReviewVerdict::NeedsFixes,
        summary: "Ошибка".into(),
        correctness: crate::workflow::dto::CriterionStatus::Fail,
        task_fit: crate::workflow::dto::CriterionStatus::Pass,
        course_grounding: crate::workflow::dto::CriterionStatus::Pass,
        teacher: crate::workflow::dto::CriterionStatus::Na,
        style_score: None,
        style_reasons: Vec::new(),
        critical: vec![crate::workflow::service::FindingInput {
            criterion: crate::workflow::dto::CriterionId::Correctness,
            text: "Нет шлюза".into(),
            location: None,
            evidence: Vec::new(),
        }],
        remarks: Vec::new(),
        unsupported_claims: Vec::new(),
        missing: Vec::new(),
    };
    let outcome = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &partial.id,
            None,
            &AiConfirmInput::Review(input.clone()),
        )
        .unwrap()
        .value;
    assert_eq!(outcome.job.edited, Some(true));
    assert_eq!(outcome.run.provenance, Provenance::Provider);
    assert_eq!(
        env.workflow().aggregate(HW).unwrap().stage,
        crate::domain::HomeworkWorkflowStage::NeedsFixes
    );
}

#[test]
fn corrupt_response_keeps_raw_text_and_wrong_format_is_rejected() {
    let env = env();
    env.connect_codex();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    env.fake.push(Behavior::Respond(
        "Извините, я не могу помочь с этим.".into(),
    ));
    let corrupt = env.send_and_wait(&job.id, &Recorder::default());
    assert_eq!(corrupt.status, JobStatus::ResponseReady);
    assert_eq!(corrupt.parse_status, Some(ParseStatus::Corrupt));
    assert_eq!(
        corrupt.response_text.as_deref(),
        Some("Извините, я не могу помочь с этим.")
    );
    assert_eq!(corrupt.draft, Some(json!({})));
    let wrong = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &job.id,
            None,
            &AiConfirmInput::Prompt { text: "x".into() },
        )
        .err()
        .unwrap();
    assert_eq!(wrong.error.code, ErrorCode::Conflict);
    let empty = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &job.id,
            None,
            &AiConfirmInput::Analysis(ManualAnalysisInput {
                summary: " ".into(),
                prompt_text: "Промт".into(),
            }),
        )
        .err()
        .unwrap();
    assert_eq!(
        empty.error.code,
        ErrorCode::Blocked,
        "same validation as manual input"
    );
    // Пользователь может отклонить ответ — запуск снова ждёт.
    let discarded = env.ai().discard(&job.id).unwrap();
    assert_eq!(discarded.status, JobStatus::Discarded);
    assert_eq!(env.run(&run).status, ReviewRunStatus::WaitingForProvider);
}

#[test]
fn failures_timeouts_and_cancel_are_reported_without_false_success() {
    let env = env();
    env.connect_codex();
    let run = env.start(RunKind::AnalyzeMaterials, None);

    // Ошибка без вывода → failed, журнал без секретов и путей.
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    env.fake.push(Behavior::Fail {
        kind: FailureKind::ExitNonZero,
        partial: None,
        stderr: "fatal: token sk-live-abcdefghijklmnop in /Users/student/.codex/config.toml".into(),
    });
    let failed = env.send_and_wait(&job.id, &Recorder::default());
    assert_eq!(failed.status, JobStatus::Failed);
    assert_eq!(failed.error_code.as_deref(), Some("provider_error"));
    let log = serde_json::to_string(&failed.log).unwrap();
    assert!(!log.contains("sk-live"), "{log}");
    assert!(!log.contains("/Users/"), "{log}");
    assert!(log.contains("~/.codex/config.toml"));
    assert_eq!(env.run(&run).status, ReviewRunStatus::WaitingForProvider);
    assert!(env.run(&run).result.is_none());

    // Тайм-аут с частичным выводом → ответ сохранён, но помечен неполным.
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    env.fake.push(Behavior::Fail {
        kind: FailureKind::Timeout,
        partial: Some("{\"summary\": \"Начало анализа\"".into()),
        stderr: String::new(),
    });
    let partial = env.send_and_wait(&job.id, &Recorder::default());
    assert_eq!(partial.status, JobStatus::ResponseReady);
    assert_ne!(partial.parse_status, Some(ParseStatus::Valid));
    assert_eq!(partial.issues[0].code, "process_failed");
    assert_eq!(partial.error_code.as_deref(), Some("timeout"));
    assert!(partial.response_truncated);

    // Отмена выполняющегося задания останавливает его, результата нет.
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    assert_eq!(
        env.ai().job(&partial.id).unwrap().status,
        JobStatus::Discarded
    );
    env.fake.push(Behavior::Hang);
    let plan = env.ai().begin_send(&job.id).unwrap();
    assert_eq!(env.run(&run).status, ReviewRunStatus::Running);
    let second = env.ai().begin_send(&job.id).err().unwrap();
    assert_eq!(second.code, ErrorCode::Conflict, "no double send");
    let sink = Recorder::default();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| env.ai().execute(plan, &sink));
        while !sink.names().contains(&names::AI_JOB_PROGRESS) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(env.ai().cancel(&job.id).unwrap());
        worker.join().unwrap();
    });
    let cancelled = env.ai().job(&job.id).unwrap();
    assert_eq!(cancelled.status, JobStatus::Cancelled);
    assert_eq!(env.run(&run).status, ReviewRunStatus::WaitingForProvider);
    assert!(
        !env.ai().cancel(&job.id).unwrap(),
        "nothing to cancel any more"
    );
    // Журнал операций: запуск отражён без секретов.
    let operations: i64 = env
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT count(*) FROM operations WHERE kind = 'ai_run' AND title LIKE 'Codex:%'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(operations, 3);
}

#[test]
fn auth_failure_marks_provider_for_relogin() {
    let env = env();
    env.connect_codex();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    env.fake.push(Behavior::Fail {
        kind: FailureKind::AuthRequired,
        partial: None,
        stderr: String::new(),
    });
    let sink = Recorder::default();
    let failed = env.send_and_wait(&job.id, &sink);
    assert_eq!(failed.error_code.as_deref(), Some("reauth_required"));
    assert!(sink.names().contains(&names::PROVIDER_STATUS_CHANGED));
    let status = env.ai().status(ProviderId::Codex).unwrap();
    assert_eq!(status.availability, ProviderAvailability::Error);
    let error = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Cli)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::ProviderUnavailable);
}

// ───────────────────────── Ручная передача ─────────────────────────

#[test]
fn manual_handoff_paste_and_confirm_with_separate_prompt_provider() {
    let env = env();
    // Claude не подключён: анализ — ручная передача; первичный промт — тоже Claude.
    env.database
        .with_connection(|connection| {
            let mut rules = crate::dto::AutomationRulesDto::default();
            rules.workflow.tasks.material_analysis = ProviderId::Claude;
            rules.workflow.tasks.initial_prompt = ProviderId::Codex;
            connection.execute(
                "INSERT INTO app_settings(key, value_json) VALUES ('automation_rules', ?1)",
                [serde_json::to_string(&rules).unwrap()],
            )?;
            Ok(())
        })
        .unwrap();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let analysis = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    assert_eq!(analysis.transport, Transport::ManualHandoff);
    assert_eq!(env.run(&run).status, ReviewRunStatus::ManualHandoff);
    let error = env
        .ai()
        .submit_response(&analysis.id, "x", ResponseSource::Cli, None)
        .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::Blocked,
        "the WebView cannot fake CLI provenance"
    );
    let pasted = env
        .ai()
        .submit_response(
            &analysis.id,
            "{\"summary\": \"Итог от Claude\"}",
            ResponseSource::Paste,
            None,
        )
        .unwrap();
    assert_eq!(pasted.parse_status, Some(ParseStatus::Valid));

    // Первичный промт — отдельная задача отдельного провайдера, по черновику анализа.
    let prompt_job = env
        .ai()
        .prepare(&run, AiTask::InitialPrompt, TransportPreference::Auto)
        .unwrap();
    assert!(prompt_job
        .prompt_text
        .as_deref()
        .unwrap()
        .contains("Итог от Claude"));
    let imported = env
        .ai()
        .submit_response(
            &prompt_job.id,
            "Сделайте BPMN-модель.",
            ResponseSource::File,
            Some("/Users/student/Downloads/ответ.txt"),
        )
        .unwrap();
    assert_eq!(
        imported.response_file_name.as_deref(),
        Some("ответ.txt"),
        "only the name is kept"
    );
    assert_eq!(
        imported.parse_status,
        Some(ParseStatus::Partial),
        "plain text prompt is accepted with a note"
    );

    let outcome = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &analysis.id,
            Some(&prompt_job.id),
            &AiConfirmInput::Analysis(ManualAnalysisInput {
                summary: "Итог от Claude".into(),
                prompt_text: "Сделайте BPMN-модель (поправлено).".into(),
            }),
        )
        .unwrap()
        .value;
    assert_eq!(outcome.job.edited, Some(false));
    let prompt_job = env.ai().job(&prompt_job.id).unwrap();
    assert_eq!(prompt_job.status, JobStatus::Confirmed);
    assert_eq!(prompt_job.edited, Some(true));
    let prompt = outcome.prompt.unwrap();
    assert_eq!(serde_json::to_value(prompt.created_by).unwrap(), "codex");
    assert_eq!(prompt.provenance, Provenance::Provider);
}

#[test]
fn revision_prompt_and_teacher_review_tasks() {
    let env = env();
    env.connect_codex();
    let version = env.to_review();
    env.extract_version_text();
    let review = env.start(RunKind::ReviewVersion, Some(&version));
    let job = env
        .ai()
        .prepare(&review, AiTask::SolutionReview, TransportPreference::Auto)
        .unwrap();
    env.fake.push(Behavior::Respond(review_json("needs_fixes")));
    let done = env.send_and_wait(&job.id, &Recorder::default());
    let draft: ManualReviewInput = serde_json::from_value(done.draft.clone().unwrap()).unwrap();
    let outcome = env
        .workflow()
        .confirm_ai_result(&op(), &job.id, None, &AiConfirmInput::Review(draft))
        .unwrap()
        .value;
    assert_eq!(outcome.job.edited, Some(false));

    // Промт исправлений — по завершённой проверке, только её проблемы.
    let fix = env
        .ai()
        .prepare(&review, AiTask::RevisionPrompt, TransportPreference::Auto)
        .unwrap();
    let text = fix.prompt_text.clone().unwrap();
    assert!(text.contains("Нет шлюза оплаты (схема)"));
    env.fake.push(Behavior::Respond(
        "{\"text\": \"Добавьте шлюз оплаты\"}".into(),
    ));
    let fixed = env.send_and_wait(&fix.id, &Recorder::default());
    assert_eq!(fixed.parse_status, Some(ParseStatus::Valid));
    let outcome = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &fix.id,
            None,
            &AiConfirmInput::Prompt {
                text: "Добавьте шлюз оплаты".into(),
            },
        )
        .unwrap()
        .value;
    let prompt = outcome.prompt.unwrap();
    assert_eq!(prompt.kind, PromptKind::Revision);
    assert_eq!(prompt.provenance, Provenance::Provider);
    assert_eq!(prompt.review_run_id.as_deref(), Some(review.as_str()));
    assert_eq!(outcome.job.prompt_id.as_deref(), Some(prompt.id.as_str()));

    // Проверка по комментариям преподавателя без комментариев невозможна.
    let teacher_runs = env
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT count(*) FROM review_runs WHERE kind = 'review_teacher'",
                [],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .unwrap();
    assert_eq!(teacher_runs, 0);
}

#[test]
fn review_requires_extracted_solution_text() {
    let env = env();
    env.connect_codex();
    let version = env.to_review();
    let run = env.start(RunKind::ReviewVersion, Some(&version));
    let error = env
        .ai()
        .prepare(&run, AiTask::SolutionReview, TransportPreference::Auto)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(error.message.contains("Решение_v1.docx"));
    let wrong = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap_err();
    assert_eq!(wrong.code, ErrorCode::Blocked, "task must match the run");
}

#[test]
fn context_limit_truncates_large_materials_with_visible_marks() {
    let env = env();
    let big = "а".repeat(200_000);
    env.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE extractions SET text = ?1 WHERE id = 'ext-lecture'",
                [&big],
            )?;
            Ok(())
        })
        .unwrap();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    assert!(job.context_truncated);
    assert_eq!(job.limit_chars, super::context::MAX_CONTEXT_CHARS);
    let lecture = &job.context[1];
    assert!(lecture.truncated);
    assert!(lecture.chars_included < lecture.chars_total);
    assert!(job.prompt_chars < 160_000);
    assert!(job
        .issues
        .iter()
        .any(|issue| issue.code == "context_truncated"));
    assert!(job.prompt_text.unwrap().contains("[… обрезано: передано"));
}

#[test]
fn deleted_extraction_blocks_preparation() {
    let env = env();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    env.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE extractions SET sha256 = ?1 WHERE id = 'ext-task'",
                ["0".repeat(64)],
            )?;
            Ok(())
        })
        .unwrap();
    let error = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(error.message.contains("Условие.pdf"));
}

#[test]
fn cancelling_the_run_stops_and_closes_its_jobs_and_restart_recovers_running_jobs() {
    let env = env();
    env.connect_codex();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let prepared = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    env.ai().cancel_for_run(&run).unwrap();
    assert_eq!(
        env.ai().job(&prepared.id).unwrap().status,
        JobStatus::Discarded
    );

    // Задание «выполняется», но процесса уже нет (перезапуск приложения).
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    let plan = env.ai().begin_send(&job.id).unwrap();
    drop(plan);
    let recovered = env
        .database
        .with_transaction(|transaction| store::recover_interrupted(transaction))
        .unwrap();
    assert_eq!(recovered, 1);
    let job = env.ai().job(&job.id).unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert_eq!(job.error_code.as_deref(), Some("interrupted"));
    assert_eq!(env.run(&run).status, ReviewRunStatus::WaitingForProvider);
}

#[test]
fn real_cli_adapter_runs_isolated_fake_codex_end_to_end() {
    // Без фабрики: настоящий CliAdapter и фиктивный исполняемый файл в TempDir.
    let tmp = TempDir::new().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let bin = root.join("prefix/bin");
    let home = root.join("home");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("logged-in"), "").unwrap();
    let marker = root.join("env-dump");
    install(
        &bin,
        "codex",
        &format!(
            r#"#!/bin/sh
case "$1" in
  --version) echo "codex-cli 1.0.0"; exit 0;;
  login) exit 0;;
  exec)
    if [ "$2" = "--help" ]; then
      echo 'Usage: codex exec [OPTIONS]'; echo 'Prompt from stdin'; echo '  read-only'
      for f in --sandbox --skip-git-repo-check --output-last-message --ephemeral --ignore-user-config --ignore-rules --strict-config; do echo "  $f <X>"; done; exit 0
    fi
    out=""; prev=""; cd_dir=""
    for a in "$@"; do
      [ "$prev" = "--output-last-message" ] && out="$a"
      [ "$prev" = "--cd" ] && cd_dir="$a"
      prev="$a"
    done
    env > '{marker}'
    pwd >> '{marker}'
    cat > /dev/null
    printf '{{"summary": "из CLI", "promptText": "промт из CLI"}}' > "$out";;
esac
"#,
            marker = marker.display()
        ),
    );
    let runtime = ProviderRuntime::for_tests(
        root.join("data/ai"),
        CliLocator::with_dirs(vec![bin], vec![root.join("prefix")], Some(home.clone())),
        None,
        Duration::from_secs(10),
    );
    let database = seeded();
    let verifier = FakeVerifier::default();
    let ai = AiService::new(&database, &runtime);
    assert!(ai.check_isolation().unwrap().passed);
    assert_eq!(
        ai.connect(ProviderId::Codex).unwrap().availability,
        ProviderAvailability::Connected
    );
    std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-should-not-leak-123456");
    let workflow = WorkflowService::new(&database, &verifier);
    let bundle = workflow
        .create_source_bundle(&op(), HW, RunKind::AnalyzeMaterials, None)
        .unwrap()
        .value
        .id;
    let run = workflow
        .start_run(
            &op(),
            HW,
            RunKind::AnalyzeMaterials,
            None,
            &bundle,
            RunMode::Provider,
        )
        .unwrap()
        .value
        .id;
    let job = ai
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Cli)
        .unwrap();
    let plan = ai.begin_send(&job.id).unwrap();
    ai.execute(plan, &Recorder::default());
    std::env::remove_var("ANTHROPIC_API_KEY");
    let done = ai.job(&job.id).unwrap();
    assert_eq!(done.status, JobStatus::ResponseReady, "{:?}", done.log);
    assert_eq!(done.parse_status, Some(ParseStatus::Valid));
    assert_eq!(
        done.adapter_label.as_deref(),
        Some("Codex CLI · codex-cli 1.0.0")
    );
    assert!(
        !marker.exists(),
        "isolated CLI cannot write outside its job directory"
    );
    let serialized = serde_json::to_string(&done).unwrap();
    assert!(!serialized.contains("ANTHROPIC_API_KEY"));
    assert!(!serialized.contains(&home.to_string_lossy().to_string()));
    let leftovers: Vec<_> = fs::read_dir(root.join("data/ai"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".kontur-ai-")
        })
        .collect();
    assert!(leftovers.is_empty(), "workdir is removed after the run");
}

#[test]
fn current_migration_keeps_rows_and_enforces_job_invariants() {
    let database = seeded();
    let (version, tables): (u32, i64) = database
        .with_connection(|connection| {
            Ok((
                connection.pragma_query_value(None, "user_version", |row| row.get(0))?,
                connection.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'ai_jobs'",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(version, crate::db::migrations::CURRENT_SCHEMA_VERSION);
    assert_eq!(tables, 1);
    // Задание чужого запуска и абсолютные пути в составе отклоняются схемой.
    let insert = |run: &str, context: &str| {
        database.with_connection(|connection| {
            Ok(connection.execute(
                "INSERT INTO ai_jobs(id, homework_id, run_id, task, provider, transport, status, prompt_text,
                                     prompt_sha256, prompt_chars, context_json, limit_chars, created_at, updated_at)
                 VALUES (?1, 'hw-1', ?2, 'material_analysis', 'codex', 'cli', 'prepared', 'x', ?3, 1, ?4, 10, 'n', 'n')",
                rusqlite::params![Uuid::new_v4().to_string(), run, "a".repeat(64), context],
            )?)
        })
    };
    assert!(insert("missing-run", "[]").is_err());
    let env = env();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    env.ai().discard(&job.id).unwrap();
    let terminal = env.database.with_connection(|connection| {
        Ok(connection.execute(
            "UPDATE ai_jobs SET status = 'prepared' WHERE id = ?1",
            [&job.id],
        )?)
    });
    assert!(terminal.is_err(), "finished job is immutable");
    let path = env.database.with_connection(|connection| {
        Ok(connection.execute(
            "INSERT INTO ai_jobs(id, homework_id, run_id, task, provider, transport, status, prompt_text,
                                 prompt_sha256, prompt_chars, context_json, limit_chars, created_at, updated_at)
             VALUES ('p', 'hw-1', ?1, 'material_analysis', 'codex', 'cli', 'prepared', 'x', ?2, 1,
                     '[{\"name\": \"/Users/student/x.pdf\"}]', 10, 'n', 'n')",
            rusqlite::params![run, "a".repeat(64)],
        )?)
    });
    assert!(path.is_err(), "absolute paths are rejected");
}

#[test]
fn prompt_typed_by_the_user_is_not_attributed_to_the_provider() {
    let env = env();
    env.connect_codex();
    let run = env.start(RunKind::AnalyzeMaterials, None);
    let job = env
        .ai()
        .prepare(&run, AiTask::MaterialAnalysis, TransportPreference::Auto)
        .unwrap();
    // Провайдер должен был вернуть и промт, но вернул только итог.
    env.fake
        .push(Behavior::Respond("{\"summary\": \"Итог\"}".into()));
    let ready = env.send_and_wait(&job.id, &Recorder::default());
    assert_eq!(ready.parse_status, Some(ParseStatus::Partial));
    let outcome = env
        .workflow()
        .confirm_ai_result(
            &op(),
            &job.id,
            None,
            &AiConfirmInput::Analysis(ManualAnalysisInput {
                summary: "Итог".into(),
                prompt_text: "Промт, написанный пользователем".into(),
            }),
        )
        .unwrap()
        .value;
    assert_eq!(
        outcome.job.edited,
        Some(true),
        "missing field filled by the user"
    );
    assert_eq!(outcome.run.provenance, Provenance::Provider);
    let prompt = outcome.prompt.unwrap();
    assert_eq!(prompt.provenance, Provenance::Manual);
    assert_eq!(serde_json::to_value(prompt.created_by).unwrap(), "user");
}
