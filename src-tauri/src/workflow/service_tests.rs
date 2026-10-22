//! Тесты application service и state machine этапа 8B. In-memory SQLite и
//! подставная сверка содержимого; файловая система не используется.

use std::{cell::RefCell, collections::HashSet};

use serde_json::Value;
use uuid::Uuid;

use crate::{
    db::Database,
    domain::{HomeworkWorkflowStage as Stage, RequirementStatus, VersionAuthor},
    error::ErrorCode,
    events::names,
    workflow_commands::{deliver, finish, EventSink},
};

use super::{
    bundle::ContentCheck,
    dto::{
        CriterionId, CriterionStatus, MissingItemKind, PromptKind, PromptStatus, Provenance,
        ReadinessCheckId, ReviewMissingItemDto, ReviewRunStatus, ReviewVerdict, RunKind,
        TeacherCommentSource,
    },
    projections, repository,
    service::{
        CommandMeta, FindingInput, ManualAnalysisInput, ManualReviewInput, RunMode,
        TeacherCommentInput, TeacherRequirementInput, VersionInput, WorkflowEvent, WorkflowService,
    },
    state_machine::{self, check_transition, guard_for, TransitionContext, ALL_STAGES},
    tests::{seeded, FakeVerifier},
};

const HW: &str = "hw-1";

fn op() -> CommandMeta {
    CommandMeta {
        operation_id: Uuid::new_v4().to_string(),
        expected_revision: None,
    }
}

fn op_at(revision: i64) -> CommandMeta {
    CommandMeta {
        operation_id: Uuid::new_v4().to_string(),
        expected_revision: Some(revision),
    }
}

fn pass_review() -> ManualReviewInput {
    ManualReviewInput {
        verdict: ReviewVerdict::Pass,
        summary: "Решение верное".into(),
        correctness: CriterionStatus::Pass,
        task_fit: CriterionStatus::Pass,
        course_grounding: CriterionStatus::Pass,
        teacher: CriterionStatus::Na,
        style_score: Some(3),
        style_reasons: Vec::new(),
        critical: Vec::new(),
        remarks: Vec::new(),
        unsupported_claims: Vec::new(),
        missing: Vec::new(),
    }
}

fn failing_review() -> ManualReviewInput {
    ManualReviewInput {
        verdict: ReviewVerdict::NeedsFixes,
        summary: "Есть ошибка в расчёте".into(),
        correctness: CriterionStatus::Fail,
        critical: vec![FindingInput {
            criterion: CriterionId::Correctness,
            text: "Неверная формула выручки".into(),
            location: Some("лист «Расчёт», B4".into()),
            evidence: Vec::new(),
        }],
        ..pass_review()
    }
}

/// Сценарий: настоящий путь команд от материалов до нужного этапа.
struct Flow {
    database: Database,
    verifier: FakeVerifier,
    analysis_prompt: Option<String>,
    review_run: Option<String>,
    review_bundle: Option<String>,
    version: Option<String>,
}

impl Flow {
    fn new() -> Self {
        Self {
            database: seeded(),
            verifier: FakeVerifier::default(),
            analysis_prompt: None,
            review_run: None,
            review_bundle: None,
            version: None,
        }
    }

    fn service(&self) -> WorkflowService<'_> {
        WorkflowService::new(&self.database, &self.verifier)
    }

    fn stage(&self) -> Stage {
        self.database
            .with_connection(|connection| repository::homework_core(connection, HW))
            .unwrap()
            .workflow_stage
    }

    fn revision(&self) -> i64 {
        self.database
            .with_connection(|connection| repository::homework_core(connection, HW))
            .unwrap()
            .revision
    }

    fn snapshot(&self) -> repository::WorkflowSnapshot {
        self.database
            .with_connection(|connection| repository::load_snapshot(connection, HW))
            .unwrap()
    }

    fn analysis_bundle(&mut self) -> String {
        self.service()
            .create_source_bundle(&op(), HW, RunKind::AnalyzeMaterials, None)
            .unwrap()
            .value
            .id
    }

    /// → codex_analysis → prompt_ready (первичный промт зафиксирован).
    fn advance_to_prompt_ready(&mut self) -> &mut Self {
        let bundle = self.analysis_bundle();
        let run = self
            .service()
            .start_run(
                &op(),
                HW,
                RunKind::AnalyzeMaterials,
                None,
                &bundle,
                RunMode::Manual,
            )
            .unwrap()
            .value;
        assert_eq!(run.status, ReviewRunStatus::ManualHandoff);
        assert_eq!(self.stage(), Stage::CodexAnalysis);
        let saved = self
            .service()
            .save_manual_analysis(
                &op(),
                &run.id,
                &ManualAnalysisInput {
                    summary: "Условие и лекция разобраны".into(),
                    prompt_text: "Постройте BPMN-модель по условию".into(),
                },
            )
            .unwrap();
        let prompt = saved.value.result.unwrap().prompt.unwrap();
        assert_eq!(prompt.status, PromptStatus::Draft);
        self.service().commit_prompt(&op(), &prompt.id).unwrap();
        assert_eq!(self.stage(), Stage::PromptReady);
        self.analysis_prompt = Some(prompt.id);
        self
    }

    /// → claude_solving → codex_review с версией v01.
    fn advance_to_codex_review(&mut self) -> &mut Self {
        self.advance_to_prompt_ready();
        let prompt = self.analysis_prompt.clone().unwrap();
        self.service().hand_off_prompt(&op(), &prompt).unwrap();
        assert_eq!(self.stage(), Stage::ClaudeSolving);
        let version = self
            .service()
            .create_version(&op(), HW, &version_input("draft-1"))
            .unwrap()
            .value;
        assert_eq!(version.label, "v01");
        assert_eq!(version.prompt_id.as_deref(), Some(prompt.as_str()));
        assert_eq!(self.stage(), Stage::CodexReview);
        self.version = Some(version.id);
        self
    }

    fn review(&mut self, input: &ManualReviewInput) -> String {
        let version = self.snapshot().homework.final_version_id.unwrap();
        let bundle = self
            .service()
            .create_source_bundle(&op(), HW, RunKind::ReviewVersion, Some(&version))
            .unwrap()
            .value
            .id;
        let run = self
            .service()
            .start_run(
                &op(),
                HW,
                RunKind::ReviewVersion,
                Some(&version),
                &bundle,
                RunMode::Provider,
            )
            .unwrap()
            .value;
        assert_eq!(run.status, ReviewRunStatus::WaitingForProvider);
        assert!(run.result.is_none(), "без провайдера результата нет");
        self.service()
            .save_manual_review(&op(), &run.id, input)
            .unwrap();
        self.review_run = Some(run.id.clone());
        self.review_bundle = Some(bundle);
        run.id
    }

    fn advance_to_teacher_review(&mut self) -> &mut Self {
        self.advance_to_codex_review();
        self.review(&pass_review());
        assert_eq!(self.stage(), Stage::TeacherReview);
        self
    }

    fn advance_to_needs_fixes(&mut self) -> &mut Self {
        self.advance_to_codex_review();
        self.review(&failing_review());
        assert_eq!(self.stage(), Stage::NeedsFixes);
        self
    }

    fn add_done_requirement(&mut self) {
        let comments = self
            .service()
            .add_teacher_comments(
                &op(),
                HW,
                &[TeacherCommentInput {
                    source: TeacherCommentSource::Chat,
                    date: "2026-10-21".into(),
                    question: "Нужна ли BPMN?".into(),
                    answer: "Да".into(),
                }],
            )
            .unwrap()
            .value;
        self.service()
            .add_teacher_requirement(
                &op(),
                HW,
                &TeacherRequirementInput {
                    comment_id: comments[0].id.clone(),
                    text: "Использовать BPMN".into(),
                    origin: "Да".into(),
                    status: RequirementStatus::Done,
                    note: None,
                },
            )
            .unwrap();
    }

    fn advance_to_ready(&mut self) -> &mut Self {
        self.advance_to_teacher_review();
        self.add_done_requirement();
        self.service().mark_ready(&op(), HW, false).unwrap();
        assert_eq!(self.stage(), Stage::ReadyToSubmit);
        self
    }
}

