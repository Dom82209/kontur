//! Application service рабочего цикла ДЗ (этап 8B).
//!
//! Каждая мутация проходит один и тот же путь:
//! 1. `operationId` (UUID v4) и отпечаток запроса; повтор с тем же id и тем же
//!    запросом возвращает сохранённый итог без повторных действий и событий,
//!    тот же id с другим запросом — `conflict`;
//! 2. транзакция: операция `running` → проверка ревизии агрегата (устаревшая —
//!    `conflict`) → проверка состояния и state machine → запись → новая ревизия →
//!    итог операции → `completed`;
//! 3. ошибка откатывает транзакцию целиком и записывает одну отдельную `failed`
//!    операцию;
//! 4. события формируются только после commit из перечитанного состояния.
//!
//! Реальных AI-адаптеров нет: запуск получает честный статус
//! `waiting_for_provider` или `manual_handoff`, результат сохраняет пользователь
//! с provenance `manual`. Фальшивый AI-результат не создаётся.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    ai::{
        self,
        model::{AiJobDto, AiTask, JobStatus, Transport},
    },
    db::Database,
    domain::{HomeworkStage, HomeworkWorkflowStage as Stage, RequirementStatus, VersionAuthor},
    dto::{
        AutomationRulesDto, FileRefDto, HomeworkDto, HomeworkVersionDto, OperationDto,
        OperationKind,
    },
    error::{AppError, BackendError, ErrorCode},
    events::{
        names, OperationCompletedPayload, OperationFailedPayload, WorkflowStageChangedPayload,
    },
    intake::validate_operation_id,
    repositories::{load_automation_rules, ReadRepository},
};

use super::{
    bundle::{prepare_snapshot, write_snapshot, ContentVerifier},
    dto::{
        CommentAuthority, CriterionId, CriterionStatus, DashboardDto, EvidenceReferenceDto,
        FindingSeverity, HomeworkIterationDto, HomeworkWorkflowDto, PromptKind, PromptStatus,
        Provenance, ProviderModeDto, ReadinessCheckId, ReadinessDto, RecommendedActionDto,
        RecommendedActionKind, ReviewCriterionDto, ReviewFindingDto, ReviewMissingItemDto,
        ReviewRunDto, ReviewRunStatus, ReviewVerdict, RevisionPromptDto, RunKind, SourceBundleDto,
        StoredReviewResult, StyleConfirmationDto, StyleDivergenceDto, StyleReasonDto,
        TeacherCommentDto, TeacherCommentSource, TeacherRequirementDto, UnsupportedClaimDto,
        VersionComparisonDto, WorkflowRole,
    },
    projections,
    repository::{self, WorkflowSnapshot},
    state_machine::{self, TransitionContext},
};

