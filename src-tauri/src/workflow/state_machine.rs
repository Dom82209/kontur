//! Централизованная state machine рабочего цикла ДЗ.
//!
//! Этап меняет только backend и только через [`check_transition`]: каждая пара
//! «откуда → куда» либо явно разрешена (с условием), либо запрещена. Перескочить
//! обязательный этап нельзя: например, из «Материалы собраны» нельзя сразу
//! перейти к проверке, а «Готово к сдаче» требует пройденной готовности.
//! Названия этапов исторические: провайдер роли задаётся в настройках.

use crate::{
    domain::HomeworkWorkflowStage as Stage,
    error::{AppError, BackendError},
};

use super::{
    dto::{PromptKind, PromptStatus, ReadinessDto, ReviewVerdict, RunKind, TransitionOptionDto},
    repository::WorkflowSnapshot,
};

pub const ALL_STAGES: [Stage; 9] = [
    Stage::MaterialsCollected,
    Stage::CodexAnalysis,
    Stage::PromptReady,
    Stage::ClaudeSolving,
    Stage::CodexReview,
    Stage::NeedsFixes,
    Stage::TeacherReview,
    Stage::ReadyToSubmit,
    Stage::Submitted,
];

/// Условие, при котором разрешённый переход выполним.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// Возврат на предыдущий этап — без условий.
    Back,
    /// Есть полный набор источников для анализа материалов.
    AnalysisBundle,
    /// Анализ сохранён и первичный промт зафиксирован.
    AnalysisAndPrompt,
    /// Зафиксированный промт передан автору.
    PromptHandedOff,
    /// Выбрана проверяемая версия.
    CurrentVersion,
    /// Проверка текущей версии требует исправлений.
    ReviewNeedsFixes,
    /// Проверка текущей версии пройдена без критических ошибок.
    ReviewPassed,
    /// Готовность пройдена, а проверка преподавателем не обязательна.
    ReadyWithoutTeacher,
    /// Готовность пройдена.
    Ready,
    /// Промт исправлений по текущей версии зафиксирован и передан автору.
    RevisionHandedOff,
    /// Пользователь решил вернуть работу на исправления.
    Reopen,
    /// Отправка подтверждается только на этапе 10 (снимок и receipt).
    SubmissionReceipt,
}

/// Полная таблица разрешённых переходов. Всё, чего здесь нет, запрещено.
pub fn guard_for(from: Stage, to: Stage) -> Option<Guard> {
    use Stage::*;
    Some(match (from, to) {
        (MaterialsCollected, CodexAnalysis) => Guard::AnalysisBundle,
        (CodexAnalysis, MaterialsCollected) => Guard::Back,
        (CodexAnalysis, PromptReady) => Guard::AnalysisAndPrompt,
        (PromptReady, CodexAnalysis) => Guard::Back,
        (PromptReady, ClaudeSolving) => Guard::PromptHandedOff,
        (ClaudeSolving, PromptReady) => Guard::Back,
        (ClaudeSolving, CodexReview) => Guard::CurrentVersion,
        (CodexReview, ClaudeSolving) => Guard::Back,
        (CodexReview, NeedsFixes) => Guard::ReviewNeedsFixes,
        (CodexReview, TeacherReview) => Guard::ReviewPassed,
        (CodexReview, ReadyToSubmit) => Guard::ReadyWithoutTeacher,
        (NeedsFixes, ClaudeSolving) => Guard::RevisionHandedOff,
        (TeacherReview, CodexReview) => Guard::Back,
        (TeacherReview, NeedsFixes) => Guard::Reopen,
        (TeacherReview, ReadyToSubmit) => Guard::Ready,
        (ReadyToSubmit, NeedsFixes) => Guard::Reopen,
        (ReadyToSubmit, TeacherReview) => Guard::Reopen,
        (ReadyToSubmit, Submitted) => Guard::SubmissionReceipt,
        _ => return None,
    })
}