fn version_input(file_id: &str) -> VersionInput {
    VersionInput {
        file_id: file_id.into(),
        attachment_file_ids: Vec::new(),
        author: Some(VersionAuthor::Claude),
        prompt_id: None,
        note: None,
    }
}

// ───────────────────────── Полный цикл и девять этапов ─────────────────────────

#[test]
fn full_manual_cycle_visits_every_reachable_stage_through_backend_only() {
    let mut flow = Flow::new();
    let mut seen = vec![flow.stage()];
    flow.advance_to_needs_fixes();
    let run = flow.review_run.clone().unwrap();
    let prompt = flow
        .service()
        .create_revision_prompt(&op(), &run, None)
        .unwrap()
        .value;
    assert_eq!(prompt.kind, PromptKind::Revision);
    assert!(prompt.text.contains("Неверная формула выручки"));
    assert_eq!(
        prompt.addresses.len(),
        1,
        "промт адресует только найденные проблемы"
    );
    flow.service().commit_prompt(&op(), &prompt.id).unwrap();
    flow.service().hand_off_prompt(&op(), &prompt.id).unwrap();
    assert_eq!(flow.stage(), Stage::ClaudeSolving);
    let v2 = flow
        .service()
        .create_version(&op(), HW, &version_input("draft-2"))
        .unwrap()
        .value;
    assert_eq!(v2.label, "v02");
    assert_eq!(v2.prompt_id.as_deref(), Some(prompt.id.as_str()));
    flow.review(&pass_review());
    assert_eq!(flow.stage(), Stage::TeacherReview);
    flow.add_done_requirement();
    flow.service().mark_ready(&op(), HW, false).unwrap();
    assert_eq!(flow.stage(), Stage::ReadyToSubmit);

    // Журнал: каждый переход — отдельная операция, revision растёт монотонно.
    let operations = crate::query_service::QueryService::new(&flow.database)
        .activity_log()
        .unwrap();
    assert!(operations
        .iter()
        .all(|item| item.status == crate::domain::OperationStatus::Completed));
    assert_eq!(
        flow.revision(),
        19,
        "каждая из 19 команд — одна новая ревизия"
    );
    for stage in [
        Stage::CodexAnalysis,
        Stage::PromptReady,
        Stage::ClaudeSolving,
        Stage::CodexReview,
        Stage::NeedsFixes,
        Stage::TeacherReview,
        Stage::ReadyToSubmit,
    ] {
        seen.push(stage);
    }
    assert_eq!(seen.len(), 8);

    // Девятый этап «Отправлено» определён, но выставляется только этапом 10.
    let error = flow
        .service()
        .transition(&op(), HW, Stage::Submitted, None)
        .unwrap_err()
        .error;
    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(flow.stage(), Stage::ReadyToSubmit);
    assert_eq!(ALL_STAGES.len(), 9);
}