// ───────────────────────── Входные DTO ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Шаг назначен провайдеру роли; адаптера нет — запуск ждёт (`waiting_for_provider`).
    Provider,
    /// Пользователь выполняет шаг вручную (`manual_handoff`).
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualAnalysisInput {
    pub summary: String,
    pub prompt_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindingInput {
    pub criterion: CriterionId,
    pub text: String,
    pub location: Option<String>,
    #[serde(default)]
    pub evidence: Vec<EvidenceReferenceDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaimInput {
    pub text: String,
    pub location: String,
}

/// Ручной результат проверки: каждое измерение задаётся отдельно.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManualReviewInput {
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub correctness: CriterionStatus,
    pub task_fit: CriterionStatus,
    pub course_grounding: CriterionStatus,
    pub teacher: CriterionStatus,
    /// Расхождение с личным стилем 0…10 (эвристика, не определение авторства).
    pub style_score: Option<u8>,
    #[serde(default)]
    pub style_reasons: Vec<StyleReasonDto>,
    #[serde(default)]
    pub critical: Vec<FindingInput>,
    #[serde(default)]
    pub remarks: Vec<FindingInput>,
    #[serde(default)]
    pub unsupported_claims: Vec<ClaimInput>,
    #[serde(default)]
    pub missing: Vec<ReviewMissingItemDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionInput {
    pub file_id: String,
    #[serde(default)]
    pub attachment_file_ids: Vec<String>,
    pub author: Option<VersionAuthor>,
    pub prompt_id: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TeacherCommentInput {
    pub source: TeacherCommentSource,
    pub date: String,
    pub question: String,
    pub answer: String,
    /// Этап 7: кто ответил. По умолчанию — неизвестно (не официально).
    #[serde(default = "unknown_authority")]
    pub authority: CommentAuthority,
    #[serde(default)]
    pub author: String,
    /// Ссылка на сообщение или цитата с контекстом.
    #[serde(default)]
    pub context: String,
    /// Пользователь явно подтвердил, что это официальный ответ.
    #[serde(default)]
    pub confirmed_official: bool,
}

impl TeacherCommentInput {
    /// Импорт переписки: автор не установлен, ответ не официальный (этап 7).
    pub fn unverified(
        source: TeacherCommentSource,
        date: String,
        question: String,
        answer: String,
    ) -> Self {
        Self {
            source,
            date,
            question,
            answer,
            authority: CommentAuthority::Unknown,
            author: String::new(),
            context: String::new(),
            confirmed_official: false,
        }
    }
}

fn unknown_authority() -> CommentAuthority {
    CommentAuthority::Unknown
}

/// Этап 7: изменение автора и официальности уже сохранённого ответа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommentAuthorityInput {
    pub authority: CommentAuthority,
    pub author: String,
    pub context: String,
    pub confirmed_official: bool,
}

fn validate_authority(
    authority: CommentAuthority,
    author: &str,
    context: &str,
    confirmed_official: bool,
) -> Result<(), AppError> {
    text_field(author, "Автор", 200, false)?;
    text_field(context, "Ссылка или контекст", 4_000, false)?;
    if confirmed_official && !authority.can_be_official() {
        return Err(BackendError::blocked(
            "Официальным может быть только ответ преподавателя или ассистента: мнение студента, сообщение из чата без проверки, заметка или системный текст не становятся требованием",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TeacherRequirementInput {
    pub comment_id: String,
    pub text: String,
    pub origin: String,
    pub status: RequirementStatus,
    pub note: Option<String>,
}

/// Метаданные команды: идемпотентность и optimistic revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandMeta {
    pub operation_id: String,
    pub expected_revision: Option<i64>,
}

// ───────────────────────── Итог команды ─────────────────────────

/// Событие для доставки после commit: имя и уже сериализованный camelCase payload.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowEvent {
    pub name: &'static str,
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct WorkflowSuccess<T> {
    pub value: T,
    pub operation: OperationDto,
    pub events: Vec<WorkflowEvent>,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct WorkflowFailure {
    pub error: AppError,
    pub operation: Option<Box<OperationDto>>,
    /// `operation:failed`, если отдельная failed-операция записана.
    pub events: Vec<WorkflowEvent>,
}

impl WorkflowFailure {
    fn plain(error: AppError) -> Self {
        Self {
            error,
            operation: None,
            events: Vec::new(),
        }
    }
}

pub type WorkflowResult<T> = Result<WorkflowSuccess<T>, WorkflowFailure>;

/// Что изменила команда: по этой ссылке итог перечитывается (и при повторе).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EntityRef {
    Bundle { id: String },
    Run { id: String },
    Prompt { id: String },
    Version { id: String },
    Homework { id: String },
    Comments { ids: Vec<String> },
    Requirement { id: String },
    Confirmation { id: String },
    AiJob { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredOutcome {
    command: String,
    homework_id: String,
    entity: EntityRef,
}

/// Как найти ДЗ команды.
enum Target<'a> {
    Homework(&'a str),
    Run(&'a str),
    Prompt(&'a str),
    Version(&'a str),
    Requirement(&'a str),
    AiJob(&'a str),
    Comment(&'a str),
}

struct Spec {
    command: &'static str,
    kind: OperationKind,
    title: String,
}

enum Pending {
    VersionCreated(String),
    ReviewCompleted(String),
    PromptCreated(String),
}

/// Состояние команды внутри транзакции.
pub struct TxContext<'c> {
    connection: &'c Connection,
    homework_id: String,
    rules: AutomationRulesDto,
    transitions: Vec<(Stage, Stage, String)>,
    pending: Vec<Pending>,
}

impl TxContext<'_> {
    fn snapshot(&self) -> Result<WorkflowSnapshot, AppError> {
        repository::load_snapshot(self.connection, &self.homework_id)
    }

    fn stage(&self) -> Result<Stage, AppError> {
        Ok(repository::homework_core(self.connection, &self.homework_id)?.workflow_stage)
    }

    /// Этап 7: если «Готово к сдаче» отмечено по другим входным данным
    /// (изменились официальные требования, материалы, условие, версия или
    /// профиль стиля), отметка снимается в той же транзакции.
    fn reopen_if_ready_stale(&mut self, reason: &str) -> Result<(), AppError> {
        let snapshot = self.snapshot()?;
        if snapshot.homework.workflow_stage == Stage::ReadyToSubmit
            && !snapshot
                .inputs
                .is_current(snapshot.ready_fingerprint.as_deref())
        {
            self.transition(Stage::TeacherReview, reason, false)?;
        }
        Ok(())
    }

    /// Переход только через state machine, по состоянию с учётом уже сделанных записей.
    fn transition(
        &mut self,
        to: Stage,
        reason: &str,
        no_teacher_comments: bool,
    ) -> Result<(), AppError> {
        let snapshot = self.snapshot()?;
        let from = snapshot.homework.workflow_stage;
        if from == to {
            return Ok(());
        }
        let readiness = projections::readiness(&snapshot);
        state_machine::check_transition(
            from,
            to,
            &TransitionContext {
                snapshot: &snapshot,
                readiness: &readiness,
                require_teacher_review: self.rules.workflow.require_teacher_review,
                no_teacher_comments,
            },
        )?;
        repository::set_workflow_stage(self.connection, &self.homework_id, to)?;
        self.transitions.push((from, to, reason.to_owned()));
        Ok(())
    }
}

fn request_hash(command: &str, request: &impl Serialize) -> Result<String, AppError> {
    let body = serde_json::to_vec(request)
        .map_err(|error| BackendError::database(format!("cannot fingerprint request: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(command.as_bytes());
    hasher.update([0_u8]);
    hasher.update(&body);
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn event<T: Serialize>(name: &'static str, payload: &T) -> Result<WorkflowEvent, AppError> {
    let payload = serde_json::to_value(payload)
        .map_err(|error| BackendError::database(format!("cannot serialize event: {error}")))?;
    Ok(WorkflowEvent { name, payload })
}

fn text_field(value: &str, field: &str, max_chars: usize, required: bool) -> Result<(), AppError> {
    let chars = value.chars().count();
    if required && value.trim().is_empty() {
        return Err(BackendError::blocked(format!(
            "Поле «{field}» не заполнено"
        )));
    }
    if chars > max_chars {
        return Err(BackendError::blocked(format!(
            "Поле «{field}» длиннее {max_chars} символов"
        )));
    }
    Ok(())
}

fn role_for(kind: RunKind) -> WorkflowRole {
    match kind {
        RunKind::AnalyzeMaterials => WorkflowRole::Analyst,
        RunKind::ReviewVersion | RunKind::RevisionPrompt | RunKind::ReviewTeacher => {
            WorkflowRole::Reviewer
        }
    }
}

/// Основная AI-задача запуска: по ней выбирается провайдер (этап 9).
pub fn task_for_run(kind: RunKind) -> AiTask {
    match kind {
        RunKind::AnalyzeMaterials => AiTask::MaterialAnalysis,
        RunKind::ReviewVersion | RunKind::RevisionPrompt => AiTask::SolutionReview,
        RunKind::ReviewTeacher => AiTask::TeacherReview,
    }
}

fn applied_threshold(rules: &AutomationRulesDto) -> Result<u8, AppError> {
    u8::try_from(rules.workflow.style_threshold)
        .ok()
        .filter(|value| *value <= 10)
        .ok_or_else(|| BackendError::database("styleThreshold is outside 0..10"))
}

// ───────────────────────── Журнал операций ─────────────────────────

fn insert_running(
    connection: &Connection,
    id: &str,
    spec: &Spec,
    homework_id: &str,
) -> Result<(), AppError> {
    connection
        .execute(
            "INSERT INTO operations(id, kind, status, progress, undoable, started_at, title, detail,
                                    paths_json, homework_id)
             VALUES (?1, ?2, 'running', 0, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?3, NULL, '[]', ?4)",
            params![id, spec.kind.as_db_str(), spec.title, homework_id],
        )
        .map_err(|error| repository::map_write_error(error, "Операция с этим идентификатором уже существует"))?;
    Ok(())
}

fn insert_failed(
    connection: &Connection,
    id: &str,
    spec: &Spec,
    homework_id: Option<&str>,
    error: &AppError,
) -> Result<(), AppError> {
    #[derive(Serialize)]
    struct SafeError<'a> {
        code: ErrorCode,
        message: &'a str,
    }
    let error_json = serde_json::to_string(&SafeError {
        code: error.code,
        message: "Операция не выполнена",
    })
    .map_err(|serialization| BackendError::database(serialization.to_string()))?;
    connection.execute(
        "INSERT INTO operations(id, kind, status, progress, undoable, started_at, completed_at, title,
                                detail, paths_json, error_json, homework_id)
         VALUES (?1, ?2, 'failed', 0, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?3, 'Операция не выполнена', '[]', ?4, ?5)",
        params![id, spec.kind.as_db_str(), spec.title, error_json, homework_id],
    )?;
    Ok(())
}

enum Prior {
    None,
    Replay(StoredOutcome),
}

fn prior_operation(
    connection: &Connection,
    operation_id: &str,
    hash: &str,
) -> Result<Prior, AppError> {
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM operations WHERE id = ?1",
            [operation_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(status) = status else {
        return Ok(Prior::None);
    };
    let stored: Option<(String, Option<String>, String)> = connection
        .query_row(
            "SELECT kind, request_hash, result_json FROM operation_results WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    match (status.as_str(), stored) {
        ("completed", Some((kind, Some(stored_hash), result))) if kind == "workflow" => {
            if stored_hash != hash {
                return Err(BackendError::conflict(
                    "Этот идентификатор операции уже использован для другой команды",
                ));
            }
            Ok(Prior::Replay(repository::parse_column(&result, "operation_results.result_json")?))
        }
        ("failed" | "cancelled", _) => Err(BackendError::conflict(
            "Операция с этим идентификатором уже завершилась ошибкой — повторите действие с новым идентификатором",
        )),
        ("running" | "started", _) => Err(BackendError::conflict(
            "Операция с этим идентификатором ещё выполняется",
        )),
        _ => Err(BackendError::conflict(
            "Этот идентификатор операции уже использован другой командой",
        )),
    }
}

fn resolve_homework(connection: &Connection, target: &Target<'_>) -> Result<String, AppError> {
    let (sql, id, missing) = match target {
        Target::Homework(id) => return Ok(repository::homework_core(connection, id)?.id),
        Target::Run(id) => (
            "SELECT homework_id FROM review_runs WHERE id = ?1",
            *id,
            "Запуск не найден",
        ),
        Target::Prompt(id) => (
            "SELECT homework_id FROM revision_prompts WHERE id = ?1",
            *id,
            "Промт не найден",
        ),
        Target::Version(id) => (
            "SELECT homework_id FROM homework_versions WHERE id = ?1",
            *id,
            "Версия не найдена",
        ),
        Target::Requirement(id) => (
            "SELECT homework_id FROM teacher_requirements WHERE id = ?1",
            *id,
            "Требование не найдено",
        ),
        Target::Comment(id) => (
            "SELECT homework_id FROM teacher_comments WHERE id = ?1",
            *id,
            "Комментарий не найден",
        ),
        Target::AiJob(id) => (
            "SELECT homework_id FROM ai_jobs WHERE id = ?1",
            *id,
            "Задание не найдено",
        ),
    };
    connection
        .query_row(sql, [id], |row| row.get(0))
        .map_err(|error| repository::not_found_or(error, missing))
}

// ───────────────────────── Сервис ─────────────────────────

pub struct WorkflowService<'a> {
    database: &'a Database,
    verifier: &'a dyn ContentVerifier,
}

impl<'a> WorkflowService<'a> {
    pub fn new(database: &'a Database, verifier: &'a dyn ContentVerifier) -> Self {
        Self { database, verifier }
    }

    // ── Чтение ──

    pub fn aggregate(&self, homework_id: &str) -> Result<HomeworkWorkflowDto, AppError> {
        self.database.with_connection(|connection| {
            let snapshot = repository::load_snapshot(connection, homework_id)?;
            let rules = load_automation_rules(connection)?;
            Ok(aggregate_dto(snapshot, &rules))
        })
    }

    pub fn readiness(&self, homework_id: &str) -> Result<ReadinessDto, AppError> {
        self.database.with_connection(|connection| {
            Ok(projections::readiness(&repository::load_snapshot(
                connection,
                homework_id,
            )?))
        })
    }

    pub fn iterations(&self, homework_id: &str) -> Result<Vec<HomeworkIterationDto>, AppError> {
        self.database.with_connection(|connection| {
            Ok(projections::iterations(&repository::load_snapshot(
                connection,
                homework_id,
            )?))
        })
    }

    pub fn compare_versions(
        &self,
        from_version_id: &str,
        to_version_id: &str,
    ) -> Result<VersionComparisonDto, AppError> {
        self.database.with_connection(|connection| {
            let homework_id = resolve_homework(connection, &Target::Version(from_version_id))?;
            if resolve_homework(connection, &Target::Version(to_version_id))? != homework_id {
                return Err(BackendError::conflict("Версии относятся к разным ДЗ"));
            }
            let snapshot = repository::load_snapshot(connection, &homework_id)?;
            projections::compare_versions(&snapshot, from_version_id, to_version_id)
        })
    }

    pub fn candidates(&self, homework_id: &str) -> Result<Vec<FileRefDto>, AppError> {
        self.database.with_connection(|connection| {
            repository::homework_core(connection, homework_id)?;
            repository::version_candidates(connection, homework_id)
        })
    }

    /// Главная. Статусы провайдеров вычисляет `AiService` (этап 9).
    pub fn dashboard(
        &self,
        providers: Vec<super::dto::ProviderStatusDto>,
    ) -> Result<DashboardDto, AppError> {
        projections::dashboard(self.database, time::OffsetDateTime::now_utc(), providers)
    }

    // ── Общий транзакционный исполнитель ──

    fn execute<T>(
        &self,
        meta: &CommandMeta,
        spec: Spec,
        request: &impl Serialize,
        target: Target<'_>,
        body: impl FnOnce(&mut TxContext<'_>) -> Result<EntityRef, AppError>,
        read: impl Fn(&Connection, &EntityRef) -> Result<T, AppError>,
    ) -> WorkflowResult<T> {
        validate_operation_id(&meta.operation_id).map_err(WorkflowFailure::plain)?;
        let hash = request_hash(spec.command, &(request, &meta.expected_revision))
            .map_err(WorkflowFailure::plain)?;
        let prior = self
            .database
            .with_connection(|connection| prior_operation(connection, &meta.operation_id, &hash))
            .map_err(WorkflowFailure::plain)?;
        if let Prior::Replay(outcome) = prior {
            return self.replay(&meta.operation_id, &outcome, read);
        }

        let mut homework_for_failure: Option<String> = None;
        let committed = self.database.with_transaction(|transaction| {
            let homework_id = resolve_homework(transaction, &target)?;
            homework_for_failure = Some(homework_id.clone());
            insert_running(transaction, &meta.operation_id, &spec, &homework_id)?;
            let core = repository::homework_core(transaction, &homework_id)?;
            if core.stage == HomeworkStage::Submitted || core.workflow_stage == Stage::Submitted {
                return Err(BackendError::immutable("Работа отправлена — снимок неизменяем"));
            }
            if let Some(expected) = meta.expected_revision {
                if expected != core.revision {
                    return Err(repository::stale_revision());
                }
            }
            let mut context = TxContext {
                connection: transaction,
                homework_id: homework_id.clone(),
                rules: load_automation_rules(transaction)?,
                transitions: Vec::new(),
                pending: Vec::new(),
            };
            let entity = body(&mut context)?;
            let revision = repository::bump_revision(transaction, &homework_id, core.revision)?;
            let outcome = StoredOutcome {
                command: spec.command.to_owned(),
                homework_id: homework_id.clone(),
                entity,
            };
            transaction.execute(
                "INSERT INTO operation_results(operation_id, kind, request_hash, result_json, created_at)
                 VALUES (?1, 'workflow', ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![
                    meta.operation_id,
                    hash,
                    repository::json_text(&outcome, "operation_results.result_json")?
                ],
            )?;
            crate::repositories::writes::complete_operation(transaction, &meta.operation_id)?;
            Ok((outcome, context.transitions, context.pending, revision))
        });

        let (outcome, transitions, pending, revision) = match committed {
            Ok(value) => value,
            Err(error) => {
                return Err(self.record_failure(
                    &meta.operation_id,
                    &spec,
                    homework_for_failure.as_deref(),
                    error,
                ))
            }
        };

        // После commit: всё перечитывается из базы; ошибка чтения не откатывает данные.
        let post = self.database.with_connection(|connection| {
            let value = read(connection, &outcome.entity)?;
            let operation = crate::repositories::operation_in(connection, &meta.operation_id)?;
            let snapshot = repository::load_snapshot(connection, &outcome.homework_id)?;
            let mut events = Vec::new();
            for item in &pending {
                match item {
                    Pending::VersionCreated(id) => {
                        let version = snapshot
                            .homework
                            .versions
                            .iter()
                            .find(|version| &version.id == id)
                            .ok_or_else(|| BackendError::database("created version is missing"))?;
                        events.push(event(
                            names::HOMEWORK_VERSION_CREATED,
                            &crate::events::HomeworkVersionCreatedPayload {
                                homework_id: outcome.homework_id.clone(),
                                version: version.clone(),
                            },
                        )?);
                    }
                    Pending::ReviewCompleted(id) => {
                        let run = repository::run(connection, id)?;
                        events.push(event(
                            names::REVIEW_COMPLETED,
                            &crate::events::ReviewCompletedPayload { run },
                        )?);
                    }
                    Pending::PromptCreated(id) => {
                        let prompt = repository::prompt(connection, id)?;
                        events.push(event(
                            names::HOMEWORK_REVISION_PROMPT_CREATED,
                            &RevisionPromptCreatedPayload {
                                homework_id: outcome.homework_id.clone(),
                                prompt_id: prompt.id,
                                kind: prompt.kind,
                                status: prompt.status,
                                review_run_id: prompt.review_run_id,
                                based_on_version_id: prompt.based_on_version_id,
                            },
                        )?);
                    }
                }
            }
            for (from, to, reason) in &transitions {
                events.push(event(
                    names::WORKFLOW_STAGE_CHANGED,
                    &WorkflowStageChangedPayload {
                        homework_id: outcome.homework_id.clone(),
                        from: *from,
                        to: *to,
                        reason: reason.clone(),
                    },
                )?);
            }
            events.push(event(
                names::HOMEWORK_UPDATED,
                &HomeworkUpdatedPayload {
                    homework_id: outcome.homework_id.clone(),
                    revision,
                    stage: snapshot.homework.workflow_stage,
                    reason: outcome.command.clone(),
                    operation_id: meta.operation_id.clone(),
                },
            )?);
            let readiness = projections::readiness(&snapshot);
            events.push(event(
                names::HOMEWORK_READINESS_UPDATED,
                &ReadinessUpdatedPayload {
                    homework_id: outcome.homework_id.clone(),
                    ready: readiness.ready,
                    blockers: readiness.blockers(),
                    revision,
                },
            )?);
            events.push(event(
                names::OPERATION_PROGRESS,
                &crate::events::OperationProgressPayload {
                    operation_id: meta.operation_id.clone(),
                    kind: spec.kind,
                    progress: 1.0,
                    message: "Сохранено".to_owned(),
                },
            )?);
            events.push(event(
                names::OPERATION_COMPLETED,
                &OperationCompletedPayload {
                    operation: operation.clone(),
                },
            )?);
            Ok((value, operation, events))
        });
        match post {
            Ok((value, operation, events)) => Ok(WorkflowSuccess {
                value,
                operation,
                events,
                replayed: false,
            }),
            Err(error) => Err(WorkflowFailure::plain(AppError::new(
                error.code,
                "Данные сохранены, но результат не удалось прочитать — перечитайте состояние ДЗ",
            ))),
        }
    }

    fn replay<T>(
        &self,
        operation_id: &str,
        outcome: &StoredOutcome,
        read: impl Fn(&Connection, &EntityRef) -> Result<T, AppError>,
    ) -> WorkflowResult<T> {
        self.database
            .with_connection(|connection| {
                let value = read(connection, &outcome.entity)?;
                let operation = crate::repositories::operation_in(connection, operation_id)?;
                Ok(WorkflowSuccess {
                    value,
                    operation,
                    events: Vec::new(),
                    replayed: true,
                })
            })
            .map_err(WorkflowFailure::plain)
    }

    fn record_failure(
        &self,
        operation_id: &str,
        spec: &Spec,
        homework_id: Option<&str>,
        error: AppError,
    ) -> WorkflowFailure {
        let recorded = self.database.with_transaction(|transaction| {
            // ДЗ могло не найтись: ссылка на несуществующее ДЗ не записывается.
            let homework = match homework_id {
                Some(id) => transaction
                    .query_row("SELECT id FROM homeworks WHERE id = ?1", [id], |row| {
                        row.get::<_, String>(0)
                    })
                    .optional()?,
                None => None,
            };
            insert_failed(transaction, operation_id, spec, homework.as_deref(), &error)
        });
        let operation = recorded.ok().and_then(|()| {
            ReadRepository::new(self.database)
                .operation(operation_id)
                .ok()
                .map(Box::new)
        });
        let events = operation
            .as_ref()
            .and_then(|operation| {
                event(
                    names::OPERATION_FAILED,
                    &OperationFailedPayload {
                        operation: (**operation).clone(),
                        error: error.clone(),
                    },
                )
                .ok()
            })
            .into_iter()
            .collect();
        WorkflowFailure {
            error,
            operation,
            events,
        }
    }

    // ── Материалы ──

    /// Неизменяемый SourceBundle из подтверждённой подборки ДЗ.
    pub fn create_source_bundle(
        &self,
        meta: &CommandMeta,
        homework_id: &str,
        purpose: RunKind,
        version_id: Option<&str>,
    ) -> WorkflowResult<SourceBundleDto> {
        let spec = Spec {
            command: "create_source_bundle",
            kind: OperationKind::PrepareSources,
            title: "Снимок источников из подборки материалов".to_owned(),
        };
        let request = (homework_id, purpose, version_id);
        // Сверка с диском — до транзакции, без удержания соединения; повтор — без сверки.
        let hash = request_hash(spec.command, &(&request, &meta.expected_revision))
            .map_err(WorkflowFailure::plain)?;
        let prior = self
            .database
            .with_connection(|connection| prior_operation(connection, &meta.operation_id, &hash));
        let plan = match prior {
            Ok(Prior::Replay(_)) | Err(_) => None,
            Ok(Prior::None) => {
                match prepare_snapshot(self.database, homework_id, purpose, self.verifier) {
                    Ok(plan) => Some(plan),
                    Err(error) => {
                        validate_operation_id(&meta.operation_id)
                            .map_err(WorkflowFailure::plain)?;
                        let exists = self.database.with_connection(|connection| {
                            repository::homework_core(connection, homework_id).map(|_| ())
                        });
                        return Err(self.record_failure(
                            &meta.operation_id,
                            &spec,
                            exists.is_ok().then_some(homework_id),
                            error,
                        ));
                    }
                }
            }
        };
        self.execute(
            meta,
            spec,
            &request,
            Target::Homework(homework_id),
            |context| {
                let plan =
                    plan.ok_or_else(|| BackendError::conflict("Набор источников не подготовлен"))?;
                let stage = context.stage()?;
                if matches!(stage, Stage::ReadyToSubmit | Stage::Submitted) {
                    return Err(BackendError::blocked(
                        "Работа отмечена «Готово к сдаче» — сначала верните её на исправления",
                    ));
                }
                if let Some(version) = version_id {
                    if resolve_homework(context.connection, &Target::Version(version))?
                        != context.homework_id
                    {
                        return Err(BackendError::not_found("Версия не найдена"));
                    }
                }
                let id = write_snapshot(
                    context.connection,
                    &context.homework_id,
                    purpose,
                    version_id,
                    &plan,
                )?;
                Ok(EntityRef::Bundle { id })
            },
            read_bundle,
        )
    }

    // ── Запуски ──

    pub fn start_run(
        &self,
        meta: &CommandMeta,
        homework_id: &str,
        kind: RunKind,
        version_id: Option<&str>,
        bundle_id: &str,
        mode: RunMode,
    ) -> WorkflowResult<ReviewRunDto> {
        let spec = Spec {
            command: "start_workflow_run",
            kind: OperationKind::AiRun,
            title: match mode {
                RunMode::Provider => "Запуск ждёт подключения провайдера".to_owned(),
                RunMode::Manual => "Шаг передан на ручное выполнение".to_owned(),
            },
        };
        self.execute(
            meta,
            spec,
            &(homework_id, kind, version_id, bundle_id, mode),
            Target::Homework(homework_id),
            |context| {
                let snapshot = context.snapshot()?;
                let stage = snapshot.homework.workflow_stage;
                if !state_machine::run_allowed_in(kind, stage) {
                    return Err(BackendError::blocked(format!(
                        "Этот шаг недоступен на этапе «{}»",
                        state_machine::stage_label(stage)
                    )));
                }
                if state_machine::has_open_run(&snapshot) {
                    return Err(BackendError::conflict(
                        "У ДЗ уже есть незавершённый запуск — сохраните его результат или отмените",
                    ));
                }
                let (bundle, changes) = snapshot
                    .bundle(bundle_id)
                    .ok_or_else(|| BackendError::not_found("Набор источников не найден"))?;
                if bundle.purpose != kind {
                    return Err(BackendError::conflict(
                        "Набор источников подготовлен для другого шага",
                    ));
                }
                if !bundle.complete {
                    let labels: Vec<_> = bundle
                        .missing
                        .iter()
                        .filter(|item| item.required)
                        .map(|item| item.label.as_str())
                        .collect();
                    return Err(BackendError::blocked(format!(
                        "Не хватает обязательных материалов: {}",
                        labels.join(", ")
                    )));
                }
                if changes.iter().any(|changed| *changed) {
                    return Err(BackendError::blocked(
                        "Материалы изменились после снимка — создайте набор источников заново",
                    ));
                }
                let version_id = match kind {
                    RunKind::AnalyzeMaterials => None,
                    _ => {
                        let version = version_id
                            .ok_or_else(|| BackendError::blocked("Укажите проверяемую версию"))?;
                        if snapshot.current_version().map(|item| item.id.as_str()) != Some(version)
                        {
                            return Err(BackendError::conflict(
                                "Проверять можно только текущую выбранную версию",
                            ));
                        }
                        if bundle.version_id.as_deref() != Some(version) {
                            return Err(BackendError::conflict(
                                "Набор источников относится к другой версии",
                            ));
                        }
                        Some(version)
                    }
                };
                let role = role_for(kind);
                let sources: Vec<EvidenceReferenceDto> = bundle
                    .items
                    .iter()
                    .filter(|item| item.included)
                    .map(|item| EvidenceReferenceDto {
                        source_id: item.id.clone(),
                        file_name: item.file.name.clone(),
                        locator: "весь документ".to_owned(),
                    })
                    .collect();
                let id = Uuid::new_v4().to_string();
                let (status, step) = match mode {
                    RunMode::Provider => (
                        ReviewRunStatus::WaitingForProvider,
                        "Провайдер недоступен: AI-адаптер ещё не подключён",
                    ),
                    RunMode::Manual => (
                        ReviewRunStatus::ManualHandoff,
                        "Выполните шаг вручную и сохраните результат",
                    ),
                };
                repository::insert_run(
                    context.connection,
                    &repository::NewRun {
                        id: &id,
                        homework_id: &context.homework_id,
                        kind,
                        role,
                        provider: context.rules.workflow.tasks.provider(task_for_run(kind)),
                        version_id,
                        source_bundle_id: bundle_id,
                        sources_used: &sources,
                        status,
                        step,
                        style_threshold: applied_threshold(&context.rules)?,
                    },
                )?;
                if kind == RunKind::AnalyzeMaterials {
                    context.transition(Stage::CodexAnalysis, "Начат анализ материалов", false)?;
                }
                Ok(EntityRef::Run { id })
            },
            read_run,
        )
    }

    pub fn cancel_run(&self, meta: &CommandMeta, run_id: &str) -> WorkflowResult<ReviewRunDto> {
        self.execute(
            meta,
            Spec {
                command: "cancel_workflow_run",
                kind: OperationKind::AiRun,
                title: "Запуск отменён".to_owned(),
            },
            &run_id,
            Target::Run(run_id),
            |context| {
                repository::cancel_run(context.connection, run_id)?;
                Ok(EntityRef::Run {
                    id: run_id.to_owned(),
                })
            },
            read_run,
        )
    }

    pub fn pin_run(&self, meta: &CommandMeta, run_id: &str) -> WorkflowResult<ReviewRunDto> {
        self.execute(
            meta,
            Spec {
                command: "pin_workflow_run",
                kind: OperationKind::AiRun,
                title: "Результат закреплён в истории итераций".to_owned(),
            },
            &run_id,
            Target::Run(run_id),
            |context| {
                repository::pin_run(context.connection, run_id)?;
                Ok(EntityRef::Run {
                    id: run_id.to_owned(),
                })
            },
            read_run,
        )
    }

    /// Ручной результат анализа материалов и черновик первичного промта.
    pub fn save_manual_analysis(
        &self,
        meta: &CommandMeta,
        run_id: &str,
        input: &ManualAnalysisInput,
    ) -> WorkflowResult<ReviewRunDto> {
        self.execute(
            meta,
            Spec {
                command: "save_manual_analysis",
                kind: OperationKind::AiRun,
                title: "Результат анализа сохранён вручную".to_owned(),
            },
            &(run_id, input),
            Target::Run(run_id),
            |context| {
                apply_analysis(context, run_id, input, &ResultOrigin::manual())?;
                Ok(EntityRef::Run {
                    id: run_id.to_owned(),
                })
            },
            read_run,
        )
    }

    /// Ручной результат проверки версии (или по комментариям преподавателя).
    pub fn save_manual_review(
        &self,
        meta: &CommandMeta,
        run_id: &str,
        input: &ManualReviewInput,
    ) -> WorkflowResult<ReviewRunDto> {
        self.execute(
            meta,
            Spec {
                command: "save_manual_review",
                kind: OperationKind::AiRun,
                title: "Результат проверки сохранён вручную".to_owned(),
            },
            &(run_id, input),
            Target::Run(run_id),
            |context| {
                apply_review(context, run_id, input, &ResultOrigin::manual())?;
                Ok(EntityRef::Run {
                    id: run_id.to_owned(),
                })
            },
            read_run,
        )
    }

    // ── Промты ──

    /// Черновик промта исправлений: только выявленные проблемы конкретной проверки.
    pub fn create_revision_prompt(
        &self,
        meta: &CommandMeta,
        review_run_id: &str,
        edited_text: Option<&str>,
    ) -> WorkflowResult<RevisionPromptDto> {
        self.execute(
            meta,
            Spec {
                command: "create_revision_prompt",
                kind: OperationKind::RevisionPrompt,
                title: "Черновик промта исправлений".to_owned(),
            },
            &(review_run_id, edited_text),
            Target::Run(review_run_id),
            |context| {
                let id = apply_revision_prompt(
                    context,
                    review_run_id,
                    edited_text,
                    VersionAuthor::User,
                    Provenance::Manual,
                )?;
                Ok(EntityRef::Prompt { id })
            },
            read_prompt,
        )
    }

    // ── Подтверждение результата провайдера (этап 9) ──

    /// Пользователь проверил ответ провайдера (возможно, поправил) и
    /// подтверждает его. Результат применяется теми же проверками, что и
    /// ручной ввод, но с provenance `provider`; задание закрывается с отметкой
    /// о правке. Без этого шага ответ AI ничего не меняет в рабочем цикле.
    pub fn confirm_ai_result(
        &self,
        meta: &CommandMeta,
        job_id: &str,
        supporting_job_id: Option<&str>,
        input: &AiConfirmInput,
    ) -> WorkflowResult<AiConfirmationDto> {
        self.execute(
            meta,
            Spec {
                command: "confirm_ai_result",
                kind: OperationKind::AiRun,
                title: "Результат провайдера подтверждён".to_owned(),
            },
            &(job_id, supporting_job_id, input),
            Target::AiJob(job_id),
            |context| {
                let (job, include_prompt) = ai::store::job_with_flags(context.connection, job_id)?;
                if job.status != JobStatus::ResponseReady {
                    return Err(BackendError::conflict(
                        "Подтвердить можно только полученный ответ, который ещё не применён",
                    ));
                }
                let author = |provider: crate::dto::ProviderId| match provider {
                    crate::dto::ProviderId::Codex => VersionAuthor::Codex,
                    crate::dto::ProviderId::Claude => VersionAuthor::Claude,
                };
                let step = match job.transport {
                    Transport::Cli => "Ответ провайдера (CLI) подтверждён пользователем",
                    Transport::ManualHandoff => "Ответ провайдера (ручная передача) подтверждён пользователем",
                };
                let draft = job.draft.clone().unwrap_or(Value::Null);
                let mut confirmed: Vec<(String, bool)> = Vec::new();
                let mut prompt_id: Option<String> = None;
                match (job.task, input) {
                    (AiTask::MaterialAnalysis, AiConfirmInput::Analysis(analysis)) => {
                        let supporting = match supporting_job_id {
                            Some(id) => {
                                let other = ai::store::job(context.connection, id)?;
                                if other.run_id != job.run_id
                                    || other.task != AiTask::InitialPrompt
                                    || other.status != JobStatus::ResponseReady
                                {
                                    return Err(BackendError::conflict(
                                        "Промт должен быть полученным ответом задачи «Первичный промт» того же запуска",
                                    ));
                                }
                                Some(other)
                            }
                            None => None,
                        };
                        let (prompt_author, prompt_provenance, prompt_draft) = match &supporting {
                            Some(other) => (
                                author(other.provider),
                                Provenance::Provider,
                                other.draft.clone().unwrap_or(Value::Null),
                            ),
                            // Промт приписывается провайдеру, только если он его действительно вернул.
                            None if draft
                                .get("promptText")
                                .and_then(Value::as_str)
                                .is_some_and(|text| !text.trim().is_empty()) =>
                            {
                                (author(job.provider), Provenance::Provider, draft.clone())
                            }
                            None => (VersionAuthor::User, Provenance::Manual, Value::Null),
                        };
                        apply_analysis(
                            context,
                            &job.run_id,
                            analysis,
                            &ResultOrigin {
                                provenance: Provenance::Provider,
                                step,
                                prompt_author,
                                prompt_provenance,
                            },
                        )?;
                        confirmed.push((
                            job.id.clone(),
                            text_differs(&draft, "summary", &analysis.summary),
                        ));
                        if let Some(other) = supporting {
                            confirmed.push((
                                other.id,
                                text_differs(&prompt_draft, "promptText", &analysis.prompt_text),
                            ));
                        } else if include_prompt {
                            confirmed[0].1 |= text_differs(&draft, "promptText", &analysis.prompt_text);
                        }
                    }
                    (AiTask::SolutionReview | AiTask::TeacherReview, AiConfirmInput::Review(review)) => {
                        if supporting_job_id.is_some() {
                            return Err(BackendError::conflict("Для проверки дополнительный ответ не используется"));
                        }
                        apply_review(
                            context,
                            &job.run_id,
                            review,
                            &ResultOrigin {
                                provenance: Provenance::Provider,
                                step,
                                prompt_author: VersionAuthor::User,
                                prompt_provenance: Provenance::Manual,
                            },
                        )?;
                        confirmed.push((job.id.clone(), review_differs(&draft, review)));
                    }
                    (AiTask::RevisionPrompt, AiConfirmInput::Prompt { text }) => {
                        if supporting_job_id.is_some() {
                            return Err(BackendError::conflict("Для промта дополнительный ответ не используется"));
                        }
                        let id = apply_revision_prompt(
                            context,
                            &job.run_id,
                            Some(text),
                            author(job.provider),
                            Provenance::Provider,
                        )?;
                        prompt_id = Some(id);
                        confirmed.push((job.id.clone(), text_differs(&draft, "text", text)));
                    }
                    (AiTask::InitialPrompt, _) => {
                        return Err(BackendError::conflict(
                            "Первичный промт подтверждается вместе с итогом анализа материалов",
                        ))
                    }
                    _ => {
                        return Err(BackendError::conflict(
                            "Формат подтверждения не соответствует задаче",
                        ))
                    }
                }
                if prompt_id.is_none() && matches!(job.task, AiTask::MaterialAnalysis) {
                    prompt_id = repository::prompts(context.connection, &context.homework_id)?
                        .into_iter()
                        .find(|prompt| prompt.review_run_id.as_deref() == Some(job.run_id.as_str()))
                        .map(|prompt| prompt.id);
                }
                for (id, edited) in &confirmed {
                    ai::store::confirm_job(context.connection, id, *edited, prompt_id.as_deref())?;
                }
                if job.task != AiTask::RevisionPrompt {
                    let keep: Vec<&str> = confirmed.iter().map(|(id, _)| id.as_str()).collect();
                    ai::store::close_other_jobs(context.connection, &job.run_id, &keep)?;
                }
                Ok(EntityRef::AiJob { id: job.id })
            },
            read_confirmation_outcome,
        )
    }

    pub fn update_prompt_draft(
        &self,
        meta: &CommandMeta,
        prompt_id: &str,
        text: &str,
    ) -> WorkflowResult<RevisionPromptDto> {
        self.execute(
            meta,
            Spec {
                command: "update_prompt_draft",
                kind: OperationKind::RevisionPrompt,
                title: "Черновик промта изменён".to_owned(),
            },
            &(prompt_id, text),
            Target::Prompt(prompt_id),
            |context| {
                text_field(text, "Текст промта", 100_000, true)?;
                repository::update_draft_text(context.connection, prompt_id, text.trim())?;
                Ok(EntityRef::Prompt {
                    id: prompt_id.to_owned(),
                })
            },
            read_prompt,
        )
    }

    pub fn commit_prompt(
        &self,
        meta: &CommandMeta,
        prompt_id: &str,
    ) -> WorkflowResult<RevisionPromptDto> {
        self.execute(
            meta,
            Spec {
                command: "commit_prompt",
                kind: OperationKind::RevisionPrompt,
                title: "Промт зафиксирован".to_owned(),
            },
            &prompt_id,
            Target::Prompt(prompt_id),
            |context| {
                let prompt = repository::prompt(context.connection, prompt_id)?;
                repository::commit_prompt(context.connection, prompt_id)?;
                if prompt.kind == PromptKind::Initial && context.stage()? == Stage::CodexAnalysis {
                    context.transition(
                        Stage::PromptReady,
                        "Первичный промт для автора зафиксирован",
                        false,
                    )?;
                }
                Ok(EntityRef::Prompt {
                    id: prompt_id.to_owned(),
                })
            },
            read_prompt,
        )
    }

    /// Ручная передача зафиксированного промта автору (адаптера ещё нет).
    pub fn hand_off_prompt(
        &self,
        meta: &CommandMeta,
        prompt_id: &str,
    ) -> WorkflowResult<RevisionPromptDto> {
        self.execute(
            meta,
            Spec {
                command: "hand_off_prompt",
                kind: OperationKind::SendPrompt,
                title: "Промт передан автору вручную".to_owned(),
            },
            &prompt_id,
            Target::Prompt(prompt_id),
            |context| {
                let prompt = repository::prompt(context.connection, prompt_id)?;
                if prompt.status != PromptStatus::Committed {
                    return Err(BackendError::blocked("Сначала зафиксируйте промт"));
                }
                if prompt.stale {
                    return Err(BackendError::blocked(
                        "Промт составлен по устаревшим входным данным (требования, материалы, условие, версия или профиль стиля изменились) — создайте новый",
                    ));
                }
                let snapshot = context.snapshot()?;
                let stage = snapshot.homework.workflow_stage;
                let allowed = match prompt.kind {
                    PromptKind::Initial => stage == Stage::PromptReady,
                    PromptKind::Revision | PromptKind::Teacher => {
                        stage == Stage::NeedsFixes
                            && prompt.based_on_version_id.as_deref()
                                == snapshot.homework.final_version_id.as_deref()
                    }
                };
                if !allowed {
                    return Err(BackendError::blocked(format!(
                        "Этот промт нельзя передать на этапе «{}»",
                        state_machine::stage_label(stage)
                    )));
                }
                repository::mark_prompt_handed_off(context.connection, prompt_id)?;
                context.transition(Stage::ClaudeSolving, "Промт передан автору вручную", false)?;
                Ok(EntityRef::Prompt {
                    id: prompt_id.to_owned(),
                })
            },
            read_prompt,
        )
    }

    // ── Версии ──

    pub fn create_version(
        &self,
        meta: &CommandMeta,
        homework_id: &str,
        input: &VersionInput,
    ) -> WorkflowResult<HomeworkVersionDto> {
        self.execute(
            meta,
            Spec {
                command: "create_homework_version",
                kind: OperationKind::CreateVersion,
                title: "Новая версия решения".to_owned(),
            },
            &(homework_id, input),
            Target::Homework(homework_id),
            |context| {
                if let Some(note) = &input.note {
                    text_field(note, "Заметка к версии", 2_000, false)?;
                }
                let snapshot = context.snapshot()?;
                let stage = snapshot.homework.workflow_stage;
                if !matches!(stage, Stage::ClaudeSolving | Stage::CodexReview) {
                    return Err(BackendError::blocked(format!(
                        "Новую версию можно добавить после передачи промта автору, сейчас этап «{}»",
                        state_machine::stage_label(stage)
                    )));
                }
                let main = repository::homework_file(context.connection, &context.homework_id, &input.file_id)?;
                let mut files = vec![main.clone()];
                for id in &input.attachment_file_ids {
                    if files.iter().any(|file| &file.id == id) {
                        return Err(BackendError::conflict("Файл указан в версии дважды"));
                    }
                    files.push(repository::homework_file(context.connection, &context.homework_id, id)?);
                }
                let prompt_id = match &input.prompt_id {
                    Some(id) => {
                        let prompt = snapshot
                            .prompts
                            .iter()
                            .find(|prompt| &prompt.id == id)
                            .ok_or_else(|| BackendError::not_found("Промт не найден"))?;
                        if prompt.status != PromptStatus::Committed {
                            return Err(BackendError::blocked("Версия связывается только с зафиксированным промтом"));
                        }
                        Some(id.clone())
                    }
                    // Последний переданный автору промт, ещё не связанный с версией.
                    None => snapshot
                        .prompts
                        .iter()
                        .filter(|prompt| prompt.sent_at.is_some())
                        .find(|prompt| {
                            !snapshot
                                .homework
                                .versions
                                .iter()
                                .any(|version| version.prompt_id.as_deref() == Some(prompt.id.as_str()))
                        })
                        .map(|prompt| prompt.id.clone()),
                };
                let id = Uuid::new_v4().to_string();
                repository::insert_version(
                    context.connection,
                    &repository::NewVersion {
                        id: &id,
                        homework_id: &context.homework_id,
                        main: &main,
                        files: &files,
                        note: input.note.as_deref().map(str::trim).filter(|note| !note.is_empty()),
                        author: input.author.unwrap_or(VersionAuthor::User),
                        prompt_id: prompt_id.as_deref(),
                        source_bundle_id: None,
                    },
                )?;
                repository::set_current_version(context.connection, &context.homework_id, &id)?;
                context.pending.push(Pending::VersionCreated(id.clone()));
                if stage == Stage::ClaudeSolving {
                    context.transition(Stage::CodexReview, "Создана новая версия — ожидает проверки", false)?;
                }
                Ok(EntityRef::Version { id })
            },
            read_version,
        )
    }

    /// Выбор текущей проверяемой версии (она же итоговая до снимка этапа 10).
    pub fn select_version(
        &self,
        meta: &CommandMeta,
        version_id: &str,
    ) -> WorkflowResult<HomeworkDto> {
        self.execute(
            meta,
            Spec {
                command: "select_homework_version",
                kind: OperationKind::MarkFinal,
                title: "Выбрана проверяемая версия".to_owned(),
            },
            &version_id,
            Target::Version(version_id),
            |context| {
                let stage = context.stage()?;
                if !matches!(
                    stage,
                    Stage::ClaudeSolving
                        | Stage::CodexReview
                        | Stage::NeedsFixes
                        | Stage::TeacherReview
                ) {
                    return Err(BackendError::blocked(format!(
                        "Версию нельзя выбрать на этапе «{}»",
                        state_machine::stage_label(stage)
                    )));
                }
                repository::set_current_version(
                    context.connection,
                    &context.homework_id,
                    version_id,
                )?;
                Ok(EntityRef::Homework {
                    id: context.homework_id.clone(),
                })
            },
            read_homework,
        )
    }

    // ── Преподаватель ──

    pub fn add_teacher_comments(
        &self,
        meta: &CommandMeta,
        homework_id: &str,
        comments: &[TeacherCommentInput],
    ) -> WorkflowResult<Vec<TeacherCommentDto>> {
        self.execute(
            meta,
            Spec {
                command: "add_teacher_comments",
                kind: OperationKind::SaveTeacherComments,
                title: "Комментарии преподавателя добавлены вручную".to_owned(),
            },
            &(homework_id, comments),
            Target::Homework(homework_id),
            |context| {
                if comments.is_empty() || comments.len() > 50 {
                    return Err(BackendError::blocked(
                        "Добавьте от 1 до 50 комментариев за раз",
                    ));
                }
                let mut ids = Vec::with_capacity(comments.len());
                for comment in comments {
                    text_field(&comment.date, "Дата", 40, true)?;
                    text_field(&comment.question, "Вопрос", 10_000, false)?;
                    text_field(&comment.answer, "Ответ", 10_000, false)?;
                    if comment.question.trim().is_empty() && comment.answer.trim().is_empty() {
                        return Err(BackendError::blocked(
                            "Комментарий пуст: заполните вопрос или ответ",
                        ));
                    }
                    validate_authority(
                        comment.authority,
                        &comment.author,
                        &comment.context,
                        comment.confirmed_official,
                    )?;
                    let id = Uuid::new_v4().to_string();
                    repository::insert_teacher_comment(
                        context.connection,
                        &repository::NewTeacherComment {
                            id: &id,
                            homework_id: &context.homework_id,
                            source: comment.source,
                            date: comment.date.trim(),
                            question: comment.question.trim(),
                            answer: comment.answer.trim(),
                            authority: comment.authority,
                            author: comment.author.trim(),
                            context: comment.context.trim(),
                            official: comment.confirmed_official,
                        },
                    )?;
                    ids.push(id);
                }
                context.reopen_if_ready_stale("Добавлен официальный ответ — готовность снята")?;
                Ok(EntityRef::Comments { ids })
            },
            read_comments,
        )
    }

    pub fn add_teacher_requirement(
        &self,
        meta: &CommandMeta,
        homework_id: &str,
        input: &TeacherRequirementInput,
    ) -> WorkflowResult<TeacherRequirementDto> {
        self.execute(
            meta,
            Spec {
                command: "add_teacher_requirement",
                kind: OperationKind::SaveTeacherComments,
                title: "Требование преподавателя добавлено вручную".to_owned(),
            },
            &(homework_id, input),
            Target::Homework(homework_id),
            |context| {
                text_field(&input.text, "Требование", 2_000, true)?;
                text_field(&input.origin, "Источник требования", 2_000, false)?;
                let note = input.note.as_deref().unwrap_or_default();
                text_field(note, "Заметка", 2_000, false)?;
                let version = repository::homework_core(context.connection, &context.homework_id)?
                    .final_version_id
                    .ok_or_else(|| {
                        BackendError::blocked("Сначала добавьте и выберите версию решения")
                    })?;
                let id = Uuid::new_v4().to_string();
                repository::insert_requirement(
                    context.connection,
                    &repository::NewRequirement {
                        id: &id,
                        homework_id: &context.homework_id,
                        comment_id: &input.comment_id,
                        text: input.text.trim(),
                        origin: input.origin.trim(),
                        status: input.status,
                        note: note.trim(),
                        checked_version_id: &version,
                    },
                )?;
                context
                    .reopen_if_ready_stale("Добавлено официальное требование — готовность снята")?;
                Ok(EntityRef::Requirement { id })
            },
            read_requirement,
        )
    }

    pub fn resolve_requirement(
        &self,
        meta: &CommandMeta,
        requirement_id: &str,
        status: RequirementStatus,
        note: &str,
    ) -> WorkflowResult<TeacherRequirementDto> {
        self.execute(
            meta,
            Spec {
                command: "resolve_teacher_requirement",
                kind: OperationKind::SaveTeacherComments,
                title: "Требование преподавателя проверено вручную".to_owned(),
            },
            &(requirement_id, status, note),
            Target::Requirement(requirement_id),
            |context| {
                if status == RequirementStatus::Manual {
                    return Err(BackendError::blocked(
                        "Выберите итог: выполнено, не выполнено или не применимо",
                    ));
                }
                text_field(note, "Заметка", 2_000, false)?;
                let version = repository::homework_core(context.connection, &context.homework_id)?
                    .final_version_id
                    .ok_or_else(|| BackendError::blocked("Сначала выберите проверяемую версию"))?;
                repository::resolve_requirement(
                    context.connection,
                    requirement_id,
                    status,
                    note.trim(),
                    &version,
                )?;
                if status == RequirementStatus::NotDone && context.stage()? == Stage::ReadyToSubmit
                {
                    context.transition(
                        Stage::TeacherReview,
                        "Требование отмечено невыполненным — готовность снята",
                        false,
                    )?;
                }
                Ok(EntityRef::Requirement {
                    id: requirement_id.to_owned(),
                })
            },
            read_requirement,
        )
    }

    /// Этап 7: автор, роль и официальность ответа. Изменение официальности
    /// меняет отпечаток входных данных — проверки, промты, готовность и
    /// комплекты, созданные раньше, устаревают.
    pub fn set_comment_authority(
        &self,
        meta: &CommandMeta,
        comment_id: &str,
        input: &CommentAuthorityInput,
    ) -> WorkflowResult<Vec<TeacherCommentDto>> {
        self.execute(
            meta,
            Spec {
                command: "set_comment_authority",
                kind: OperationKind::SaveTeacherComments,
                title: if input.confirmed_official {
                    "Ответ подтверждён как официальный".to_owned()
                } else {
                    "Ответ не считается официальным".to_owned()
                },
            },
            &(comment_id, input),
            Target::Comment(comment_id),
            |context| {
                validate_authority(
                    input.authority,
                    &input.author,
                    &input.context,
                    input.confirmed_official,
                )?;
                repository::set_comment_authority(
                    context.connection,
                    comment_id,
                    input.authority,
                    input.author.trim(),
                    input.context.trim(),
                    input.confirmed_official,
                )?;
                context
                    .reopen_if_ready_stale("Изменилась официальность ответа — готовность снята")?;
                Ok(EntityRef::Comments {
                    ids: vec![comment_id.to_owned()],
                })
            },
            read_comments,
        )
    }

    /// Этап 7: требование удалено пользователем (остаётся в истории как отозванное).
    pub fn withdraw_requirement(
        &self,
        meta: &CommandMeta,
        requirement_id: &str,
    ) -> WorkflowResult<TeacherRequirementDto> {
        self.execute(
            meta,
            Spec {
                command: "withdraw_teacher_requirement",
                kind: OperationKind::SaveTeacherComments,
                title: "Требование отозвано".to_owned(),
            },
            &requirement_id,
            Target::Requirement(requirement_id),
            |context| {
                repository::withdraw_requirement(context.connection, requirement_id)?;
                context.reopen_if_ready_stale("Требование отозвано — готовность снята")?;
                Ok(EntityRef::Requirement {
                    id: requirement_id.to_owned(),
                })
            },
            read_requirement,
        )
    }

    // ── Готовность ──

    /// Явное подтверждение пользователем расхождения со стилем выше порога.
    pub fn confirm_style(
        &self,
        meta: &CommandMeta,
        review_run_id: &str,
    ) -> WorkflowResult<StyleConfirmationDto> {
        self.execute(
            meta,
            Spec {
                command: "confirm_style_divergence",
                kind: OperationKind::ConfirmStyle,
                title: "Расхождение с личным стилем подтверждено пользователем".to_owned(),
            },
            &review_run_id,
            Target::Run(review_run_id),
            |context| {
                let run = repository::run(context.connection, review_run_id)?;
                let assessment = run
                    .assessment
                    .as_ref()
                    .filter(|_| run.kind == RunKind::ReviewVersion)
                    .ok_or_else(|| {
                        BackendError::blocked(
                            "Подтверждение относится к завершённой проверке версии",
                        )
                    })?;
                let current = repository::homework_core(context.connection, &context.homework_id)?
                    .final_version_id;
                if run.version_id != current {
                    return Err(BackendError::conflict(
                        "Проверка относится не к текущей версии",
                    ));
                }
                let score = assessment
                    .style_score
                    .ok_or_else(|| BackendError::blocked("Расхождение со стилем не оценивалось"))?;
                if score <= run.style_threshold {
                    return Err(BackendError::blocked(
                        "Расхождение не выше порога — подтверждение не нужно",
                    ));
                }
                let id = Uuid::new_v4().to_string();
                repository::insert_style_confirmation(context.connection, &id, &run, score)?;
                Ok(EntityRef::Confirmation { id })
            },
            read_confirmation,
        )
    }

    /// «Готово к сдаче» — только явным действием и только при пройденной готовности.
    pub fn mark_ready(
        &self,
        meta: &CommandMeta,
        homework_id: &str,
        no_teacher_comments: bool,
    ) -> WorkflowResult<HomeworkDto> {
        self.execute(
            meta,
            Spec {
                command: "mark_ready_to_submit",
                kind: OperationKind::MarkReady,
                title: "Работа отмечена «Готово к сдаче»".to_owned(),
            },
            &(homework_id, no_teacher_comments),
            Target::Homework(homework_id),
            |context| {
                context.transition(
                    Stage::ReadyToSubmit,
                    "Все обязательные проверки пройдены, подтверждено пользователем",
                    no_teacher_comments,
                )?;
                let inputs = super::fingerprint::current(context.connection, &context.homework_id)?;
                repository::set_ready_fingerprint(
                    context.connection,
                    &context.homework_id,
                    Some(&inputs.combined),
                )?;
                Ok(EntityRef::Homework {
                    id: context.homework_id.clone(),
                })
            },
            read_homework,
        )
    }

    /// Явный допустимый переход (возвраты и решения пользователя).
    pub fn transition(
        &self,
        meta: &CommandMeta,
        homework_id: &str,
        to: Stage,
        reason: Option<&str>,
    ) -> WorkflowResult<HomeworkDto> {
        self.execute(
            meta,
            Spec {
                command: "transition_workflow",
                kind: OperationKind::WorkflowTransition,
                title: format!("Этап: {}", state_machine::stage_label(to)),
            },
            &(homework_id, to, reason),
            Target::Homework(homework_id),
            |context| {
                if to == Stage::ReadyToSubmit {
                    return Err(BackendError::blocked(
                        "«Готово к сдаче» отмечается отдельным действием с проверкой готовности",
                    ));
                }
                let reason = reason.map(str::trim).filter(|value| !value.is_empty());
                if let Some(reason) = reason {
                    text_field(reason, "Причина", 500, false)?;
                }
                context.transition(
                    to,
                    reason.unwrap_or("Переход выполнен пользователем"),
                    false,
                )?;
                Ok(EntityRef::Homework {
                    id: context.homework_id.clone(),
                })
            },
            read_homework,
        )
    }
}

// ───────────────────────── Чтение итогов ─────────────────────────

fn read_bundle(connection: &Connection, entity: &EntityRef) -> Result<SourceBundleDto, AppError> {
    match entity {
        EntityRef::Bundle { id } => Ok(repository::bundle(connection, id)?.0),
        _ => Err(BackendError::database("unexpected operation result")),
    }
}

fn read_run(connection: &Connection, entity: &EntityRef) -> Result<ReviewRunDto, AppError> {
    match entity {
        EntityRef::Run { id } => repository::run(connection, id),
        _ => Err(BackendError::database("unexpected operation result")),
    }
}

fn read_prompt(connection: &Connection, entity: &EntityRef) -> Result<RevisionPromptDto, AppError> {
    match entity {
        EntityRef::Prompt { id } => repository::prompt(connection, id),
        _ => Err(BackendError::database("unexpected operation result")),
    }
}

fn read_version(
    connection: &Connection,
    entity: &EntityRef,
) -> Result<HomeworkVersionDto, AppError> {
    let EntityRef::Version { id } = entity else {
        return Err(BackendError::database("unexpected operation result"));
    };
    let homework_id = resolve_homework(connection, &Target::Version(id))?;
    repository::versions(connection, &homework_id)?
        .into_iter()
        .find(|version| &version.id == id)
        .ok_or_else(|| BackendError::not_found("Версия не найдена"))
}

fn read_homework(connection: &Connection, entity: &EntityRef) -> Result<HomeworkDto, AppError> {
    match entity {
        EntityRef::Homework { id } => repository::homework_dto(connection, id),
        _ => Err(BackendError::database("unexpected operation result")),
    }
}

fn read_comments(
    connection: &Connection,
    entity: &EntityRef,
) -> Result<Vec<TeacherCommentDto>, AppError> {
    let EntityRef::Comments { ids } = entity else {
        return Err(BackendError::database("unexpected operation result"));
    };
    let Some(first) = ids.first() else {
        return Ok(Vec::new());
    };
    let homework_id: String = connection
        .query_row(
            "SELECT homework_id FROM teacher_comments WHERE id = ?1",
            [first],
            |row| row.get(0),
        )
        .map_err(|error| repository::not_found_or(error, "Комментарий не найден"))?;
    Ok(repository::teacher_comments(connection, &homework_id)?
        .into_iter()
        .filter(|comment| ids.contains(&comment.id))
        .collect())
}

fn read_requirement(
    connection: &Connection,
    entity: &EntityRef,
) -> Result<TeacherRequirementDto, AppError> {
    let EntityRef::Requirement { id } = entity else {
        return Err(BackendError::database("unexpected operation result"));
    };
    let homework_id = repository::requirement_homework(connection, id)?;
    repository::teacher_requirements(connection, &homework_id)?
        .into_iter()
        .find(|item| &item.id == id)
        .ok_or_else(|| BackendError::not_found("Требование не найдено"))
}

fn read_confirmation(
    connection: &Connection,
    entity: &EntityRef,
) -> Result<StyleConfirmationDto, AppError> {
    let EntityRef::Confirmation { id } = entity else {
        return Err(BackendError::database("unexpected operation result"));
    };
    let homework_id: String = connection
        .query_row(
            "SELECT homework_id FROM style_confirmations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .map_err(|error| repository::not_found_or(error, "Подтверждение не найдено"))?;
    repository::style_confirmations(connection, &homework_id)?
        .into_iter()
        .find(|item| &item.id == id)
        .ok_or_else(|| BackendError::not_found("Подтверждение не найдено"))
}

// ───────────────────────── Payload событий ─────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkUpdatedPayload {
    pub homework_id: String,
    pub revision: i64,
    pub stage: Stage,
    pub reason: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessUpdatedPayload {
    pub homework_id: String,
    pub ready: bool,
    pub blockers: Vec<ReadinessCheckId>,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPromptCreatedPayload {
    pub homework_id: String,
    pub prompt_id: String,
    pub kind: PromptKind,
    pub status: PromptStatus,
    pub review_run_id: Option<String>,
    pub based_on_version_id: Option<String>,
}

// ───────────────────────── Подтверждение ответа AI ─────────────────────────

/// Что подтверждает пользователь: те же поля, что у ручного ввода.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum AiConfirmInput {
    Analysis(ManualAnalysisInput),
    Review(ManualReviewInput),
    Prompt { text: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfirmationDto {
    pub job: AiJobDto,
    pub run: ReviewRunDto,
    pub prompt: Option<RevisionPromptDto>,
}

fn read_confirmation_outcome(
    connection: &Connection,
    entity: &EntityRef,
) -> Result<AiConfirmationDto, AppError> {
    let EntityRef::AiJob { id } = entity else {
        return Err(BackendError::database("unexpected entity"));
    };
    let job = ai::store::job(connection, id)?;
    let run = repository::run(connection, &job.run_id)?;
    let prompt = match &job.prompt_id {
        Some(prompt_id) => Some(repository::prompt(connection, prompt_id)?),
        None => None,
    };
    Ok(AiConfirmationDto { job, run, prompt })
}

/// Поправил ли пользователь текстовое поле черновика.
fn text_differs(draft: &Value, field: &str, value: &str) -> bool {
    draft.get(field).and_then(Value::as_str).map(str::trim) != Some(value.trim())
}

/// Поправил ли пользователь проверку: сравниваются все поля ручного ввода.
fn review_differs(draft: &Value, input: &ManualReviewInput) -> bool {
    let Ok(Value::Object(confirmed)) = serde_json::to_value(input) else {
        return true;
    };
    confirmed.iter().any(|(key, value)| {
        let original = draft.get(key).cloned().unwrap_or(Value::Null);
        let empty = |item: &Value| match item {
            Value::Null => true,
            Value::Array(items) => items.is_empty(),
            _ => false,
        };
        if empty(value) && empty(&original) {
            return false;
        }
        match (value, &original) {
            (Value::Array(left), Value::Array(right)) => {
                // Сравниваются тексты: служебные поля (evidence) черновик не содержит.
                let texts = |items: &Vec<Value>| -> Vec<String> {
                    items
                        .iter()
                        .map(|item| {
                            item.get("text")
                                .or_else(|| item.get("label"))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .trim()
                                .to_owned()
                        })
                        .collect()
                };
                texts(left) != texts(right)
            }
            (Value::String(left), Value::String(right)) => left.trim() != right.trim(),
            _ => value != &original,
        }
    })
}

// ───────────────────────── Применение результатов ─────────────────────────

/// Откуда пришёл результат шага: введён вручную или получен от провайдера и
/// подтверждён пользователем (этап 9).
pub(crate) struct ResultOrigin {
    pub provenance: Provenance,
    pub step: &'static str,
    /// Кто написал текст первичного промта и его происхождение.
    pub prompt_author: VersionAuthor,
    pub prompt_provenance: Provenance,
}

impl ResultOrigin {
    pub(crate) fn manual() -> Self {
        Self {
            provenance: Provenance::Manual,
            step: "Результат сохранён вручную",
            prompt_author: VersionAuthor::User,
            prompt_provenance: Provenance::Manual,
        }
    }
}

fn apply_analysis(
    context: &mut TxContext<'_>,
    run_id: &str,
    input: &ManualAnalysisInput,
    origin: &ResultOrigin,
) -> Result<(), AppError> {
    text_field(&input.summary, "Итог анализа", 20_000, true)?;
    text_field(&input.prompt_text, "Промт для автора", 100_000, true)?;
    let run = repository::run(context.connection, run_id)?;
    if run.kind != RunKind::AnalyzeMaterials {
        return Err(BackendError::conflict("Этот запуск — не анализ материалов"));
    }
    if !run.status.is_open() {
        return Err(BackendError::conflict("Запуск уже завершён или отменён"));
    }
    let bundle_id = run
        .source_bundle_id
        .clone()
        .ok_or_else(|| BackendError::database("analysis run has no bundle"))?;
    let (bundle, _) = repository::bundle(context.connection, &bundle_id)?;
    let missing: Vec<&str> = bundle
        .missing
        .iter()
        .map(|item| item.label.as_str())
        .collect();
    let result = StoredReviewResult {
        verdict: ReviewVerdict::Pass,
        summary: input.summary.trim().to_owned(),
        criteria: vec![ReviewCriterionDto {
            id: CriterionId::Completeness,
            label: "Комплектность материалов".to_owned(),
            status: if missing.is_empty() {
                CriterionStatus::Pass
            } else {
                CriterionStatus::Warn
            },
            note: if missing.is_empty() {
                "Все материалы на месте".to_owned()
            } else {
                missing.join(", ")
            },
        }],
        critical: Vec::new(),
        remarks: Vec::new(),
        unsupported_claims: Vec::new(),
        style: None,
        teacher_requirements: Vec::new(),
        recommended_action: RecommendedActionDto {
            kind: RecommendedActionKind::SendPromptToAuthor,
            label: "Зафиксировать промт и передать автору".to_owned(),
        },
        missing: Vec::new(),
    };
    repository::complete_run(
        context.connection,
        &repository::ManualCompletion {
            run_id,
            result: &result,
            correctness: CriterionStatus::Na,
            task_fit: CriterionStatus::Na,
            course_grounding: CriterionStatus::Na,
            teacher: CriterionStatus::Na,
            style_score: None,
        },
        origin.provenance,
        origin.step,
    )?;
    let prompt_id = Uuid::new_v4().to_string();
    repository::insert_draft_prompt_with(
        context.connection,
        &repository::NewPrompt {
            id: &prompt_id,
            homework_id: &context.homework_id,
            kind: PromptKind::Initial,
            text: input.prompt_text.trim(),
            created_by: origin.prompt_author,
            based_on_version_id: None,
            review_run_id: Some(run_id),
            addresses: &[],
            sources: &run.sources_used,
        },
        origin.prompt_provenance,
    )?;
    context
        .pending
        .push(Pending::ReviewCompleted(run_id.to_owned()));
    context.pending.push(Pending::PromptCreated(prompt_id));
    Ok(())
}

fn apply_review(
    context: &mut TxContext<'_>,
    run_id: &str,
    input: &ManualReviewInput,
    origin: &ResultOrigin,
) -> Result<(), AppError> {
    validate_review_input(input)?;
    let run = repository::run(context.connection, run_id)?;
    if run.kind == RunKind::AnalyzeMaterials {
        return Err(BackendError::conflict(
            "Для анализа материалов сохраните итог анализа",
        ));
    }
    if !run.status.is_open() {
        return Err(BackendError::conflict("Запуск уже завершён или отменён"));
    }
    let version_id = run
        .version_id
        .clone()
        .ok_or_else(|| BackendError::database("review run has no version"))?;
    let snapshot = context.snapshot()?;
    let version = snapshot
        .homework
        .versions
        .iter()
        .find(|item| item.id == version_id)
        .ok_or_else(|| BackendError::database("review version is missing"))?;
    let result = build_review_result(input, &run, version, &snapshot.teacher_requirements);
    repository::complete_run(
        context.connection,
        &repository::ManualCompletion {
            run_id,
            result: &result,
            correctness: input.correctness,
            task_fit: input.task_fit,
            course_grounding: input.course_grounding,
            teacher: input.teacher,
            style_score: input.style_score,
        },
        origin.provenance,
        origin.step,
    )?;
    context
        .pending
        .push(Pending::ReviewCompleted(run_id.to_owned()));
    let needs_fixes = matches!(
        input.verdict,
        ReviewVerdict::NeedsFixes | ReviewVerdict::Blocked
    );
    let is_current = snapshot.homework.final_version_id.as_deref() == Some(version_id.as_str());
    match (run.kind, needs_fixes, is_current) {
        (RunKind::ReviewVersion, true, true) => context.transition(
            Stage::NeedsFixes,
            &format!("Проверка {}: нужны исправления", version.label),
            false,
        )?,
        (RunKind::ReviewVersion, false, true) if context.rules.workflow.require_teacher_review => {
            context.transition(
                Stage::TeacherReview,
                &format!(
                    "Проверка {} пройдена — сверьте с комментариями преподавателя",
                    version.label
                ),
                false,
            )?
        }
        (RunKind::ReviewTeacher, true, true) => context.transition(
            Stage::NeedsFixes,
            "Требования преподавателя выполнены не полностью",
            false,
        )?,
        _ => {}
    }
    Ok(())
}

fn apply_revision_prompt(
    context: &mut TxContext<'_>,
    review_run_id: &str,
    edited_text: Option<&str>,
    created_by: VersionAuthor,
    provenance: Provenance,
) -> Result<String, AppError> {
    let run = repository::run(context.connection, review_run_id)?;
    if run.kind == RunKind::AnalyzeMaterials || run.status != ReviewRunStatus::Completed {
        return Err(BackendError::blocked(
            "Промт исправлений создаётся по завершённой проверке",
        ));
    }
    let version_id = run
        .version_id
        .clone()
        .ok_or_else(|| BackendError::database("review run has no version"))?;
    let snapshot = context.snapshot()?;
    if snapshot.homework.final_version_id.as_deref() != Some(version_id.as_str()) {
        return Err(BackendError::conflict(
            "Проверка относится не к текущей версии — выберите версию или проверьте её заново",
        ));
    }
    let version = snapshot
        .homework
        .versions
        .iter()
        .find(|item| item.id == version_id)
        .ok_or_else(|| BackendError::database("review version is missing"))?;
    let stage = snapshot.homework.workflow_stage;
    if !matches!(stage, Stage::NeedsFixes | Stage::TeacherReview) {
        return Err(BackendError::blocked(format!(
            "Промт исправлений недоступен на этапе «{}»",
            state_machine::stage_label(stage)
        )));
    }
    let (text, addresses) = revision_text(&run, version, &snapshot.teacher_requirements)?;
    let text = match edited_text {
        Some(edited) => {
            text_field(edited, "Промт исправлений", 100_000, true)?;
            edited.trim().to_owned()
        }
        None => text,
    };
    let kind = if run.kind == RunKind::ReviewTeacher {
        PromptKind::Teacher
    } else {
        PromptKind::Revision
    };
    let id = Uuid::new_v4().to_string();
    repository::insert_draft_prompt_with(
        context.connection,
        &repository::NewPrompt {
            id: &id,
            homework_id: &context.homework_id,
            kind,
            text: &text,
            created_by,
            based_on_version_id: Some(&version_id),
            review_run_id: Some(review_run_id),
            addresses: &addresses,
            sources: &run.sources_used,
        },
        provenance,
    )?;
    if stage == Stage::TeacherReview {
        context.transition(
            Stage::NeedsFixes,
            "Создан промт исправлений по требованиям преподавателя",
            false,
        )?;
    }
    context.pending.push(Pending::PromptCreated(id.clone()));
    Ok(id)
}

// ───────────────────────── Сборка результатов ─────────────────────────

fn aggregate_dto(snapshot: WorkflowSnapshot, rules: &AutomationRulesDto) -> HomeworkWorkflowDto {
    let readiness = projections::readiness(&snapshot);
    let iterations = projections::iterations(&snapshot);
    let stage = snapshot.homework.workflow_stage;
    let ready_stale = stage == Stage::ReadyToSubmit
        && !snapshot
            .inputs
            .is_current(snapshot.ready_fingerprint.as_deref());
    let transitions = state_machine::transition_options(
        stage,
        &TransitionContext {
            snapshot: &snapshot,
            readiness: &readiness,
            require_teacher_review: rules.workflow.require_teacher_review,
            no_teacher_comments: false,
        },
    );
    HomeworkWorkflowDto {
        revision: snapshot.homework.revision,
        stage,
        transitions,
        readiness,
        iterations,
        homework: snapshot.homework,
        bundles: snapshot.bundles,
        runs: snapshot.runs,
        prompts: snapshot.prompts,
        teacher_comments: snapshot.teacher_comments,
        teacher_requirements: snapshot.teacher_requirements,
        style_confirmations: snapshot.style_confirmations,
        submission_package: snapshot.submission_package,
        submission_receipt: snapshot.submission_receipt,
        ai_jobs: snapshot.ai_jobs,
        provider_mode: ProviderModeDto::AiJobs,
        ready_stale,
        inputs: snapshot.inputs,
    }
}

fn validate_review_input(input: &ManualReviewInput) -> Result<(), AppError> {
    text_field(&input.summary, "Итог проверки", 20_000, true)?;
    if input.style_score.is_some_and(|score| score > 10) {
        return Err(BackendError::blocked(
            "Расхождение со стилем — число от 0 до 10",
        ));
    }
    let total = input.critical.len()
        + input.remarks.len()
        + input.unsupported_claims.len()
        + input.missing.len();
    if total > 400 || input.style_reasons.len() > 50 {
        return Err(BackendError::blocked(
            "Слишком много замечаний в одном результате",
        ));
    }
    for finding in input.critical.iter().chain(input.remarks.iter()) {
        text_field(&finding.text, "Замечание", 2_000, true)?;
        if let Some(location) = &finding.location {
            text_field(location, "Место", 500, false)?;
        }
        if finding.evidence.len() > 20 {
            return Err(BackendError::blocked(
                "Слишком много ссылок на материалы в одном замечании",
            ));
        }
    }
    for claim in &input.unsupported_claims {
        text_field(&claim.text, "Неподтверждённое утверждение", 2_000, true)?;
        text_field(&claim.location, "Место", 500, false)?;
    }
    for item in &input.missing {
        text_field(&item.label, "Отсутствующий файл или ссылка", 500, true)?;
    }
    for reason in &input.style_reasons {
        text_field(&reason.text, "Причина расхождения", 1_000, true)?;
    }
    let passes = matches!(
        input.verdict,
        ReviewVerdict::Pass | ReviewVerdict::NeedsTeacherReview
    );
    if passes && !input.critical.is_empty() {
        return Err(BackendError::blocked(
            "Есть критические ошибки — итоговое решение не может быть «пройдено»",
        ));
    }
    if passes && input.missing.iter().any(|item| item.required) {
        return Err(BackendError::blocked(
            "Не хватает обязательных файлов или ссылок — итоговое решение не может быть «пройдено»",
        ));
    }
    Ok(())
}

fn criterion(
    id: CriterionId,
    label: &str,
    status: CriterionStatus,
    note: String,
) -> ReviewCriterionDto {
    ReviewCriterionDto {
        id,
        label: label.to_owned(),
        status,
        note,
    }
}

fn build_review_result(
    input: &ManualReviewInput,
    run: &ReviewRunDto,
    version: &HomeworkVersionDto,
    requirements: &[TeacherRequirementDto],
) -> StoredReviewResult {
    let threshold = run.style_threshold;
    let finding = |item: &FindingInput, severity: FindingSeverity| ReviewFindingDto {
        id: Uuid::new_v4().to_string(),
        severity,
        criterion: item.criterion,
        text: item.text.trim().to_owned(),
        location: item
            .location
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        evidence: item.evidence.clone(),
        introduced_in: version.id.clone(),
        fixed_in: None,
    };
    let critical: Vec<_> = input
        .critical
        .iter()
        .map(|item| finding(item, FindingSeverity::Critical))
        .collect();
    let remarks: Vec<_> = input
        .remarks
        .iter()
        .map(|item| finding(item, FindingSeverity::Normal))
        .collect();
    let unsupported: Vec<_> = input
        .unsupported_claims
        .iter()
        .map(|claim| UnsupportedClaimDto {
            id: Uuid::new_v4().to_string(),
            text: claim.text.trim().to_owned(),
            location: claim.location.trim().to_owned(),
        })
        .collect();
    let style = input.style_score.map(|score| StyleDivergenceDto {
        score,
        threshold,
        reasons: input.style_reasons.clone(),
    });
    let missing_required = input.missing.iter().filter(|item| item.required).count();
    let criteria = vec![
        criterion(
            CriterionId::Correctness,
            "Корректность решения",
            input.correctness,
            format!("Критических ошибок: {}", critical.len()),
        ),
        criterion(
            CriterionId::TaskFit,
            "Соответствие условию",
            input.task_fit,
            String::new(),
        ),
        criterion(
            CriterionId::CourseGrounding,
            "Опора на материалы курса",
            input.course_grounding,
            format!("Неподтверждённых утверждений: {}", unsupported.len()),
        ),
        criterion(
            CriterionId::Style,
            "Расхождение с личным стилем",
            match input.style_score {
                None => CriterionStatus::Na,
                Some(score) if score <= threshold => CriterionStatus::Pass,
                Some(_) => CriterionStatus::Fail,
            },
            match input.style_score {
                Some(score) => format!("Расхождение {score}/10 при применённом пороге {threshold}"),
                None => "Не оценивалось".to_owned(),
            },
        ),
        criterion(
            CriterionId::Teacher,
            "Требования преподавателя",
            input.teacher,
            String::new(),
        ),
        criterion(
            CriterionId::Completeness,
            "Файлы и ссылки",
            if missing_required == 0 {
                CriterionStatus::Pass
            } else {
                CriterionStatus::Fail
            },
            format!("Не хватает обязательных: {missing_required}"),
        ),
    ];
    let recommended_action = match input.verdict {
        ReviewVerdict::NeedsFixes | ReviewVerdict::Blocked => RecommendedActionDto {
            kind: RecommendedActionKind::CreateRevisionPrompt,
            label: "Создать промт исправлений".to_owned(),
        },
        ReviewVerdict::NeedsTeacherReview => RecommendedActionDto {
            kind: RecommendedActionKind::AddTeacherComments,
            label: "Сверить с комментариями преподавателя".to_owned(),
        },
        ReviewVerdict::Pass => RecommendedActionDto {
            kind: RecommendedActionKind::MarkReady,
            label: "Проверить готовность к сдаче".to_owned(),
        },
    };
    StoredReviewResult {
        verdict: input.verdict,
        summary: input.summary.trim().to_owned(),
        criteria,
        critical,
        remarks,
        unsupported_claims: unsupported,
        style,
        teacher_requirements: requirements
            .iter()
            .filter(|item| item.checked_version_id.as_deref() == Some(version.id.as_str()))
            .cloned()
            .collect(),
        recommended_action,
        missing: input.missing.clone(),
    }
}

/// Текст промта исправлений: только проблемы, найденные этой проверкой.
fn revision_text(
    run: &ReviewRunDto,
    version: &HomeworkVersionDto,
    requirements: &[TeacherRequirementDto],
) -> Result<(String, Vec<String>), AppError> {
    let result = run
        .result
        .as_ref()
        .ok_or_else(|| BackendError::database("completed review has no result"))?;
    let mut lines = vec![format!(
        "Исправь {} и создай новую версию. Предыдущую версию не изменяй.",
        version.label
    )];
    let mut addresses = Vec::new();
    let section = |lines: &mut Vec<String>, title: &str, items: Vec<String>| {
        if !items.is_empty() {
            lines.push(String::new());
            lines.push(title.to_owned());
            lines.extend(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| format!("{}. {text}", index + 1)),
            );
        }
    };
    let describe = |finding: &ReviewFindingDto| match &finding.location {
        Some(location) => format!("{} ({location})", finding.text),
        None => finding.text.clone(),
    };
    section(
        &mut lines,
        "Критические ошибки:",
        result.critical.iter().map(describe).collect(),
    );
    section(
        &mut lines,
        "Замечания:",
        result.remarks.iter().map(describe).collect(),
    );
    section(
        &mut lines,
        "Утверждения без опоры на материалы — удали или подтверди ссылкой:",
        result
            .unsupported_claims
            .iter()
            .map(|claim| format!("{} ({})", claim.text, claim.location))
            .collect(),
    );
    section(
        &mut lines,
        "Не хватает обязательных файлов или ссылок:",
        result
            .missing
            .iter()
            .filter(|item| item.required)
            .map(|item| item.label.clone())
            .collect(),
    );
    addresses.extend(result.critical.iter().map(|item| item.id.clone()));
    addresses.extend(result.remarks.iter().map(|item| item.id.clone()));
    addresses.extend(result.unsupported_claims.iter().map(|item| item.id.clone()));
    if let Some(style) = result
        .style
        .as_ref()
        .filter(|style| style.score > style.threshold)
    {
        let mut items = vec![format!(
            "Расхождение с личным стилем {}/10 при пороге {}",
            style.score, style.threshold
        )];
        items.extend(style.reasons.iter().map(|reason| reason.text.clone()));
        section(&mut lines, "Стиль:", items);
    }
    let not_done: Vec<_> = requirements
        .iter()
        .filter(|item| {
            item.status == RequirementStatus::NotDone
                && item.checked_version_id.as_deref() == Some(version.id.as_str())
        })
        .collect();
    section(
        &mut lines,
        "Требования преподавателя, которые не выполнены:",
        not_done.iter().map(|item| item.text.clone()).collect(),
    );
    addresses.extend(not_done.iter().map(|item| item.id.clone()));
    if lines.len() == 1 {
        return Err(BackendError::blocked(
            "Проверка не нашла проблем — промт исправлений не нужен",
        ));
    }
    lines.push(String::new());
    lines.push(
        "Не меняй то, что уже верно. Новые утверждения — только со ссылкой на материалы набора."
            .to_owned(),
    );
    Ok((lines.join("\n"), addresses))
}