/// Данные, по которым проверяются условия переходов.
pub struct TransitionContext<'a> {
    pub snapshot: &'a WorkflowSnapshot,
    pub readiness: &'a ReadinessDto,
    pub require_teacher_review: bool,
    /// Пользователь подтвердил, что комментариев преподавателя нет.
    pub no_teacher_comments: bool,
}

pub fn stage_label(stage: Stage) -> &'static str {
    match stage {
        Stage::MaterialsCollected => "Материалы собраны",
        Stage::CodexAnalysis => "Анализ материалов",
        Stage::PromptReady => "Промт готов",
        Stage::ClaudeSolving => "Решение",
        Stage::CodexReview => "Проверка версии",
        Stage::NeedsFixes => "Нужны исправления",
        Stage::TeacherReview => "Проверка преподавателем",
        Stage::ReadyToSubmit => "Готово к сдаче",
        Stage::Submitted => "Отправлено",
    }
}

fn blocked(message: impl Into<String>) -> AppError {
    BackendError::blocked(message)
}

/// Проверяет переход. Запрещённая пара — `conflict`, невыполненное условие — `blocked`.
pub fn check_transition(
    from: Stage,
    to: Stage,
    context: &TransitionContext<'_>,
) -> Result<(), AppError> {
    if from == Stage::Submitted {
        return Err(BackendError::immutable(
            "Работа отправлена — этап больше не меняется",
        ));
    }
    let Some(guard) = guard_for(from, to) else {
        return Err(BackendError::conflict(format!(
            "Переход «{}» → «{}» недопустим: обязательные этапы нельзя пропускать",
            stage_label(from),
            stage_label(to)
        )));
    };
    check_guard(guard, context)
}

fn check_guard(guard: Guard, context: &TransitionContext<'_>) -> Result<(), AppError> {
    let snapshot = context.snapshot;
    let current = snapshot.current_version();
    let current_review = current.and_then(|version| {
        snapshot.latest_completed(RunKind::ReviewVersion, Some(version.id.as_str()))
    });
    match guard {
        Guard::Back | Guard::Reopen => Ok(()),
        Guard::AnalysisBundle => snapshot
            .bundles
            .iter()
            .any(|bundle| bundle.purpose == RunKind::AnalyzeMaterials && bundle.complete && bundle.selection_id.is_some())
            .then_some(())
            .ok_or_else(|| blocked("Сначала создайте полный набор источников из подборки материалов")),
        Guard::AnalysisAndPrompt => {
            let analysis = snapshot.latest_completed(RunKind::AnalyzeMaterials, None);
            let prompt = snapshot.prompts.iter().any(|prompt| {
                prompt.kind == PromptKind::Initial && prompt.status == PromptStatus::Committed
            });
            if analysis.is_none() {
                Err(blocked("Сохраните результат анализа материалов"))
            } else if !prompt {
                Err(blocked("Зафиксируйте первичный промт для автора"))
            } else {
                Ok(())
            }
        }
        Guard::PromptHandedOff => snapshot
            .prompts
            .iter()
            .any(|prompt| prompt.status == PromptStatus::Committed && prompt.sent_at.is_some())
            .then_some(())
            .ok_or_else(|| blocked("Передайте зафиксированный промт автору")),
        Guard::CurrentVersion => current
            .map(|_| ())
            .ok_or_else(|| blocked("Добавьте версию решения")),
        Guard::ReviewNeedsFixes => match current_review.and_then(|run| run.assessment.as_ref()) {
            Some(assessment)
                if matches!(assessment.verdict, ReviewVerdict::NeedsFixes | ReviewVerdict::Blocked) =>
            {
                Ok(())
            }
            Some(_) => Err(blocked("Проверка текущей версии не требует исправлений")),
            None => Err(blocked("Сначала сохраните проверку текущей версии")),
        },
        Guard::ReviewPassed => match current_review.and_then(|run| run.assessment.as_ref()) {
            Some(assessment)
                if matches!(assessment.verdict, ReviewVerdict::Pass | ReviewVerdict::NeedsTeacherReview)
                    && assessment.critical_count == 0 =>
            {
                Ok(())
            }
            Some(_) => Err(blocked("Проверка текущей версии не пройдена")),
            None => Err(blocked("Сначала сохраните проверку текущей версии")),
        },
        Guard::ReadyWithoutTeacher => {
            if context.require_teacher_review {
                Err(blocked(
                    "В настройках включена обязательная проверка по комментариям преподавателя",
                ))
            } else {
                check_ready(context)
            }
        }
        Guard::Ready => check_ready(context),
        Guard::RevisionHandedOff => {
            let Some(version) = current else {
                return Err(blocked("Не выбрана проверяемая версия"));
            };
            snapshot
                .prompts
                .iter()
                .any(|prompt| {
                    matches!(prompt.kind, PromptKind::Revision | PromptKind::Teacher)
                        && prompt.status == PromptStatus::Committed
                        && prompt.sent_at.is_some()
                        && prompt.based_on_version_id.as_deref() == Some(version.id.as_str())
                })
                .then_some(())
                .ok_or_else(|| blocked("Передайте автору зафиксированный промт исправлений по текущей версии"))
        }
        Guard::SubmissionReceipt => Err(blocked(
            "Отметка «Отправлено» ставится только подтверждением отправки с неизменяемым снимком (этап 10)",
        )),
    }
}