#[test]
fn iteration_chain_links_versions_prompts_and_reviews_without_changing_old_iterations() {
    let mut flow = Flow::new();
    flow.advance_to_needs_fixes();
    let first_review = flow.review_run.clone().unwrap();
    let before = projections::iterations(&flow.snapshot());
    let prompt = flow
        .service()
        .create_revision_prompt(&op(), &first_review, None)
        .unwrap()
        .value;
    flow.service().commit_prompt(&op(), &prompt.id).unwrap();
    flow.service().hand_off_prompt(&op(), &prompt.id).unwrap();
    flow.service()
        .create_version(&op(), HW, &version_input("draft-2"))
        .unwrap();
    flow.review(&pass_review());
    let after = projections::iterations(&flow.snapshot());
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].version_id, before[0].version_id);
    assert_eq!(after[0].review.as_ref().unwrap().id, first_review);
    assert_eq!(after[0].revision_prompt.as_ref().unwrap().id, prompt.id);
    assert_eq!(
        after[0].next_version_id.as_deref(),
        Some(after[1].version_id.as_str())
    );
    assert_eq!(after[1].prompt.as_ref().unwrap().id, prompt.id);
    assert_eq!(
        after[1].fixed_findings.len(),
        1,
        "ошибка v01 исправлена в v02"
    );
    assert!(after[1].open_findings.is_empty());
    // Старая итерация не изменилась: та же проверка и тот же результат.
    let stored = |iteration: &super::dto::HomeworkIterationDto| {
        let mut result = iteration.review.as_ref().unwrap().result.clone().unwrap();
        result.prompt = None; // связь с промтом — производная, сам результат неизменен
        result
    };
    assert_eq!(stored(&after[0]), stored(&before[0]));
    let comparison = flow
        .service()
        .compare_versions(&after[0].version_id, &after[1].version_id)
        .unwrap();
    assert_eq!(comparison.fixed.len(), 1);
    assert!(comparison.introduced.is_empty());
}

// ───────────────────────── State machine ─────────────────────────

#[test]
fn every_forbidden_transition_is_rejected_without_evaluating_guards() {
    let flow = Flow::new();
    let snapshot = flow.snapshot();
    let readiness = projections::readiness(&snapshot);
    let context = TransitionContext {
        snapshot: &snapshot,
        readiness: &readiness,
        require_teacher_review: true,
        no_teacher_comments: true,
    };
    let mut forbidden = 0;
    for from in ALL_STAGES {
        for to in ALL_STAGES {
            if guard_for(from, to).is_some() {
                continue;
            }
            forbidden += 1;
            let error = check_transition(from, to, &context).unwrap_err();
            let expected = if from == Stage::Submitted {
                ErrorCode::Immutable
            } else {
                ErrorCode::Conflict
            };
            assert_eq!(error.code, expected, "{from:?} → {to:?}");
        }
    }
    assert_eq!(forbidden, 81 - 18);
}

#[test]
fn every_allowed_transition_passes_when_its_condition_holds() {
    let check = |flow: &Flow, from: Stage, to: Stage, require_teacher: bool| {
        let snapshot = flow.snapshot();
        let readiness = projections::readiness(&snapshot);
        check_transition(
            from,
            to,
            &TransitionContext {
                snapshot: &snapshot,
                readiness: &readiness,
                require_teacher_review: require_teacher,
                no_teacher_comments: true,
            },
        )
    };
    let mut covered = HashSet::new();
    let mut ok = |flow: &Flow, from: Stage, to: Stage, require_teacher: bool| {
        check(flow, from, to, require_teacher)
            .unwrap_or_else(|error| panic!("{from:?} → {to:?}: {error:?}"));
        covered.insert((from, to));
    };

    let mut flow = Flow::new();
    flow.analysis_bundle();
    ok(&flow, Stage::MaterialsCollected, Stage::CodexAnalysis, true);
    ok(&flow, Stage::CodexAnalysis, Stage::MaterialsCollected, true);
    ok(&flow, Stage::PromptReady, Stage::CodexAnalysis, true);
    ok(&flow, Stage::ClaudeSolving, Stage::PromptReady, true);
    ok(&flow, Stage::CodexReview, Stage::ClaudeSolving, true);
    ok(&flow, Stage::TeacherReview, Stage::CodexReview, true);
    ok(&flow, Stage::TeacherReview, Stage::NeedsFixes, true);
    ok(&flow, Stage::ReadyToSubmit, Stage::NeedsFixes, true);
    ok(&flow, Stage::ReadyToSubmit, Stage::TeacherReview, true);

    let mut flow = Flow::new();
    flow.advance_to_prompt_ready();
    ok(&flow, Stage::CodexAnalysis, Stage::PromptReady, true);

    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    ok(&flow, Stage::PromptReady, Stage::ClaudeSolving, true);
    ok(&flow, Stage::ClaudeSolving, Stage::CodexReview, true);

    let mut flow = Flow::new();
    flow.advance_to_needs_fixes();
    ok(&flow, Stage::CodexReview, Stage::NeedsFixes, true);
    let run = flow.review_run.clone().unwrap();
    let prompt = flow
        .service()
        .create_revision_prompt(&op(), &run, None)
        .unwrap()
        .value;
    flow.service().commit_prompt(&op(), &prompt.id).unwrap();
    flow.service().hand_off_prompt(&op(), &prompt.id).unwrap();
    ok(&flow, Stage::NeedsFixes, Stage::ClaudeSolving, true);

    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    ok(&flow, Stage::CodexReview, Stage::TeacherReview, true);
    ok(&flow, Stage::CodexReview, Stage::ReadyToSubmit, false);
    ok(&flow, Stage::TeacherReview, Stage::ReadyToSubmit, true);

    // Отправка — определённый переход, но до этапа 10 всегда заблокирован.
    let error = check(&flow, Stage::ReadyToSubmit, Stage::Submitted, true).unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    covered.insert((Stage::ReadyToSubmit, Stage::Submitted));

    let all_allowed: HashSet<_> = ALL_STAGES
        .iter()
        .flat_map(|from| ALL_STAGES.iter().map(move |to| (*from, *to)))
        .filter(|(from, to)| guard_for(*from, *to).is_some())
        .collect();
    assert_eq!(covered, all_allowed, "каждый допустимый переход проверен");
}

