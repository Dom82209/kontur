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
        self.style_ok();
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
        self.style_ok();
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
                    authority: super::dto::CommentAuthority::TeacherOfficial,
                    author: String::new(),
                    context: String::new(),
                    confirmed_official: true,
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
        // Этап 7: новое официальное требование делает прежнюю проверку
        // устаревшей — версию проверяют заново по актуальным требованиям.
        if self.stage() == Stage::TeacherReview {
            self.rereview();
        }
    }

    fn rereview(&mut self) {
        self.service()
            .transition(&op(), HW, Stage::CodexReview, None)
            .unwrap();
        self.review(&pass_review());
        assert_eq!(self.stage(), Stage::TeacherReview);
        self.style_ok();
    }

    /// Этап 9: стиль проверен (без профиля — ручное подтверждение).
    fn style_ok(&self) {
        let style = crate::style::StyleService::new(&self.database, None);
        let check = style.check(HW).unwrap();
        if !check.passes {
            style.confirm(&check.id, "проверено вручную").unwrap();
        }
    }

    fn mark_ready_ok(&self, no_teacher_comments: bool) {
        self.style_ok();
        self.service()
            .mark_ready(&op(), HW, no_teacher_comments)
            .unwrap();
    }

    fn advance_to_ready(&mut self) -> &mut Self {
        self.advance_to_teacher_review();
        self.add_done_requirement();
        self.mark_ready_ok(false);
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
        parent_version_id: None,
        origin: None,
        creation_reason: None,
        review_run_id: None,
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
    flow.mark_ready_ok(false);
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
        23,
        "каждая из 23 команд (19 + повторная проверка после нового требования) — одна новая ревизия"
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
            "aiJobs",
            "bundles",
            "homework",
            "inputs",
            "iterations",
            "packageHistory",
            "prompts",
            "providerMode",
            "readiness",
            "readyStale",
            "revision",
            "runs",
            "stage",
            "styleCheck",
            "styleConfirmations",
            "submissionPackage",
            "submissionReceipt",
            "teacherComments",
            "teacherRequirements",
            "transitions"
        ]
    );
    assert_eq!(value["providerMode"], "ai_jobs");
    assert_eq!(value["aiJobs"], serde_json::json!([]));
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
                authority: super::dto::CommentAuthority::TeacherOfficial,
                author: String::new(),
                context: String::new(),
                confirmed_official: true,
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
                authority: super::dto::CommentAuthority::TeacherOfficial,
                author: String::new(),
                context: String::new(),
                confirmed_official: true,
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
                authority: super::dto::CommentAuthority::TeacherOfficial,
                author: String::new(),
                context: String::new(),
                confirmed_official: true,
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
                    authority: super::dto::CommentAuthority::TeacherOfficial,
                    author: String::new(),
                    context: String::new(),
                    confirmed_official: true,
                },
                TeacherCommentInput {
                    source: TeacherCommentSource::Chat,
                    date: "2026-10-21".into(),
                    question: "".into(),
                    answer: " ".into(),
                    authority: super::dto::CommentAuthority::TeacherOfficial,
                    author: String::new(),
                    context: String::new(),
                    confirmed_official: true,
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
    // Запись не меняется; производный флаг `stale` отражает новые входные данные.
    assert!(unchanged.stale, "промт по прежней версии устарел");
    let unchanged = super::dto::RevisionPromptDto {
        stale: stored.stale,
        ..unchanged
    };
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

    // Этап 9: стиль — по локальной проверке. Без профиля проверка требует
    // ручного подтверждения; до него — блок; оценка AI-проверки не заменяет её.
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    flow.review(&ManualReviewInput {
        style_score: Some(2),
        ..pass_review()
    });
    let style = crate::style::StyleService::new(&flow.database, None);
    let check = style.check(HW).unwrap();
    assert_eq!(
        check.status,
        crate::style::StyleCheckStatus::InsufficientSources
    );
    assert!(check.score.is_none() && !check.passes);
    assert_eq!(blockers(&flow), [ReadinessCheckId::Style]);
    style
        .confirm(&check.id, "сверила с прошлыми работами")
        .unwrap();
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
    flow.mark_ready_ok(true);
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
                    authority: super::dto::CommentAuthority::TeacherOfficial,
                    author: String::new(),
                    context: String::new(),
                    confirmed_official: true,
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
                            authority: super::dto::CommentAuthority::Unknown,
                            author: String::new(),
                            context: String::new(),
                            confirmed_official: false,
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

// ───────────────────────── Этап 7: авторитетность и устаревание ─────────────────────────

fn comment(
    authority: super::dto::CommentAuthority,
    confirmed_official: bool,
) -> TeacherCommentInput {
    TeacherCommentInput {
        source: TeacherCommentSource::Chat,
        date: "2026-10-22".into(),
        question: "Нужно ли приложить расчёт?".into(),
        answer: "Да, отдельным листом".into(),
        authority,
        author: "Автор".into(),
        context: "https://t.me/c/1/2".into(),
        confirmed_official,
    }
}

impl Flow {
    fn add_comment(&self, input: TeacherCommentInput) -> super::dto::TeacherCommentDto {
        self.service()
            .add_teacher_comments(&op(), HW, &[input])
            .unwrap()
            .value
            .remove(0)
    }

    fn requirement_from(
        &self,
        comment_id: &str,
    ) -> Result<super::dto::TeacherRequirementDto, ErrorCode> {
        self.service()
            .add_teacher_requirement(
                &op(),
                HW,
                &TeacherRequirementInput {
                    comment_id: comment_id.to_owned(),
                    text: "Приложить расчёт отдельным листом".into(),
                    origin: "Да, отдельным листом".into(),
                    status: RequirementStatus::Done,
                    note: None,
                },
            )
            .map(|success| success.value)
            .map_err(|failure| failure.error.code)
    }

    fn review_is_stale(&self) -> bool {
        let snapshot = self.snapshot();
        let id = self.review_run.clone().unwrap();
        snapshot.runs.iter().find(|run| run.id == id).unwrap().stale
    }
}

#[test]
fn only_confirmed_teacher_or_assistant_answers_become_requirements() {
    use super::dto::CommentAuthority::*;
    let mut flow = Flow::new();
    flow.advance_to_teacher_review();
    for authority in [Student, GroupChatUnverified, UserNote, System, Unknown] {
        let error = flow
            .service()
            .add_teacher_comments(&op(), HW, &[comment(authority, true)])
            .unwrap_err()
            .error;
        assert_eq!(
            error.code,
            ErrorCode::Blocked,
            "{authority:?} не может быть официальным"
        );
        // Без подтверждения сохраняется, но требование из него создать нельзя.
        let saved = flow.add_comment(comment(authority, false));
        assert!(!saved.official);
        assert_eq!(
            flow.requirement_from(&saved.id).unwrap_err(),
            ErrorCode::Blocked
        );
    }
    // Ответ преподавателя без подтверждения пользователя — тоже не официальный.
    let unconfirmed = flow.add_comment(comment(TeacherOfficial, false));
    assert_eq!(
        flow.requirement_from(&unconfirmed.id).unwrap_err(),
        ErrorCode::Blocked
    );
    // Неофициальные ответы не меняют входные данные: проверка действует.
    assert!(!flow.review_is_stale());

    for authority in [TeacherOfficial, TeachingAssistant] {
        let official = flow.add_comment(comment(authority, true));
        assert!(official.official && official.official_confirmed_at.is_some());
        assert_eq!(official.author, "Автор");
        assert_eq!(official.context, "https://t.me/c/1/2");
        assert!(flow.requirement_from(&official.id).is_ok());
    }
    // Даже триггер SQLite не даёт сделать студента официальным в обход сервиса.
    assert!(flow
        .database
        .with_connection(|c| {
            c.execute(
                "UPDATE teacher_comments SET authority='student', official_confirmed_at='x'
                  WHERE official_confirmed_at IS NULL",
                [],
            )?;
            Ok(())
        })
        .is_err());
}

#[test]
fn new_official_requirement_makes_review_readiness_and_ready_stale() {
    use super::dto::CommentAuthority::*;
    let mut flow = Flow::new();
    flow.advance_to_ready();
    assert!(!flow.service().aggregate(HW).unwrap().ready_stale);

    // Мнение студента ничего не меняет.
    flow.add_comment(comment(Student, false));
    assert_eq!(flow.stage(), Stage::ReadyToSubmit);
    assert!(!flow.review_is_stale());

    // Официальный ответ преподавателя — новое входное данное: готовность снята.
    let official = flow.add_comment(comment(TeacherOfficial, true));
    assert_eq!(flow.stage(), Stage::TeacherReview);
    assert!(flow.review_is_stale(), "прежняя проверка устарела");
    assert_eq!(
        blockers(&flow),
        vec![ReadinessCheckId::InputsFresh, ReadinessCheckId::Style]
    );
    let error = flow
        .service()
        .mark_ready(&op(), HW, false)
        .unwrap_err()
        .error;
    assert_eq!(error.code, ErrorCode::Blocked);

    // Устаревшая проверка остаётся в истории.
    let old_review = flow.review_run.clone().unwrap();
    flow.requirement_from(&official.id).unwrap();
    flow.rereview();
    let snapshot = flow.snapshot();
    assert!(snapshot
        .runs
        .iter()
        .any(|run| run.id == old_review && run.stale));
    assert!(blockers(&flow).is_empty());
    flow.mark_ready_ok(false);
    assert_eq!(flow.stage(), Stage::ReadyToSubmit);
}

#[test]
fn revoking_or_withdrawing_official_requirements_invalidates_and_keeps_history() {
    use super::dto::CommentAuthority::*;
    let mut flow = Flow::new();
    flow.advance_to_ready();
    let official = flow
        .snapshot()
        .teacher_comments
        .into_iter()
        .find(|comment| comment.official)
        .unwrap();

    // Отзыв официальности: требования отозваны, но видны в истории.
    flow.service()
        .set_comment_authority(
            &op(),
            &official.id,
            &super::service::CommentAuthorityInput {
                authority: GroupChatUnverified,
                author: String::new(),
                context: "переслано из общего чата".into(),
                confirmed_official: false,
            },
        )
        .unwrap();
    assert_eq!(flow.stage(), Stage::TeacherReview);
    let snapshot = flow.snapshot();
    assert!(snapshot
        .teacher_requirements
        .iter()
        .all(|requirement| requirement.withdrawn_at.is_some()));
    assert!(flow.review_is_stale());
    // Нельзя вернуть официальность ответу из общего чата.
    let error = flow
        .service()
        .set_comment_authority(
            &op(),
            &official.id,
            &super::service::CommentAuthorityInput {
                authority: GroupChatUnverified,
                author: String::new(),
                context: String::new(),
                confirmed_official: true,
            },
        )
        .unwrap_err()
        .error;
    assert_eq!(error.code, ErrorCode::Blocked);

    // Ассистент подтверждён — новое требование и затем его удаление.
    flow.service()
        .set_comment_authority(
            &op(),
            &official.id,
            &super::service::CommentAuthorityInput {
                authority: TeachingAssistant,
                author: "Ассистент".into(),
                context: String::new(),
                confirmed_official: true,
            },
        )
        .unwrap();
    let requirement = flow.requirement_from(&official.id).unwrap();
    flow.rereview();
    flow.mark_ready_ok(false);
    flow.service()
        .withdraw_requirement(&op(), &requirement.id)
        .unwrap();
    assert_eq!(flow.stage(), Stage::TeacherReview);
    assert!(flow.review_is_stale());
    let withdrawn = flow
        .snapshot()
        .teacher_requirements
        .into_iter()
        .find(|item| item.id == requirement.id)
        .unwrap();
    assert!(
        withdrawn.withdrawn_at.is_some(),
        "отозванное требование хранится"
    );
    // Отозванное требование неизменяемо.
    assert!(flow
        .service()
        .resolve_requirement(&op(), &requirement.id, RequirementStatus::NotDone, "")
        .is_err());
}

#[test]
fn materials_task_and_style_changes_invalidate_results() {
    // Смена выбранной версии проверена в
    // committed_prompts_are_immutable_and_new_prompts_are_new_rows (версии неизменяемы).
    for change in [
        "UPDATE files SET content_hash = 'changed' WHERE category IN ('lecture','seminar','material','homework_data')",
        "UPDATE files SET content_hash = 'changed' WHERE category = 'homework_task'",
        "UPDATE homeworks SET assignment_url = 'https://my.centraluniversity.ru/new' WHERE id = 'hw-1'",
        "INSERT INTO style_sources(id, relative_path, source_kind, language, document_type, sha256, read_status)
         VALUES ('style-new', 'ПРОФИЛЬ_СТИЛЯ_ДЗ.md', 'style_markdown', 'ru', 'markdown', NULL, 'ok')",
    ] {
        let mut flow = Flow::new();
        flow.advance_to_ready();
        let before = flow.snapshot().inputs;
        flow.database
            .with_connection(|c| {
                c.execute(change, [])?;
                Ok(())
            })
            .unwrap();
        let after = flow.snapshot().inputs;
        assert_ne!(before.combined, after.combined, "{change}");
        assert!(flow.review_is_stale(), "{change}");
        let aggregate = flow.service().aggregate(HW).unwrap();
        assert!(aggregate.ready_stale, "{change}: «Готово к сдаче» не действует");
        assert!(!aggregate.readiness.ready);
        assert!(aggregate
            .readiness
            .blockers()
            .contains(&ReadinessCheckId::InputsFresh));
    }
}

#[test]
fn stale_revision_prompt_cannot_be_handed_off_until_edited() {
    use super::dto::CommentAuthority::*;
    let mut flow = Flow::new();
    flow.advance_to_needs_fixes();
    let run = flow.review_run.clone().unwrap();
    let prompt = flow
        .service()
        .create_revision_prompt(&op(), &run, None)
        .unwrap()
        .value;
    assert!(!prompt.stale);
    let official = flow.add_comment(comment(TeacherOfficial, true));
    flow.requirement_from(&official.id).unwrap();
    // Правка черновика — решение пользователя по актуальным данным.
    let edited = flow
        .service()
        .update_prompt_draft(&op(), &prompt.id, "Исправь формулу и приложи расчёт")
        .unwrap()
        .value;
    assert!(!edited.stale);
    flow.service().commit_prompt(&op(), &prompt.id).unwrap();
    // Новое официальное требование после фиксации — промт устарел.
    let another = flow.add_comment(comment(TeachingAssistant, true));
    flow.requirement_from(&another.id).unwrap();
    let error = flow
        .service()
        .hand_off_prompt(&op(), &prompt.id)
        .unwrap_err()
        .error;
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(flow
        .snapshot()
        .prompts
        .iter()
        .any(|item| item.id == prompt.id && item.stale));
}

#[test]
fn not_done_requirement_reopens_ready_homework() {
    let mut flow = Flow::new();
    flow.advance_to_ready();
    let requirement = flow.snapshot().teacher_requirements[0].id.clone();
    flow.service()
        .resolve_requirement(
            &op(),
            &requirement,
            RequirementStatus::NotDone,
            "не приложен",
        )
        .unwrap();
    assert_eq!(flow.stage(), Stage::TeacherReview);
}

// ───────────────────────── Этап 8: неизменяемые ревизии комплекта ─────────────────────────

struct Vault {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
}

impl Flow {
    /// Временный vault с файлами из seed и итоговые фамилия/имя.
    fn with_vault(&self) -> Vault {
        use crate::intake::{RootKind, RootsService};
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().join("vault");
        let backup = temp.path().join("backup");
        std::fs::create_dir_all(&backup).unwrap();
        for (path, body) in [
            (
                "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_v1.docx",
                "v1",
            ),
            (
                "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_v2.docx",
                "v2",
            ),
            (
                "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Приложение.xlsx",
                "a,b",
            ),
        ] {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        let roots = RootsService::new(&self.database, None);
        roots.set_root(RootKind::Vault, &root).unwrap();
        roots.set_root(RootKind::Backup, &backup).unwrap();
        let rules = crate::dto::AutomationRulesDto {
            student_name: crate::dto::StudentNameDto {
                last_name: "Гончарова".into(),
                first_name: "Доминика".into(),
            },
            ..Default::default()
        };
        crate::mutation_service::MutationService::new(&self.database)
            .update_automation_rules(rules)
            .unwrap();
        Vault { _temp: temp, root }
    }

    fn submission(&self) -> crate::submission::SubmissionService<'_> {
        crate::submission::SubmissionService::new(&self.database, None)
    }

    fn confirm_manual_checks(&self, package_id: &str) {
        use super::dto::PackageCheckId;
        for check in [PackageCheckId::Links, PackageCheckId::ManualReview] {
            let _ = self.submission().set_check(package_id, check, true);
        }
    }
}

fn confirmation() -> super::dto::SubmissionConfirmationInput {
    super::dto::SubmissionConfirmationInput {
        submitted_at: "2026-10-26T12:00:00Z".into(),
        assignment_url: "https://my.centraluniversity.ru/task".into(),
        proof: None,
        user_confirmed: true,
    }
}

fn ready_with_link() -> (Flow, Vault) {
    let mut flow = Flow::new();
    flow.database
        .with_connection(|c| {
            c.execute(
                "UPDATE homeworks SET assignment_url = 'https://my.centraluniversity.ru/task' WHERE id = 'hw-1'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let vault = flow.with_vault();
    flow.advance_to_ready();
    (flow, vault)
}

#[test]
fn package_build_is_blocked_until_current_ready() {
    let mut flow = Flow::new();
    let _vault = flow.with_vault();
    flow.advance_to_teacher_review();
    assert_eq!(
        flow.submission().build(HW).unwrap_err().code,
        ErrorCode::Blocked
    );
    flow.add_done_requirement();
    flow.mark_ready_ok(false);
    // Изменились материалы — «Готово к сдаче» не действует, комплект не собрать.
    flow.database
        .with_connection(|c| {
            c.execute(
                "UPDATE files SET content_hash = 'changed' WHERE id = 'lecture-file'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        flow.submission().build(HW).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn every_rebuild_is_a_new_immutable_revision_with_full_contents() {
    use super::dto::{PackageCheckId, PackageItemRole, PackageStatus};
    let (flow, vault) = ready_with_link();
    let first = flow.submission().build(HW).unwrap().value;
    assert_eq!(first.revision, 1);
    assert!(first.folder_path.ends_with("03 На сдачу/Ревизия 01"));
    let main = first
        .items
        .iter()
        .find(|i| i.role == PackageItemRole::Main)
        .unwrap();
    assert!(main.final_name.starts_with("Гончарова_Доминика_Неделя7 ДЗ"));
    assert_eq!(main.status, PackageStatus::Verified);
    assert_eq!(main.sha256.as_deref().map(str::len), Some(64));
    assert!(vault
        .root
        .join(&main.file.as_ref().unwrap().relative_path)
        .is_file());
    assert!(first.inputs.is_some() && first.inputs_fingerprint.is_some());
    assert_eq!(
        first.requirements.len(),
        1,
        "актуальные требования в ревизии"
    );
    // Ссылка не считается доступной без ручного подтверждения.
    assert_eq!(first.links[0].status, PackageStatus::ManualRequired);
    assert!(!first.links[0].verified);
    let links = first
        .checklist
        .iter()
        .find(|c| c.id == PackageCheckId::Links)
        .unwrap();
    assert_eq!(links.status, PackageStatus::ManualRequired);
    assert!(!links.done);
    assert!(!first.stale);

    let second = flow.submission().build(HW).unwrap().value;
    assert_eq!(second.revision, 2);
    assert_ne!(
        second.id, first.id,
        "пересборка не возвращает старый комплект"
    );
    assert_ne!(second.folder_path, first.folder_path);
    let old = flow
        .database
        .with_connection(|c| repository::submission_package_by_id(c, &first.id))
        .unwrap();
    assert!(old.stale, "прежняя ревизия в истории, но не действует");
    let history = flow.service().aggregate(HW).unwrap().package_history;
    assert_eq!(
        history
            .iter()
            .map(|h| (h.revision, h.stale))
            .collect::<Vec<_>>(),
        [(2, false), (1, true)]
    );
    // Отметить пункт старой ревизии нельзя; содержимое и удаление запрещены триггерами.
    assert_eq!(
        flow.submission()
            .set_check(&first.id, PackageCheckId::ManualReview, true)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    for sql in [
        "UPDATE submission_packages SET items_json = '[]' WHERE revision = 1",
        "UPDATE submission_packages SET status = 'ready' WHERE revision = 1",
        "DELETE FROM submission_packages WHERE revision = 1",
    ] {
        assert!(
            flow.database
                .with_connection(|c| {
                    c.execute(sql, [])?;
                    Ok(())
                })
                .is_err(),
            "{sql}"
        );
    }
    // Автоматический пункт вручную не отмечается.
    assert_eq!(
        flow.submission()
            .set_check(&second.id, PackageCheckId::Attachments, true)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn confirm_is_blocked_by_manual_missing_mismatch_and_stale() {
    use super::dto::CommentAuthority;
    let (flow, vault) = ready_with_link();
    let package = flow.submission().build(HW).unwrap().value;
    // Ручные пункты не подтверждены.
    assert_eq!(
        flow.submission()
            .confirm(HW, &confirmation(), None)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    flow.confirm_manual_checks(&package.id);
    let main = vault
        .root
        .join(&package.items[0].file.as_ref().unwrap().relative_path);
    // Файл изменён после сборки — mismatch.
    std::fs::write(&main, "подменённое содержимое").unwrap();
    let verified = flow.submission().verify(HW).unwrap().unwrap();
    assert_eq!(
        verified.items[0].status,
        super::dto::PackageStatus::Mismatch
    );
    assert_eq!(
        flow.submission()
            .confirm(HW, &confirmation(), None)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    // Файла нет — missing.
    std::fs::remove_file(&main).unwrap();
    assert_eq!(
        flow.submission().verify(HW).unwrap().unwrap().items[0].status,
        super::dto::PackageStatus::Missing
    );
    assert_eq!(
        flow.submission()
            .confirm(HW, &confirmation(), None)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    // Новое официальное требование — комплект и готовность устарели.
    let fresh = flow.submission().build(HW).unwrap().value;
    flow.confirm_manual_checks(&fresh.id);
    let official = flow.add_comment(comment(CommentAuthority::TeacherOfficial, true));
    assert_eq!(flow.stage(), Stage::TeacherReview);
    let stale = flow
        .database
        .with_connection(|c| repository::submission_package(c, HW))
        .unwrap()
        .unwrap();
    assert!(stale.stale);
    assert_eq!(
        flow.submission()
            .confirm(HW, &confirmation(), None)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    assert_eq!(
        flow.submission().build(HW).unwrap_err().code,
        ErrorCode::Blocked
    );
    let _ = official;
}

#[test]
fn snapshot_is_created_only_from_current_verified_package_and_locks_homework() {
    use super::dto::PackageCheckId;
    let (flow, vault) = ready_with_link();
    let package = flow.submission().build(HW).unwrap().value;
    flow.confirm_manual_checks(&package.id);
    let receipt = flow
        .submission()
        .confirm(HW, &confirmation(), None)
        .unwrap()
        .value;
    assert_eq!(receipt.package_id, package.id);
    assert!(vault.root.join(&receipt.snapshot_path).is_dir());
    assert_eq!(receipt.snapshot_hash.len(), 64);
    let submitted = flow
        .database
        .with_connection(|c| repository::submission_package(c, HW))
        .unwrap()
        .unwrap();
    assert!(submitted.submitted && !submitted.stale);
    assert_eq!(
        flow.submission().build(HW).unwrap_err().code,
        ErrorCode::Immutable
    );
    assert_eq!(
        flow.submission()
            .set_check(&package.id, PackageCheckId::ManualReview, false)
            .unwrap_err()
            .code,
        ErrorCode::Immutable
    );
    assert_eq!(
        flow.service()
            .add_teacher_comments(
                &op(),
                HW,
                &[comment(super::dto::CommentAuthority::Student, false)]
            )
            .unwrap_err()
            .error
            .code,
        ErrorCode::Immutable
    );
}

// ───────────────────────── Этап 9: профиль личного стиля ─────────────────────────

const MD_DRAFT: &str = "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение.md";

impl Flow {
    fn style(&self) -> crate::style::StyleService<'_> {
        crate::style::StyleService::new(&self.database, None)
    }

    /// Версия-Markdown с заданным текстом становится итоговой.
    fn md_version(&mut self, vault: &Vault, text: &str) {
        std::fs::write(vault.root.join(MD_DRAFT), text).unwrap();
        self.database
            .with_connection(|c| {
                c.execute(
                    "INSERT OR IGNORE INTO files(id, week_id, homework_id, category, name, kind, size_bytes, modified_at, relative_path)
                     VALUES ('draft-md', NULL, 'hw-1', 'homework_draft', 'Решение.md', 'text', 10, '2026-10-22T10:00:00Z', ?1)",
                    [MD_DRAFT],
                )?;
                Ok(())
            })
            .unwrap();
        let version = self
            .service()
            .create_version(&op(), HW, &version_input("draft-md"))
            .unwrap()
            .value;
        self.version = Some(version.id);
    }
}

fn write_sample(vault: &Vault, path: &str, text: &str) {
    let full = vault.root.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

#[test]
fn style_sources_are_explicit_and_profile_needs_enough_confirmed_text() {
    use crate::style::{ProfileStatus, StyleSourceKind};
    let flow = Flow::new();
    let vault = flow.with_vault();
    let style = flow.style();
    // Без источников — ничего не сканируется, профиль непригоден.
    let empty = style.rebuild("ru").unwrap();
    assert!(empty.sources.is_empty());
    let first = empty.current.unwrap();
    assert_eq!(first.status, ProfileStatus::InsufficientSources);
    assert!(first.content.is_none() && !first.unusable_reasons.is_empty());

    // Только файлы внутри хранилища и только существующие.
    for bad in ["/etc/passwd", "../outside.md", "нет/такого.md"] {
        assert!(style
            .add_source(bad, StyleSourceKind::PreviousHomework, "ru")
            .is_err());
    }
    write_sample(
        &vault,
        "Стиль/ПРОФИЛЬ_СТИЛЯ_ДЗ.md",
        "# Стиль\n- Без канцелярита\n- Короткие выводы",
    );
    write_sample(&vault, "Стиль/ДЗ1.md", &crate::style::tests::sample_text(2));
    assert!(style
        .add_source("Стиль/ДЗ1.md", StyleSourceKind::StyleMarkdown, "xx")
        .is_err());
    style
        .add_source(
            "Стиль/ПРОФИЛЬ_СТИЛЯ_ДЗ.md",
            StyleSourceKind::StyleMarkdown,
            "ru",
        )
        .unwrap();
    let state = style
        .add_source("Стиль/ДЗ1.md", StyleSourceKind::PreviousHomework, "ru")
        .unwrap();
    assert!(state.stale, "новый источник делает профиль устаревшим");
    let source = &state.sources[1];
    assert_eq!(source.sha256.as_deref().map(str::len), Some(64));
    assert_eq!(source.document_type, "markdown");
    assert_eq!(source.confirmation, "confirmed");
    assert!(source.last_read_at.is_some());
    assert!(
        style
            .add_source("Стиль/ДЗ1.md", StyleSourceKind::PreviousHomework, "ru")
            .is_err(),
        "повторный выбор того же файла"
    );

    // 2 повтора образца < 300 слов → insufficient_sources.
    let second = style.rebuild("ru").unwrap();
    let revision = second.current.unwrap();
    assert_eq!(revision.revision, 2);
    assert_eq!(revision.status, ProfileStatus::InsufficientSources);
    assert_eq!(revision.sources.len(), 2);

    write_sample(&vault, "Стиль/ДЗ2.md", &crate::style::tests::sample_text(6));
    style
        .add_source("Стиль/ДЗ2.md", StyleSourceKind::ConfirmedSample, "ru")
        .unwrap();
    let third = style.rebuild("ru").unwrap();
    let ready = third.current.clone().unwrap();
    assert_eq!((ready.revision, ready.status), (3, ProfileStatus::Ready));
    let content = ready.content.unwrap();
    assert_eq!(
        content.declared_rules,
        vec!["Без канцелярита", "Короткие выводы"]
    );
    assert!(content.features.words >= 300);
    assert!(!third.stale);
    // Без изменений новая ревизия не создаётся; история сохраняется.
    assert_eq!(style.rebuild("ru").unwrap().current.unwrap().revision, 3);
    assert_eq!(
        third.history.iter().map(|h| h.revision).collect::<Vec<_>>(),
        [3, 2, 1]
    );

    // Изменение содержимого → stale; исключение источника → следующая ревизия без него.
    write_sample(&vault, "Стиль/ДЗ1.md", &crate::style::tests::sample_text(3));
    assert!(style.refresh_sources().unwrap().stale);
    let removed = style.remove_source(&third.sources[1].id).unwrap();
    assert!(removed.sources[1].removed_at.is_some());
    let fourth = style.rebuild("ru").unwrap().current.unwrap();
    assert_eq!(fourth.revision, 4);
    assert!(fourth
        .sources
        .iter()
        .all(|s| s.relative_path != "Стиль/ДЗ1.md"));
    // Ревизии неизменяемы.
    assert!(flow
        .database
        .with_connection(|c| {
            c.execute(
                "UPDATE style_profile_revisions SET status = 'ready' WHERE revision = 1",
                [],
            )?;
            Ok(())
        })
        .is_err());
}

#[test]
fn style_check_reports_score_fragments_and_honest_statuses() {
    use crate::style::{StyleCheckStatus, StyleSourceKind};
    let mut flow = Flow::new();
    let vault = flow.with_vault();
    flow.advance_to_codex_review();
    // Итоговая версия .docx без текста → после профиля: ручная проверка.
    write_sample(&vault, "Стиль/ДЗ.md", &crate::style::tests::sample_text(8));
    let style = flow.style();
    style
        .add_source("Стиль/ДЗ.md", StyleSourceKind::PreviousHomework, "ru")
        .unwrap();
    style.rebuild("ru").unwrap();
    let manual = style.check(HW).unwrap();
    assert_eq!(manual.status, StyleCheckStatus::ManualRequired);
    assert!(manual.score.is_none() && !manual.passes);

    // Текст в собственном стиле → оценка в пределах порога.
    flow.md_version(&vault, &crate::style::tests::sample_text(3));
    let style = flow.style();
    let close = style.check(HW).unwrap();
    assert_eq!(close.status, StyleCheckStatus::Checked);
    assert!(close.score.unwrap() <= close.threshold && close.passes);
    assert!(close.body.explanation.contains("не доказывает"));
    assert!(!close.body.sample_refs.is_empty());
    assert_eq!(close.profile_revision, Some(1));
    // Подтверждать нечего.
    assert!(style.confirm(&close.id, "").is_err());

    // Нехарактерный текст → фрагменты с объяснениями; без подтверждения не проходит.
    let odd = "Важно отметить, что в современном мире аналитика играет ключевую роль. ".repeat(4)
        + &"Очевидно, что данный подход безусловно является наилучшим, поскольку он обеспечивает широкий спектр возможностей, позволяет учитывать множество факторов одновременно и гарантирует высокую точность прогнозов на всех этапах работы организации. ".repeat(4);
    std::fs::write(vault.root.join(MD_DRAFT), &odd).unwrap();
    let far = style.check(HW).unwrap();
    assert_eq!(far.status, StyleCheckStatus::Checked);
    assert!(far.score.unwrap() > far.threshold, "{:?}", far.score);
    assert!(!far.passes && !far.body.fragments.is_empty());
    assert!(far.body.fragments.iter().all(|f| !f.explanation.is_empty()));
    let confirmed = style.confirm(&far.id, "перечитала, это мой текст").unwrap();
    assert!(confirmed.passes);

    // Изменение профиля делает проверку стиля устаревшей (общий отпечаток этапа 7).
    write_sample(&vault, "Стиль/ДЗ.md", &crate::style::tests::sample_text(9));
    let refreshed = style.refresh_sources().unwrap();
    assert!(refreshed.stale);
    let stale = crate::style::StyleService::new(&flow.database, None);
    let reread = flow
        .database
        .with_connection(|c| crate::style::check_by_id(c, &confirmed.id))
        .unwrap();
    assert_eq!(reread.status, StyleCheckStatus::Stale);
    assert!(!reread.passes);
    // Проверка по устаревшему профилю не выполняется.
    assert_eq!(
        stale.check(HW).unwrap().status,
        StyleCheckStatus::NotChecked
    );
}

#[test]
fn style_profile_change_invalidates_review_readiness_and_package() {
    use crate::style::StyleSourceKind;
    let (flow, vault) = ready_with_link();
    let package = flow.submission().build(HW).unwrap().value;
    assert!(!package.stale);
    write_sample(&vault, "Стиль/ДЗ.md", &crate::style::tests::sample_text(8));
    flow.style()
        .add_source("Стиль/ДЗ.md", StyleSourceKind::PreviousHomework, "ru")
        .unwrap();
    flow.style().rebuild("ru").unwrap();
    let aggregate = flow.service().aggregate(HW).unwrap();
    assert!(aggregate.ready_stale);
    assert!(aggregate
        .readiness
        .blockers()
        .contains(&ReadinessCheckId::Style));
    assert!(aggregate
        .readiness
        .blockers()
        .contains(&ReadinessCheckId::InputsFresh));
    assert!(aggregate.submission_package.unwrap().stale);
}

// ───────────────────────── Этап 10: родословная и сравнение ─────────────────────────

fn decision(flow: &Flow, version_id: &str) -> crate::dto::VersionLineageDto {
    flow.snapshot()
        .homework
        .versions
        .into_iter()
        .find(|v| v.id == version_id)
        .unwrap()
        .lineage
}

impl Flow {
    /// Нужны исправления → промт исправлений передан автору (этап «Решение»).
    fn revise(&mut self) {
        let review = self.review_run.clone().unwrap();
        let prompt = self
            .service()
            .create_revision_prompt(&op(), &review, None)
            .unwrap()
            .value;
        self.service().commit_prompt(&op(), &prompt.id).unwrap();
        self.service().hand_off_prompt(&op(), &prompt.id).unwrap();
    }

    fn versions(&self) -> crate::versions::VersionService<'_> {
        crate::versions::VersionService::new(&self.database, None)
    }

    /// Файл ДЗ с текстом в vault и версия из него (родитель — текущая версия).
    fn text_version(&mut self, vault: &Vault, id: &str, path: &str, text: &str) -> String {
        write_sample(vault, path, text);
        let name = path.rsplit('/').next().unwrap();
        self.database
            .with_connection(|c| {
                c.execute(
                    "INSERT OR REPLACE INTO files(id, week_id, homework_id, category, name, kind, size_bytes, modified_at, relative_path)
                     VALUES (?1, NULL, 'hw-1', 'homework_draft', ?2, 'text', 10, '2026-10-22T10:00:00Z', ?3)",
                    params![id, name, path],
                )?;
                Ok(())
            })
            .unwrap();
        let version = self
            .service()
            .create_version(&op(), HW, &version_input(id))
            .unwrap()
            .value;
        self.version = Some(version.id.clone());
        version.id
    }
}

use rusqlite::params;

#[test]
fn child_version_records_parent_origin_and_reason() {
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    let first = flow.version.clone().unwrap();
    let parent = decision(&flow, &first);
    assert_eq!(parent.parent_version_id, None);
    assert_eq!(parent.origin, crate::dto::VersionOrigin::ClaudeResult);
    assert_eq!(parent.decision, crate::dto::VersionDecision::Pending);
    assert!(parent.inputs_fingerprint.is_some());
    let run = flow.review(&failing_review());
    flow.revise();
    let child = flow.service().create_version(
        &op(),
        HW,
        &VersionInput {
            creation_reason: Some("Исправлена формула выручки".into()),
            review_run_id: Some(run.clone()),
            origin: Some(crate::dto::VersionOrigin::ManualEdit),
            author: Some(VersionAuthor::User),
            ..version_input("draft-2")
        },
    );
    let child = match child {
        Ok(result) => result.value,
        Err(error) => panic!("{error:?} на этапе {:?}", flow.stage()),
    };
    let lineage = child.lineage.clone();
    assert_eq!(lineage.parent_version_id.as_deref(), Some(first.as_str()));
    assert_eq!(lineage.origin, crate::dto::VersionOrigin::ManualEdit);
    assert_eq!(lineage.creation_reason, "Исправлена формула выручки");
    assert_eq!(lineage.review_run_id.as_deref(), Some(run.as_str()));
    assert_eq!(lineage.created_by, "user");
    let chain = flow.versions().lineage(HW, &child.id).unwrap();
    assert_eq!(
        chain.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        vec![first.as_str(), child.id.as_str()]
    );
}

#[test]
fn lineage_forbids_cycles_foreign_parents_and_mutation() {
    let mut flow = Flow::new();
    flow.advance_to_codex_review();
    let first = flow.version.clone().unwrap();
    // Версии неизменяемы: родителя нельзя переписать (и замкнуть цикл).
    let update = flow.database.with_connection(|c| {
        c.execute(
            "UPDATE homework_versions SET parent_version_id = id WHERE id = ?1",
            [&first],
        )?;
        Ok(())
    });
    assert!(update.is_err());
    let rename = flow.database.with_connection(|c| {
        c.execute(
            "UPDATE homework_versions SET note = 'x' WHERE id = ?1",
            [&first],
        )?;
        Ok(())
    });
    assert!(rename.is_err());
    // Родитель — сама версия: запрещено триггером.
    let self_parent = flow.database.with_connection(|c| {
        c.execute(
            "INSERT INTO homework_versions(id, homework_id, number, label, file_name, file_path, size_bytes,
                source_file_id, author, files_json, parent_version_id)
             SELECT 'loop', homework_id, 99, 'v99', file_name, file_path, size_bytes, source_file_id, author,
                files_json, 'loop' FROM homework_versions WHERE id = ?1",
            [&first],
        )?;
        Ok(())
    });
    assert!(self_parent.is_err());
    // Родитель из другого ДЗ (или другого рабочего пространства — там этой версии нет).
    flow.database
        .with_connection(|c| {
            c.execute_batch(
                "INSERT INTO homeworks(id, week_id, ordinal, title, deadline, stage, workflow_stage, folder_path)
                 SELECT 'hw-2', week_id, 99, 'Другое ДЗ', deadline, stage, workflow_stage, folder_path || '-2' FROM homeworks WHERE id = 'hw-1';",
            )?;
            Ok(())
        })
        .unwrap();
    let foreign = flow.database.with_connection(|c| {
        c.execute(
            "INSERT INTO homework_versions(id, homework_id, number, label, file_name, file_path, size_bytes,
                source_file_id, author, files_json, parent_version_id)
             SELECT 'foreign', 'hw-2', 1, 'v01', file_name, file_path, size_bytes, source_file_id, author,
                files_json, id FROM homework_versions WHERE id = ?1",
            [&first],
        )?;
        Ok(())
    });
    assert!(foreign.is_err());
    let error = flow
        .service()
        .create_version(
            &op(),
            HW,
            &VersionInput {
                parent_version_id: Some("version-from-other-workspace".into()),
                ..version_input("draft-2")
            },
        )
        .unwrap_err();
    assert_eq!(error.error.code, ErrorCode::Conflict);
}

#[test]
fn accept_reject_and_replace_final_with_invalidation() {
    let (mut flow, vault) = ready_with_link();
    let first = flow.version.clone().unwrap();
    assert_eq!(
        decision(&flow, &first).decision,
        crate::dto::VersionDecision::Final
    );
    let package = flow.submission().build(HW).unwrap().value;
    assert!(!package.stale);
    // Ready → проверка преподавателя → снова проверка Codex: можно добавить версию.
    flow.service()
        .transition(&op(), HW, Stage::TeacherReview, None)
        .unwrap();
    flow.service()
        .transition(&op(), HW, Stage::CodexReview, None)
        .unwrap();
    // Текущую версию нельзя отклонить, а отклонение требует причины.
    let current = flow
        .service()
        .reject_version(&op(), &first, "плохо")
        .unwrap_err();
    assert_eq!(current.error.code, ErrorCode::Blocked);
    let second = flow.text_version(
        &vault,
        "draft-md-2",
        "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение2.md",
        "# v2",
    );
    assert_eq!(
        decision(&flow, &second).parent_version_id.as_deref(),
        Some(first.as_str())
    );
    let third = flow.text_version(
        &vault,
        "draft-md-3",
        "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение3.md",
        "# v3",
    );
    assert!(flow.service().reject_version(&op(), &second, "  ").is_err());
    flow.service()
        .reject_version(&op(), &second, "Потеряны выводы")
        .unwrap();
    let rejected = decision(&flow, &second);
    assert_eq!(rejected.decision, crate::dto::VersionDecision::Rejected);
    assert_eq!(rejected.decision_reason.as_deref(), Some("Потеряны выводы"));
    assert!(rejected.decided_at.is_some());
    // Отклонённую версию нельзя назначить итоговой, пока её не примут.
    assert_eq!(
        flow.service()
            .select_version(&op(), &second)
            .unwrap_err()
            .error
            .code,
        ErrorCode::Blocked
    );
    flow.service().accept_version(&op(), &second, "").unwrap();
    assert_eq!(
        decision(&flow, &second).decision,
        crate::dto::VersionDecision::Accepted
    );
    // Замена итоговой: прежняя итоговая — «заменена», новая — «итоговая».
    flow.service().select_version(&op(), &second).unwrap();
    assert_eq!(
        decision(&flow, &second).decision,
        crate::dto::VersionDecision::Final
    );
    let superseded = decision(&flow, &first);
    assert_eq!(superseded.decision, crate::dto::VersionDecision::Superseded);
    assert_eq!(
        superseded.superseded_by_version_id.as_deref(),
        Some(second.as_str())
    );
    assert_eq!(
        decision(&flow, &third).decision,
        crate::dto::VersionDecision::Pending
    );
    // Существующий механизм этапа 7–8: проверки, готовность и комплект устарели.
    let aggregate = flow.service().aggregate(HW).unwrap();
    assert!(aggregate.submission_package.unwrap().stale);
    assert!(!aggregate.readiness.blockers().is_empty());
    // Журнал решений только дополняется.
    let tamper = flow.database.with_connection(|c| {
        c.execute("DELETE FROM version_decisions", [])?;
        Ok(())
    });
    assert!(tamper.is_err());
}

#[test]
fn stored_comparison_is_reused_and_becomes_stale_after_file_change() {
    let mut flow = Flow::new();
    let vault = flow.with_vault();
    flow.advance_to_codex_review();
    let a_path = "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_a.md";
    let b_path = "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_b.md";
    let a = flow.text_version(&vault, "md-a", a_path, "# Решение\n\nСреднее 10.\n");
    let b = flow.text_version(
        &vault,
        "md-b",
        b_path,
        "# Решение\n\nСреднее 12.\n\n## Выводы\n\nЕсть рост.\n",
    );
    let first = flow.versions().compare(HW, &a, &b).unwrap();
    assert!(!first.stale);
    assert_eq!(
        first.status,
        crate::versions::diff::ComparisonStatus::Complete
    );
    assert!(first
        .changes
        .iter()
        .any(|c| c.category == crate::versions::diff::ChangeCategory::Heading));
    assert_eq!(
        flow.versions().compare(HW, &a, &b).unwrap().id,
        first.id,
        "свежее сравнение переиспользуется"
    );
    assert!(flow.versions().compare(HW, &a, &a).is_err());
    write_sample(&vault, b_path, "# Решение\n\nСреднее 13.\n");
    assert!(flow.versions().comparison(&first.id).unwrap().stale);
    let fresh = flow.versions().compare(HW, &a, &b).unwrap();
    assert_ne!(fresh.id, first.id);
    assert!(!fresh.stale);
    // Сохранённые сравнения неизменяемы.
    let tamper = flow.database.with_connection(|c| {
        c.execute("UPDATE version_comparisons SET summary = 'x'", [])?;
        Ok(())
    });
    assert!(tamper.is_err());
}

#[test]
fn correction_plan_comes_from_current_review_and_requirements() {
    use crate::versions::CorrectionStatus;
    let mut flow = Flow::new();
    let vault = flow.with_vault();
    flow.advance_to_needs_fixes();
    let version = flow.version.clone().unwrap();
    let plan = flow.versions().build_plan(HW).unwrap();
    assert_eq!(plan.version_id, version);
    assert!(!plan.stale);
    assert_eq!(plan.items.len(), 1);
    let item = &plan.items[0];
    assert_eq!(item.origin, "review");
    assert_eq!(item.priority, "high");
    assert_eq!(
        item.status,
        CorrectionStatus::Proposed,
        "предложение AI ждёт подтверждения"
    );
    assert_eq!(item.location.as_deref(), Some("лист «Расчёт», B4"));
    assert_eq!(item.linked_review_run_id, flow.review_run);
    // Пока не подтверждено — нельзя отметить исправленным.
    assert_eq!(
        flow.versions()
            .set_item_status(&item.id, CorrectionStatus::Resolved, Some(&version))
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    flow.versions()
        .set_item_status(&item.id, CorrectionStatus::Open, None)
        .unwrap();
    // Новое невыполненное требование делает план устаревшим; новый план его учитывает.
    let comments = flow
        .service()
        .add_teacher_comments(
            &op(),
            HW,
            &[comment(super::dto::CommentAuthority::TeacherOfficial, true)],
        )
        .unwrap()
        .value;
    flow.service()
        .add_teacher_requirement(
            &op(),
            HW,
            &TeacherRequirementInput {
                comment_id: comments[0].id.clone(),
                text: "Вывод отдельным листом".into(),
                origin: "Да, отдельным листом".into(),
                status: RequirementStatus::NotDone,
                note: None,
            },
        )
        .unwrap();
    assert!(flow.versions().current_plan(HW).unwrap().unwrap().stale);
    let plan = flow.versions().build_plan(HW).unwrap();
    assert!(!plan.stale);
    let requirement = plan
        .items
        .iter()
        .find(|i| i.origin == "requirement")
        .unwrap();
    assert_eq!(requirement.status, CorrectionStatus::Open);
    assert_eq!(requirement.priority, "high");
    assert_eq!(requirement.source_requirement, "Вывод отдельным листом");
    // Исправление связывается только с версией-потомком.
    assert_eq!(
        flow.versions()
            .set_item_status(&requirement.id, CorrectionStatus::Resolved, Some(&version))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    flow.revise();
    let fixed = flow.text_version(
        &vault,
        "md-fix",
        "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Исправлено.md",
        "# Вывод",
    );
    let plan = flow
        .versions()
        .set_item_status(&requirement.id, CorrectionStatus::Resolved, Some(&fixed))
        .unwrap();
    let resolved = plan.items.iter().find(|i| i.id == requirement.id).unwrap();
    assert_eq!(
        resolved.resolved_in_version_id.as_deref(),
        Some(fixed.as_str())
    );
    // Итоговая версия сменилась — план прежней версии устарел.
    assert!(plan.stale);
    // Содержимое пунктов неизменяемо.
    let tamper = flow.database.with_connection(|c| {
        c.execute("UPDATE correction_items SET issue = 'x'", [])?;
        Ok(())
    });
    assert!(tamper.is_err());
}