fn check_ready(context: &TransitionContext<'_>) -> Result<(), AppError> {
    let blockers: Vec<&str> = context
        .readiness
        .checks
        .iter()
        .filter(|check| !check.ok)
        .map(|check| check.label.as_str())
        .collect();
    if !blockers.is_empty() {
        return Err(blocked(format!(
            "Не пройдены проверки готовности: {}",
            blockers.join("; ")
        )));
    }
    if context.require_teacher_review
        && !context
            .snapshot
            .teacher_comments
            .iter()
            .any(|comment| comment.official)
        && !context.no_teacher_comments
    {
        return Err(BackendError::new(
            crate::error::ErrorCode::ConfirmationRequired,
            "Официальных ответов преподавателя нет — подтвердите, что их и не было",
        ));
    }
    Ok(())
}

/// Варианты переходов из текущего этапа — для блокировки кнопок по backend.
pub fn transition_options(
    from: Stage,
    context: &TransitionContext<'_>,
) -> Vec<TransitionOptionDto> {
    ALL_STAGES
        .iter()
        .copied()
        .filter(|to| *to != from && guard_for(from, *to).is_some())
        .map(|to| {
            let mut probe = TransitionContext {
                snapshot: context.snapshot,
                readiness: context.readiness,
                require_teacher_review: context.require_teacher_review,
                no_teacher_comments: true,
            };
            if to != Stage::ReadyToSubmit {
                probe.no_teacher_comments = context.no_teacher_comments;
            }
            match check_transition(from, to, &probe) {
                Ok(()) => TransitionOptionDto {
                    to,
                    allowed: true,
                    reason: format!("Перейти к этапу «{}»", stage_label(to)),
                },
                Err(error) => TransitionOptionDto {
                    to,
                    allowed: false,
                    reason: error.message,
                },
            }
        })
        .collect()
}

/// Этапы, на которых разрешено запускать шаг указанного вида.
pub fn run_allowed_in(kind: RunKind, stage: Stage) -> bool {
    match kind {
        RunKind::AnalyzeMaterials => {
            matches!(stage, Stage::MaterialsCollected | Stage::CodexAnalysis)
        }
        RunKind::ReviewVersion => stage == Stage::CodexReview,
        RunKind::RevisionPrompt => matches!(stage, Stage::CodexReview | Stage::NeedsFixes),
        RunKind::ReviewTeacher => stage == Stage::TeacherReview,
    }
}

/// Есть ли незавершённый запуск — второй параллельный шаг запрещён.
pub fn has_open_run(snapshot: &WorkflowSnapshot) -> bool {
    snapshot.runs.iter().any(|run| run.status.is_open())
}