#[test]
fn guarded_transitions_fail_until_their_condition_holds() {
    let flow = Flow::new();
    let snapshot = flow.snapshot();
    let readiness = projections::readiness(&snapshot);
    let context = TransitionContext {
        snapshot: &snapshot,
        readiness: &readiness,
        require_teacher_review: true,
        no_teacher_comments: false,
    };
    for (from, to) in [
        (Stage::MaterialsCollected, Stage::CodexAnalysis),
        (Stage::CodexAnalysis, Stage::PromptReady),
        (Stage::PromptReady, Stage::ClaudeSolving),
        (Stage::ClaudeSolving, Stage::CodexReview),
        (Stage::CodexReview, Stage::NeedsFixes),
        (Stage::CodexReview, Stage::TeacherReview),
        (Stage::CodexReview, Stage::ReadyToSubmit),
        (Stage::NeedsFixes, Stage::ClaudeSolving),
        (Stage::TeacherReview, Stage::ReadyToSubmit),
    ] {
        assert_eq!(
            check_transition(from, to, &context).unwrap_err().code,
            ErrorCode::Blocked,
            "{from:?} → {to:?}"
        );
    }
}

#[test]
fn stages_cannot_be_skipped_or_written_directly() {
    let mut flow = Flow::new();
    flow.analysis_bundle();
    for to in [
        Stage::CodexReview,
        Stage::PromptReady,
        Stage::ClaudeSolving,
        Stage::TeacherReview,
    ] {
        let failure = flow.service().transition(&op(), HW, to, None).unwrap_err();
        assert_eq!(failure.error.code, ErrorCode::Conflict, "{to:?}");
        assert_eq!(
            failure.operation.unwrap().status,
            crate::domain::OperationStatus::Failed
        );
    }
    assert_eq!(
        flow.service()
            .mark_ready(&op(), HW, true)
            .unwrap_err()
            .error
            .code,
        ErrorCode::Conflict
    );
    // Версию нельзя добавить, пока промт не передан автору.
    assert_eq!(
        flow.service()
            .create_version(&op(), HW, &version_input("draft-1"))
            .unwrap_err()
            .error
            .code,
        ErrorCode::Blocked
    );
    assert_eq!(flow.stage(), Stage::MaterialsCollected);
    // Explicit transition через state machine разрешён только по таблице.
    flow.service()
        .transition(&op(), HW, Stage::CodexAnalysis, Some("Начинаю анализ"))
        .unwrap();
    flow.service()
        .transition(&op(), HW, Stage::MaterialsCollected, None)
        .unwrap();
    assert_eq!(flow.stage(), Stage::MaterialsCollected);
    assert_eq!(
        flow.service()
            .transition(&op(), HW, Stage::ReadyToSubmit, None)
            .unwrap_err()
            .error
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn aggregate_lists_transitions_computed_by_backend() {
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    let aggregate = flow.service().aggregate(HW).unwrap();
    assert_eq!(aggregate.stage, Stage::TeacherReview);
    let targets: Vec<_> = aggregate
        .transitions
        .iter()
        .map(|option| (option.to, option.allowed))
        .collect();
    assert_eq!(
        targets,
        [
            (Stage::CodexReview, true),
            (Stage::NeedsFixes, true),
            (Stage::ReadyToSubmit, true)
        ]
    );
    assert_eq!(
        state_machine::stage_label(Stage::TeacherReview),
        "Проверка преподавателем"
    );
    let value = serde_json::to_value(&aggregate).unwrap();
    let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "bundles",
            "homework",
            "iterations",
            "prompts",
            "providerMode",
            "readiness",
            "revision",
            "runs",
            "stage",
            "styleConfirmations",
            "submissionPackage",
            "submissionReceipt",
            "teacherComments",
            "teacherRequirements",
            "transitions"
        ]
    );
    assert_eq!(value["providerMode"], "manual_handoff");
}

// ───────────────────────── Идемпотентность и конфликты ─────────────────────────

#[test]
fn replaying_the_same_operation_id_is_idempotent() {
    let flow = Flow::new();
    let meta = op();
    let first = flow
        .service()
        .create_source_bundle(&meta, HW, RunKind::AnalyzeMaterials, None)
        .unwrap();
    assert!(!first.replayed);
    assert!(!first.events.is_empty());
    let second = flow
        .service()
        .create_source_bundle(&meta, HW, RunKind::AnalyzeMaterials, None)
        .unwrap();
    assert!(second.replayed);
    assert!(second.events.is_empty(), "повтор не эмитит событий");
    assert_eq!(second.value, first.value);
    assert_eq!(second.operation.id, first.operation.id);
    let bundles: i64 =
        flow.database
            .with_connection(|connection| {
                Ok(connection
                    .query_row("SELECT count(*) FROM source_bundles", [], |row| row.get(0))?)
            })
            .unwrap();
    assert_eq!(bundles, 1);
    assert_eq!(flow.revision(), 1, "повтор не меняет ревизию");

    // Тот же id для другой команды — конфликт без изменений.
    let other = flow
        .service()
        .add_teacher_comments(
            &meta,
            HW,
            &[TeacherCommentInput {
                source: TeacherCommentSource::Email,
                date: "2026-10-21".into(),
                question: "q".into(),
                answer: "a".into(),
            }],
        )
        .unwrap_err();
    assert_eq!(other.error.code, ErrorCode::Conflict);
    assert!(flow.snapshot().teacher_comments.is_empty());

    // Неверный идентификатор операции отклоняется до записи.
    let invalid = CommandMeta {
        operation_id: "not-a-uuid".into(),
        expected_revision: None,
    };
    assert_eq!(
        flow.service()
            .transition(&invalid, HW, Stage::CodexAnalysis, None)
            .unwrap_err()
            .error
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn failed_operation_id_cannot_be_reused() {
    let flow = Flow::new();
    let meta = op();
    let failure = flow
        .service()
        .transition(&meta, HW, Stage::CodexReview, None)
        .unwrap_err();
    assert_eq!(
        failure.operation.as_ref().unwrap().status,
        crate::domain::OperationStatus::Failed
    );
    assert_eq!(failure.events.len(), 1);
    assert_eq!(failure.events[0].name, names::OPERATION_FAILED);
    let again = flow
        .service()
        .transition(&meta, HW, Stage::CodexReview, None)
        .unwrap_err();
    assert_eq!(again.error.code, ErrorCode::Conflict);
    assert!(again.events.is_empty());
}

#[test]
fn stale_revision_is_a_conflict_and_changes_nothing() {
    let flow = Flow::new();
    let revision = flow.revision();
    // Два клиента прочитали одну ревизию; второй опоздал.
    flow.service()
        .add_teacher_comments(
            &op_at(revision),
            HW,
            &[TeacherCommentInput {
                source: TeacherCommentSource::Chat,
                date: "2026-10-21".into(),
                question: "Первый".into(),
                answer: "".into(),
            }],
        )
        .unwrap();
    let late = flow
        .service()
        .add_teacher_comments(
            &op_at(revision),
            HW,
            &[TeacherCommentInput {
                source: TeacherCommentSource::Chat,
                date: "2026-10-21".into(),
                question: "Второй".into(),
                answer: "".into(),
            }],
        )
        .unwrap_err();
    assert_eq!(late.error.code, ErrorCode::Conflict);
    assert!(late.error.message.contains("перечитайте"));
    assert_eq!(flow.snapshot().teacher_comments.len(), 1);
    assert_eq!(flow.revision(), revision + 1);
    assert_eq!(
        late.operation.unwrap().status,
        crate::domain::OperationStatus::Failed
    );
}

#[test]
fn failure_rolls_back_every_write_of_the_command() {
    let flow = Flow::new();
    let failure = flow
        .service()
        .add_teacher_comments(
            &op(),
            HW,
            &[
                TeacherCommentInput {
                    source: TeacherCommentSource::Chat,
                    date: "2026-10-21".into(),
                    question: "Корректный".into(),
                    answer: "Ответ".into(),
                },
                TeacherCommentInput {
                    source: TeacherCommentSource::Chat,
                    date: "2026-10-21".into(),
                    question: "".into(),
                    answer: " ".into(),
                },
            ],
        )
        .unwrap_err();
    assert_eq!(failure.error.code, ErrorCode::Blocked);
    assert!(
        flow.snapshot().teacher_comments.is_empty(),
        "первый комментарий откатан"
    );
    assert_eq!(flow.revision(), 0);
    let log = crate::query_service::QueryService::new(&flow.database)
        .activity_log()
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].status, crate::domain::OperationStatus::Failed);
    assert_eq!(log[0].homework_id.as_deref(), Some(HW));
}

// ───────────────────────── События ─────────────────────────

struct RecordingSink<'a> {
    database: &'a Database,
    fail: bool,
    seen: RefCell<Vec<(String, i64)>>,
}

impl EventSink for RecordingSink<'_> {
    fn emit_event(&self, event: &WorkflowEvent) -> Result<(), ()> {
        // Событие видит уже зафиксированное состояние: ревизия в базе — новая.
        let revision = self
            .database
            .with_connection(|connection| repository::homework_core(connection, HW))
            .map(|core| core.revision)
            .unwrap_or(-1);
        self.seen
            .borrow_mut()
            .push((event.name.to_owned(), revision));
        if self.fail {
            Err(())
        } else {
            Ok(())
        }
    }
}

#[test]
fn events_are_formed_only_after_commit_from_committed_state() {
    let flow = Flow::new();
    let result = flow
        .service()
        .create_source_bundle(&op(), HW, RunKind::AnalyzeMaterials, None);
    let sink = RecordingSink {
        database: &flow.database,
        fail: false,
        seen: RefCell::new(Vec::new()),
    };
    finish(&sink, result).unwrap();
    let seen = sink.seen.borrow();
    let names: Vec<_> = seen.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        [
            names::HOMEWORK_UPDATED,
            names::HOMEWORK_READINESS_UPDATED,
            names::OPERATION_PROGRESS,
            names::OPERATION_COMPLETED
        ]
    );
    assert!(seen.iter().all(|(_, revision)| *revision == 1));

    // Ошибка: доменных событий нет, только operation:failed.
    let failure = flow
        .service()
        .transition(&op(), HW, Stage::CodexReview, None);
    let sink = RecordingSink {
        database: &flow.database,
        fail: false,
        seen: RefCell::new(Vec::new()),
    };
    assert!(finish(&sink, failure).is_err());
    let failed: Vec<_> = sink
        .seen
        .borrow()
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    assert_eq!(failed, [names::OPERATION_FAILED]);
}

#[test]
fn event_delivery_failure_keeps_committed_data_and_asks_to_reload() {
    let flow = Flow::new();
    let result = flow
        .service()
        .create_source_bundle(&op(), HW, RunKind::AnalyzeMaterials, None);
    let sink = RecordingSink {
        database: &flow.database,
        fail: true,
        seen: RefCell::new(Vec::new()),
    };
    let error = finish(&sink, result).unwrap_err();
    assert_eq!(error.code, ErrorCode::Unknown);
    assert!(error.message.contains("перечитайте"));
    assert_eq!(flow.snapshot().bundles.len(), 1, "данные не откатываются");
    assert_eq!(flow.revision(), 1);
    assert!(deliver(&sink, &[]).is_ok());
}

#[test]
fn stage_change_version_review_and_prompt_events_carry_ids_without_paths() {
    let mut flow = Flow::new();
    flow.advance_to_prompt_ready();
    let prompt = flow.analysis_prompt.clone().unwrap();
    flow.service().hand_off_prompt(&op(), &prompt).unwrap();
    let created = flow
        .service()
        .create_version(&op(), HW, &version_input("draft-1"))
        .unwrap();
    let names_seen: Vec<_> = created.events.iter().map(|event| event.name).collect();
    assert_eq!(
        names_seen,
        [
            names::HOMEWORK_VERSION_CREATED,
            names::WORKFLOW_STAGE_CHANGED,
            names::HOMEWORK_UPDATED,
            names::HOMEWORK_READINESS_UPDATED,
            names::OPERATION_PROGRESS,
            names::OPERATION_COMPLETED
        ]
    );
    let stage = &created.events[1].payload;
    assert_eq!(stage["from"], "claude_solving");
    assert_eq!(stage["to"], "codex_review");
    assert_eq!(created.events[2].payload["homeworkId"], HW);
    assert!(created.events[2].payload["revision"].is_i64());
    let review = flow.review(&failing_review());
    let snapshot = flow.snapshot();
    assert!(snapshot.runs.iter().any(|run| run.id == review));
    let saved = flow
        .service()
        .create_revision_prompt(&op(), &review, None)
        .unwrap();
    let prompt_event = saved
        .events
        .iter()
        .find(|event| event.name == names::HOMEWORK_REVISION_PROMPT_CREATED)
        .unwrap();
    assert_eq!(prompt_event.payload["reviewRunId"], review.as_str());
    assert_eq!(prompt_event.payload["status"], "draft");
    for event in created.events.iter().chain(saved.events.iter()) {
        let text = event.payload.to_string();
        assert!(
            !text.contains("/Users/") && !text.contains("file://") && !text.contains("~/"),
            "{text}"
        );
        assert!(
            !text.contains("Постройте модель процесса"),
            "без текста извлечения"
        );
        assert_no_snake_case_keys(&event.payload);
    }
}

fn assert_no_snake_case_keys(value: &Value) {
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                assert!(!key.contains('_'), "snake_case key {key}");
                assert_no_snake_case_keys(nested);
            }
        }
        Value::Array(items) => items.iter().for_each(assert_no_snake_case_keys),
        _ => {}
    }
}

// ───────────────────────── Проверка, порог, промты ─────────────────────────

#[test]
fn review_is_bound_to_version_and_bundle_and_keeps_applied_threshold() {
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    let run = flow.review_run.clone().unwrap();
    let bundle = flow.review_bundle.clone().unwrap();
    let version = flow.version.clone().unwrap();
    let stored = flow
        .database
        .with_connection(|connection| repository::run(connection, &run))
        .unwrap();
    assert_eq!(stored.version_id.as_deref(), Some(version.as_str()));
    assert_eq!(stored.source_bundle_id.as_deref(), Some(bundle.as_str()));
    assert_eq!(stored.provenance, Provenance::Manual);
    assert_eq!(stored.style_threshold, 4);
    assert_eq!(
        stored
            .result
            .as_ref()
            .unwrap()
            .style
            .as_ref()
            .unwrap()
            .threshold,
        4
    );

    let mut rules = crate::dto::AutomationRulesDto::default();
    rules.workflow.style_threshold = 2;
    crate::mutation_service::MutationService::new(&flow.database)
        .update_automation_rules(rules)
        .unwrap();
    let old = flow
        .database
        .with_connection(|connection| repository::run(connection, &run))
        .unwrap();
    assert_eq!(
        old.style_threshold, 4,
        "изменение настройки не переписывает старую проверку"
    );
    // Новая проверка получает новый порог.
    flow.service()
        .transition(&op(), HW, Stage::CodexReview, None)
        .unwrap();
    let newer = flow.review(&pass_review());
    let newer = flow
        .database
        .with_connection(|connection| repository::run(connection, &newer))
        .unwrap();
    assert_eq!(newer.style_threshold, 2);
}

#[test]
fn review_runs_require_complete_unchanged_bundle_and_current_version() {
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    let version = flow.version.clone().unwrap();
    let analysis_bundle = flow.snapshot().bundles[0].id.clone();
    let wrong_purpose = flow
        .service()
        .start_run(
            &op(),
            HW,
            RunKind::ReviewVersion,
            Some(&version),
            &analysis_bundle,
            RunMode::Manual,
        )
        .unwrap_err();
    assert_eq!(wrong_purpose.error.code, ErrorCode::Conflict);
    let bundle = flow
        .service()
        .create_source_bundle(&op(), HW, RunKind::ReviewVersion, Some(&version))
        .unwrap()
        .value
        .id;
    // Материал изменился после снимка → запуск заблокирован.
    flow.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE material_extractions SET sha256 = ?1 WHERE material_id = 'task-file'",
                ["7".repeat(64)],
            )?;
            Ok(())
        })
        .unwrap();
    let changed = flow
        .service()
        .start_run(
            &op(),
            HW,
            RunKind::ReviewVersion,
            Some(&version),
            &bundle,
            RunMode::Manual,
        )
        .unwrap_err();
    assert_eq!(changed.error.code, ErrorCode::Blocked);
    // Повреждённый источник блокирует и новый набор.
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    flow.verifier
        .checks
        .insert("lecture-file".into(), ContentCheck::Changed);
    let version = flow.version.clone().unwrap();
    let blocked = flow
        .service()
        .create_source_bundle(&op(), HW, RunKind::ReviewVersion, Some(&version))
        .unwrap_err();
    assert_eq!(blocked.error.code, ErrorCode::Blocked);
    assert_eq!(
        blocked.operation.unwrap().status,
        crate::domain::OperationStatus::Failed
    );
}

#[test]
fn one_open_run_at_a_time_and_cancelled_runs_are_final() {
    let mut flow = Flow::new();
    let bundle = flow.analysis_bundle();
    let run = flow
        .service()
        .start_run(
            &op(),
            HW,
            RunKind::AnalyzeMaterials,
            None,
            &bundle,
            RunMode::Provider,
        )
        .unwrap()
        .value;
    let parallel = flow
        .service()
        .start_run(
            &op(),
            HW,
            RunKind::AnalyzeMaterials,
            None,
            &bundle,
            RunMode::Manual,
        )
        .unwrap_err();
    assert_eq!(parallel.error.code, ErrorCode::Conflict);
    let cancelled = flow.service().cancel_run(&op(), &run.id).unwrap().value;
    assert_eq!(cancelled.status, ReviewRunStatus::Cancelled);
    let late = flow
        .service()
        .save_manual_analysis(
            &op(),
            &run.id,
            &ManualAnalysisInput {
                summary: "x".into(),
                prompt_text: "y".into(),
            },
        )
        .unwrap_err();
    assert_eq!(late.error.code, ErrorCode::Conflict);
    assert!(flow.snapshot().prompts.is_empty());
}

#[test]
fn manual_review_cannot_claim_pass_with_critical_errors() {
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    let invalid = ManualReviewInput {
        verdict: ReviewVerdict::Pass,
        ..failing_review()
    };
    let version = flow.version.clone().unwrap();
    let bundle = flow
        .service()
        .create_source_bundle(&op(), HW, RunKind::ReviewVersion, Some(&version))
        .unwrap()
        .value
        .id;
    let run = flow
        .service()
        .start_run(
            &op(),
            HW,
            RunKind::ReviewVersion,
            Some(&version),
            &bundle,
            RunMode::Manual,
        )
        .unwrap()
        .value;
    let error = flow
        .service()
        .save_manual_review(&op(), &run.id, &invalid)
        .unwrap_err()
        .error;
    assert_eq!(error.code, ErrorCode::Blocked);
    let still_open = flow
        .database
        .with_connection(|connection| repository::run(connection, &run.id))
        .unwrap();
    assert_eq!(still_open.status, ReviewRunStatus::ManualHandoff);
}

#[test]
fn committed_prompts_are_immutable_and_new_prompts_are_new_rows() {
    let mut flow = Flow::new();
    flow.advance_to_prompt_ready();
    let prompt = flow.analysis_prompt.clone().unwrap();
    let edit = flow
        .service()
        .update_prompt_draft(&op(), &prompt, "Подмена")
        .unwrap_err();
    assert_eq!(edit.error.code, ErrorCode::Immutable);
    let recommit = flow.service().commit_prompt(&op(), &prompt).unwrap_err();
    assert_eq!(recommit.error.code, ErrorCode::Immutable);
    flow.service().hand_off_prompt(&op(), &prompt).unwrap();
    let again = flow.service().hand_off_prompt(&op(), &prompt).unwrap_err();
    assert_eq!(again.error.code, ErrorCode::Blocked);
    let stored = flow
        .database
        .with_connection(|connection| repository::prompt(connection, &prompt))
        .unwrap();
    assert_eq!(stored.text, "Постройте BPMN-модель по условию");

    // Новый промт исправлений — новая строка, старый промт не меняется.
    flow.service()
        .create_version(&op(), HW, &version_input("draft-1"))
        .unwrap();
    flow.review(&failing_review());
    let run = flow.review_run.clone().unwrap();
    let draft = flow
        .service()
        .create_revision_prompt(&op(), &run, Some("Исправь формулу"))
        .unwrap()
        .value;
    assert_ne!(draft.id, prompt);
    let edited = flow
        .service()
        .update_prompt_draft(&op(), &draft.id, "Исправь формулу выручки")
        .unwrap()
        .value;
    assert_eq!(edited.text, "Исправь формулу выручки");
    let unchanged = flow
        .database
        .with_connection(|connection| repository::prompt(connection, &prompt))
        .unwrap();
    assert_eq!(
        unchanged, stored,
        "старый промт не меняется новой итерацией"
    );
}

// ───────────────────────── Readiness ─────────────────────────

fn blockers(flow: &Flow) -> Vec<ReadinessCheckId> {
    projections::readiness(&flow.snapshot()).blockers()
}

#[test]
fn readiness_is_ready_after_full_cycle() {
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    flow.add_done_requirement();
    assert_eq!(blockers(&flow), Vec::<ReadinessCheckId>::new());
}

#[test]
fn readiness_is_blocked_by_each_required_condition_separately() {
    // Критические ошибки (и итог «нужны исправления»).
    let mut flow = Flow::new();
    flow.advance_to_needs_fixes();
    assert_eq!(
        blockers(&flow),
        [ReadinessCheckId::ReviewPassed, ReadinessCheckId::NoCritical]
    );

    // Обязательные условия задания не покрыты.
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    flow.review(&ManualReviewInput {
        task_fit: CriterionStatus::Warn,
        ..pass_review()
    });
    assert_eq!(blockers(&flow), [ReadinessCheckId::TaskCoverage]);

    // Отсутствует обязательный файл.
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    flow.review(&ManualReviewInput {
        verdict: ReviewVerdict::NeedsFixes,
        missing: vec![ReviewMissingItemDto {
            kind: MissingItemKind::File,
            label: "Пояснительная записка".into(),
            required: true,
        }],
        ..pass_review()
    });
    assert_eq!(
        blockers(&flow),
        [
            ReadinessCheckId::ReviewPassed,
            ReadinessCheckId::Attachments
        ]
    );

    // Расхождение со стилем выше порога без подтверждения → блок; с подтверждением — нет.
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    let run = flow.review(&ManualReviewInput {
        style_score: Some(7),
        ..pass_review()
    });
    assert_eq!(blockers(&flow), [ReadinessCheckId::Style]);
    let confirmation = flow.service().confirm_style(&op(), &run).unwrap().value;
    assert_eq!(confirmation.review_run_id, run);
    assert_eq!(confirmation.version_id, flow.version.clone().unwrap());
    assert_eq!((confirmation.score, confirmation.threshold), (7, 4));
    assert_eq!(confirmation.confirmed_by, "user");
    assert_eq!(blockers(&flow), Vec::<ReadinessCheckId>::new());

    // Требование преподавателя не выполнено.
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    flow.add_done_requirement();
    let requirement = flow.snapshot().teacher_requirements[0].id.clone();
    flow.service()
        .resolve_requirement(
            &op(),
            &requirement,
            RequirementStatus::NotDone,
            "Нет диаграммы",
        )
        .unwrap();
    assert_eq!(blockers(&flow), [ReadinessCheckId::Teacher]);

    // Исходный материал изменился после проверки.
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    flow.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE inbox_files SET sha256 = ?1 WHERE id = 'inbox-1'",
                ["8".repeat(64)],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(blockers(&flow), [ReadinessCheckId::SourcesFresh]);

    // Проверка относится не к текущей версии (появилась и выбрана новая).
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    flow.service()
        .transition(&op(), HW, Stage::CodexReview, None)
        .unwrap();
    let v2 = flow
        .service()
        .create_version(&op(), HW, &version_input("draft-2"))
        .unwrap()
        .value;
    assert_eq!(
        flow.snapshot().homework.final_version_id.as_deref(),
        Some(v2.id.as_str())
    );
    let current = blockers(&flow);
    assert!(current.contains(&ReadinessCheckId::ReviewCurrent));
    let v1 = flow.version.clone().unwrap();
    flow.service().select_version(&op(), &v1).unwrap();
    assert_eq!(
        blockers(&flow),
        Vec::<ReadinessCheckId>::new(),
        "возврат к проверенной версии"
    );

    // Нет материалов и версии.
    let flow = Flow::new();
    let empty = blockers(&flow);
    assert!(empty.contains(&ReadinessCheckId::Materials));
    assert!(empty.contains(&ReadinessCheckId::FinalVersion));
}

#[test]
fn style_confirmation_is_bound_to_one_review_of_one_version() {
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    let first = flow.review(&ManualReviewInput {
        style_score: Some(8),
        ..pass_review()
    });
    flow.service().confirm_style(&op(), &first).unwrap();
    assert_eq!(
        flow.service()
            .confirm_style(&op(), &first)
            .unwrap_err()
            .error
            .code,
        ErrorCode::Conflict
    );
    // Новая версия — новая проверка — нужно новое подтверждение.
    flow.service()
        .transition(&op(), HW, Stage::CodexReview, None)
        .unwrap();
    flow.service()
        .create_version(&op(), HW, &version_input("draft-2"))
        .unwrap();
    flow.review(&ManualReviewInput {
        style_score: Some(8),
        ..pass_review()
    });
    assert!(blockers(&flow).contains(&ReadinessCheckId::Style));
    let low = {
        let mut flow = Flow::new();
        flow.advance_to_codex_review();
        let run = flow.review(&pass_review());
        flow.service()
            .confirm_style(&op(), &run)
            .unwrap_err()
            .error
            .code
    };
    assert_eq!(low, ErrorCode::Blocked, "ниже порога подтверждать нечего");
}

#[test]
fn mark_ready_requires_confirmation_when_teacher_comments_are_absent() {
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    let error = flow
        .service()
        .mark_ready(&op(), HW, false)
        .unwrap_err()
        .error;
    assert_eq!(error.code, ErrorCode::ConfirmationRequired);
    flow.service().mark_ready(&op(), HW, true).unwrap();
    assert_eq!(flow.stage(), Stage::ReadyToSubmit);
    let homework = flow.snapshot().homework;
    assert_eq!(homework.stage, crate::domain::HomeworkStage::Ready);
    // Из «Готово к сдаче» нельзя создавать наборы и версии — только вернуть на исправления.
    assert_eq!(
        flow.service()
            .create_source_bundle(
                &op(),
                HW,
                RunKind::ReviewVersion,
                homework.final_version_id.as_deref()
            )
            .unwrap_err()
            .error
            .code,
        ErrorCode::Blocked
    );
    flow.service()
        .transition(&op(), HW, Stage::NeedsFixes, Some("Нашла опечатку"))
        .unwrap();
    assert_eq!(flow.stage(), Stage::NeedsFixes);
}

// ───────────────────────── Отправленная работа ─────────────────────────

#[test]
fn submitted_homework_rejects_every_command() {
    let mut flow = Flow::new();
    flow.advance_to_ready();
    flow.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE homeworks SET stage = 'submitted', workflow_stage = 'submitted',
                        submitted_at = '2026-10-22T10:00:00Z' WHERE id = 'hw-1'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let version = flow.version.clone().unwrap();
    let run = flow.review_run.clone().unwrap();
    let codes = [
        flow.service()
            .create_source_bundle(&op(), HW, RunKind::ReviewVersion, Some(&version))
            .unwrap_err()
            .error
            .code,
        flow.service()
            .transition(&op(), HW, Stage::NeedsFixes, None)
            .unwrap_err()
            .error
            .code,
        flow.service()
            .select_version(&op(), &version)
            .unwrap_err()
            .error
            .code,
        flow.service().pin_run(&op(), &run).unwrap_err().error.code,
        flow.service()
            .add_teacher_comments(
                &op(),
                HW,
                &[TeacherCommentInput {
                    source: TeacherCommentSource::Other,
                    date: "2026-10-23".into(),
                    question: "Поздно".into(),
                    answer: String::new(),
                }],
            )
            .unwrap_err()
            .error
            .code,
    ];
    assert!(
        codes.iter().all(|code| *code == ErrorCode::Immutable),
        "{codes:?}"
    );
    // Агрегат отправленной работы по-прежнему читается.
    let aggregate = flow.service().aggregate(HW).unwrap();
    assert_eq!(aggregate.stage, Stage::Submitted);
    assert!(aggregate.transitions.is_empty());
}

#[test]
fn concurrent_commands_on_one_revision_produce_one_winner() {
    let flow = std::sync::Arc::new(Flow::new());
    let revision = flow.revision();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|index| {
            let flow = std::sync::Arc::clone(&flow);
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                flow.service()
                    .add_teacher_comments(
                        &op_at(revision),
                        HW,
                        &[TeacherCommentInput {
                            source: TeacherCommentSource::Chat,
                            date: "2026-10-21".into(),
                            question: format!("Вопрос {index}"),
                            answer: String::new(),
                        }],
                    )
                    .map(|_| ())
                    .map_err(|failure| failure.error.code)
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(results
        .iter()
        .filter_map(|result| result.err())
        .all(|code| code == ErrorCode::Conflict));
    assert_eq!(flow.snapshot().teacher_comments.len(), 1);
}
