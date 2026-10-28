//! Вычисляемые представления рабочего цикла: Readiness, IterationChain,
//! сравнение версий и Главная. Ничего из этого не хранится отдельными строками —
//! всё детерминированно строится из таблиц.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::{
    domain::{HomeworkStage, HomeworkWorkflowStage, RequirementStatus},
    dto::{AutomationRulesDto, HomeworkVersionDto},
    error::{AppError, BackendError},
    repositories::{hydrate_file, load_homeworks, ReadRepository},
};

use super::{
    dto::{
        AcademicWeekDto, AutomationState, CriterionStatus, DashboardAutomationDto, DashboardDto,
        DashboardInboxDto, DeadlineDto, DoneTotalDto, GroundingComparisonDto, HomeworkIterationDto,
        IterationStatus, ProviderStatusDto, ReadinessCheckDto, ReadinessCheckId, ReadinessDto,
        ReviewFindingDto, ReviewRunDto, ReviewRunStatus, ReviewVerdict, RunKind,
        SectionComparisonDto, SectionDiff, StyleComparisonDto, TeacherComparisonDto,
        VersionComparisonDto, WeekProgressDto, WeekProgressItemDto,
    },
    repository::WorkflowSnapshot,
};

// ───────────────────────── Readiness ─────────────────────────

fn check(
    id: ReadinessCheckId,
    label: &str,
    ok: bool,
    detail: impl Into<String>,
) -> ReadinessCheckDto {
    ReadinessCheckDto {
        id,
        label: label.to_owned(),
        ok,
        detail: detail.into(),
    }
}

/// Почему работа может или не может стать «Готово к сдаче». Каждое обязательное
/// условие — отдельная проверка; `ready` — только если пройдены все.
pub fn readiness(snapshot: &WorkflowSnapshot) -> ReadinessDto {
    let homework = &snapshot.homework;
    let mut checks = Vec::with_capacity(10);

    let material_bundle = snapshot
        .bundles
        .iter()
        .rev()
        .find(|bundle| bundle.complete && bundle.selection_id.is_some());
    checks.push(check(
        ReadinessCheckId::Materials,
        "Материалы собраны",
        material_bundle.is_some(),
        match material_bundle {
            Some(bundle) => format!("Набор источников: {} материалов", bundle.items.len()),
            None => "Нет полного набора источников из подборки материалов".to_owned(),
        },
    ));

    let current = snapshot.current_version();
    checks.push(check(
        ReadinessCheckId::FinalVersion,
        "Выбрана проверяемая версия",
        current.is_some(),
        match current {
            Some(version) => format!("Текущая версия — {}", version.label),
            None => "Версия решения не выбрана".to_owned(),
        },
    ));

    let review = current.and_then(|version| {
        snapshot.latest_completed(RunKind::ReviewVersion, Some(version.id.as_str()))
    });
    let latest_any_review = snapshot.latest_completed(RunKind::ReviewVersion, None);
    let review_current = review.is_some()
        && latest_any_review.map(|run| run.id.as_str()) == review.map(|run| run.id.as_str());
    checks.push(check(
        ReadinessCheckId::ReviewCurrent,
        "Проверка относится к текущей версии",
        review_current,
        match (review, current) {
            (Some(_), Some(version)) if review_current => format!("Проверена {}", version.label),
            (Some(_), Some(version)) => format!(
                "После проверки {} была проверена другая версия — выберите актуальную",
                version.label
            ),
            (None, Some(version)) => format!("{} ещё не проверена", version.label),
            _ => "Нет проверяемой версии".to_owned(),
        },
    ));

    let assessment = review.and_then(|run| run.assessment.as_ref());
    let verdict_ok = assessment.is_some_and(|value| {
        matches!(
            value.verdict,
            ReviewVerdict::Pass | ReviewVerdict::NeedsTeacherReview
        )
    });
    checks.push(check(
        ReadinessCheckId::ReviewPassed,
        "Проверка пройдена",
        verdict_ok,
        match assessment.map(|value| value.verdict) {
            Some(ReviewVerdict::Pass) => "Итоговое решение проверки: пройдено".to_owned(),
            Some(ReviewVerdict::NeedsTeacherReview) => {
                "Пройдено, нужна сверка с преподавателем".to_owned()
            }
            Some(ReviewVerdict::NeedsFixes) => "Проверка требует исправлений".to_owned(),
            Some(ReviewVerdict::Blocked) => "Проверка заблокирована".to_owned(),
            None => "Нет завершённой проверки".to_owned(),
        },
    ));

    let critical = assessment.map(|value| value.critical_count);
    checks.push(check(
        ReadinessCheckId::NoCritical,
        "Нет критических ошибок",
        critical == Some(0),
        match critical {
            Some(0) => "Критических ошибок нет".to_owned(),
            Some(count) => format!("Критических ошибок: {count}"),
            None => "Нет данных проверки".to_owned(),
        },
    ));

    let task_fit = assessment.map(|value| value.task_fit);
    checks.push(check(
        ReadinessCheckId::TaskCoverage,
        "Условия задания покрыты",
        task_fit == Some(CriterionStatus::Pass),
        match task_fit {
            Some(CriterionStatus::Pass) => "Все обязательные условия выполнены".to_owned(),
            Some(CriterionStatus::Manual) => "Покрытие условий требует ручной проверки".to_owned(),
            Some(_) => "Не все обязательные условия задания покрыты".to_owned(),
            None => "Нет данных проверки".to_owned(),
        },
    ));

    let open_requirements: Vec<_> = snapshot
        .teacher_requirements
        .iter()
        .filter(|item| {
            matches!(
                item.status,
                RequirementStatus::NotDone | RequirementStatus::Manual
            )
        })
        .collect();
    let not_done = open_requirements
        .iter()
        .filter(|item| item.status == RequirementStatus::NotDone)
        .count();
    checks.push(check(
        ReadinessCheckId::Teacher,
        "Требования преподавателя выполнены",
        open_requirements.is_empty(),
        if open_requirements.is_empty() {
            if snapshot.teacher_comments.is_empty() {
                "Комментариев преподавателя нет — при отметке готовности это нужно подтвердить"
                    .to_owned()
            } else {
                format!(
                    "Выполнено требований: {}",
                    snapshot.teacher_requirements.len()
                )
            }
        } else {
            format!(
                "Не выполнено: {not_done}, требует ручной проверки: {}",
                open_requirements.len() - not_done
            )
        },
    ));

    let missing = assessment.map(|value| value.missing_required);
    checks.push(check(
        ReadinessCheckId::Attachments,
        "Обязательные файлы и ссылки на месте",
        missing == Some(0),
        match missing {
            Some(0) => "Отсутствующих обязательных файлов и ссылок нет".to_owned(),
            Some(count) => format!("Не хватает обязательных файлов или ссылок: {count}"),
            None => "Нет данных проверки".to_owned(),
        },
    ));

    let style = review.map(|run| {
        let score = run.assessment.as_ref().and_then(|value| value.style_score);
        let confirmed = snapshot
            .style_confirmations
            .iter()
            .any(|item| item.review_run_id == run.id);
        (score, run.style_threshold, confirmed)
    });
    let style_ok = match style {
        Some((Some(score), threshold, confirmed)) => score <= threshold || confirmed,
        Some((None, _, _)) => true,
        None => false,
    };
    checks.push(check(
        ReadinessCheckId::Style,
        "Расхождение с личным стилем в пределах порога",
        style_ok,
        match style {
            Some((Some(score), threshold, _)) if score <= threshold => {
                format!("Расхождение {score}/10 при пороге {threshold}")
            }
            Some((Some(score), threshold, true)) => {
                format!(
                    "Расхождение {score}/10 выше порога {threshold}, подтверждено пользователем"
                )
            }
            Some((Some(score), threshold, false)) => format!(
                "Расхождение {score}/10 выше порога {threshold} — нужно явное подтверждение"
            ),
            Some((None, _, _)) => "Расхождение со стилем не оценивалось".to_owned(),
            None => "Нет данных проверки".to_owned(),
        },
    ));

    let bundle_changes = review
        .and_then(|run| run.source_bundle_id.as_deref())
        .and_then(|bundle_id| snapshot.bundle(bundle_id));
    let changed: Vec<&str> = bundle_changes
        .map(|(bundle, changes)| {
            bundle
                .items
                .iter()
                .zip(changes.iter())
                .filter(|(_, changed)| **changed)
                .map(|(item, _)| item.file.name.as_str())
                .collect()
        })
        .unwrap_or_default();
    let fresh = bundle_changes.is_some() && changed.is_empty();
    checks.push(check(
        ReadinessCheckId::SourcesFresh,
        "Материалы не менялись после проверки",
        fresh,
        if fresh {
            "Исходные материалы совпадают со снимком проверки".to_owned()
        } else if bundle_changes.is_none() {
            "Нет снимка материалов проверки".to_owned()
        } else {
            format!("Изменились после проверки: {}", changed.join(", "))
        },
    ));

    ReadinessDto {
        homework_id: homework.id.clone(),
        ready: checks.iter().all(|item| item.ok),
        checks,
        version_id: current.map(|version| version.id.clone()),
        review_run_id: review.map(|run| run.id.clone()),
        revision: homework.revision,
    }
}

// ───────────────────────── IterationChain ─────────────────────────

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn findings(run: Option<&ReviewRunDto>) -> Vec<ReviewFindingDto> {
    run.and_then(|run| run.result.as_ref())
        .map(|result| {
            result
                .critical
                .iter()
                .chain(result.remarks.iter())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn pinned_or_latest<'a>(
    snapshot: &'a WorkflowSnapshot,
    kind: RunKind,
    version_id: &str,
) -> Option<&'a ReviewRunDto> {
    let completed = || {
        snapshot.runs.iter().filter(move |run| {
            run.kind == kind
                && run.status == ReviewRunStatus::Completed
                && run.version_id.as_deref() == Some(version_id)
        })
    };
    completed()
        .find(|run| run.saved_as_iteration)
        .or_else(|| completed().next())
}

/// Цепочка итераций: версия → промт → проверка → промт исправлений → следующая версия.
pub fn iterations(snapshot: &WorkflowSnapshot) -> Vec<HomeworkIterationDto> {
    let homework = &snapshot.homework;
    let mut versions: Vec<&HomeworkVersionDto> = homework.versions.iter().collect();
    versions.sort_by_key(|version| version.number);
    let mut previous_findings: Vec<ReviewFindingDto> = Vec::new();
    let mut output = Vec::with_capacity(versions.len());
    for (index, version) in versions.iter().enumerate() {
        let review = pinned_or_latest(snapshot, RunKind::ReviewVersion, &version.id);
        let teacher_review = pinned_or_latest(snapshot, RunKind::ReviewTeacher, &version.id);
        let open_findings = findings(review);
        let current_texts: HashSet<String> = open_findings
            .iter()
            .map(|item| normalized(&item.text))
            .collect();
        let fixed_findings = if review.is_some() {
            previous_findings
                .iter()
                .filter(|item| !current_texts.contains(&normalized(&item.text)))
                .cloned()
                .map(|mut item| {
                    item.fixed_in = Some(version.id.clone());
                    item
                })
                .collect()
        } else {
            Vec::new()
        };
        let reviewing = snapshot.runs.iter().any(|run| {
            run.kind == RunKind::ReviewVersion
                && run.version_id.as_deref() == Some(version.id.as_str())
                && run.status.is_open()
        });
        let submitted = snapshot
            .submission_receipt
            .as_ref()
            .is_some_and(|receipt| receipt.version_id == version.id);
        let is_final = homework.final_version_id.as_deref() == Some(version.id.as_str())
            && matches!(
                homework.workflow_stage,
                HomeworkWorkflowStage::ReadyToSubmit | HomeworkWorkflowStage::Submitted
            );
        let verdict = review.and_then(|run| run.assessment.as_ref().map(|value| value.verdict));
        let status = if submitted {
            IterationStatus::Submitted
        } else if is_final {
            IterationStatus::Final
        } else if reviewing {
            IterationStatus::Reviewing
        } else {
            match verdict {
                Some(ReviewVerdict::Pass | ReviewVerdict::NeedsTeacherReview) => {
                    IterationStatus::Passed
                }
                Some(ReviewVerdict::NeedsFixes | ReviewVerdict::Blocked) => IterationStatus::Failed,
                None => IterationStatus::Draft,
            }
        };
        let prompt = version
            .prompt_id
            .as_deref()
            .and_then(|id| snapshot.prompts.iter().find(|prompt| prompt.id == id))
            .cloned();
        let revision_prompt = snapshot
            .prompts
            .iter()
            .find(|prompt| {
                prompt.kind == super::dto::PromptKind::Revision
                    && prompt.based_on_version_id.as_deref() == Some(version.id.as_str())
            })
            .cloned();
        if review.is_some() {
            previous_findings = open_findings.clone();
        }
        output.push(HomeworkIterationDto {
            version_id: version.id.clone(),
            label: version.label.clone(),
            author: version.author,
            created_at: version.created_at.clone(),
            files: version.files.clone(),
            prompt,
            source_bundle_id: review
                .and_then(|run| run.source_bundle_id.clone())
                .or_else(|| version.source_bundle_id.clone()),
            review: review.cloned(),
            teacher_review: teacher_review.cloned(),
            revision_prompt,
            next_version_id: versions.get(index + 1).map(|next| next.id.clone()),
            fixed_findings,
            open_findings,
            status,
        });
    }
    output
}

// ───────────────────────── Сравнение версий ─────────────────────────

pub fn compare_versions(
    snapshot: &WorkflowSnapshot,
    from_version_id: &str,
    to_version_id: &str,
) -> Result<VersionComparisonDto, AppError> {
    let find = |id: &str| {
        snapshot
            .homework
            .versions
            .iter()
            .find(|version| version.id == id)
            .ok_or_else(|| BackendError::not_found("Версия не найдена"))
    };
    let from = find(from_version_id)?;
    let to = find(to_version_id)?;
    let from_review = pinned_or_latest(snapshot, RunKind::ReviewVersion, &from.id);
    let to_review = pinned_or_latest(snapshot, RunKind::ReviewVersion, &to.id);
    let from_findings = findings(from_review);
    let to_findings = findings(to_review);
    let from_texts: HashSet<String> = from_findings
        .iter()
        .map(|item| normalized(&item.text))
        .collect();
    let to_texts: HashSet<String> = to_findings
        .iter()
        .map(|item| normalized(&item.text))
        .collect();
    let mut sections = Vec::new();
    for section in &to.sections {
        let diff = match from.sections.iter().find(|item| item.id == section.id) {
            Some(previous) if previous.hash == section.hash => SectionDiff::Same,
            Some(_) => SectionDiff::Changed,
            None => SectionDiff::Added,
        };
        sections.push(SectionComparisonDto {
            title: section.title.clone(),
            diff,
        });
    }
    for section in &from.sections {
        if !to.sections.iter().any(|item| item.id == section.id) {
            sections.push(SectionComparisonDto {
                title: section.title.clone(),
                diff: SectionDiff::Removed,
            });
        }
    }
    let teacher_for = |version_id: &str| {
        let related: Vec<_> = snapshot
            .teacher_requirements
            .iter()
            .filter(|item| item.checked_version_id.as_deref() == Some(version_id))
            .collect();
        (!related.is_empty()).then(|| DoneTotalDto {
            done: related
                .iter()
                .filter(|item| item.status == RequirementStatus::Done)
                .count() as u32,
            total: related.len() as u32,
        })
    };
    let assessment = |run: Option<&ReviewRunDto>| run.and_then(|run| run.assessment.clone());
    let from_assessment = assessment(from_review);
    let to_assessment = assessment(to_review);
    Ok(VersionComparisonDto {
        from_version_id: from.id.clone(),
        to_version_id: to.id.clone(),
        fixed: from_findings
            .iter()
            .filter(|item| !to_texts.contains(&normalized(&item.text)))
            .cloned()
            .collect(),
        introduced: to_findings
            .iter()
            .filter(|item| !from_texts.contains(&normalized(&item.text)))
            .cloned()
            .collect(),
        sections,
        style: StyleComparisonDto {
            from: from_assessment.as_ref().and_then(|value| value.style_score),
            to: to_assessment.as_ref().and_then(|value| value.style_score),
        },
        grounding: GroundingComparisonDto {
            from: from_assessment.as_ref().map(|value| value.course_grounding),
            to: to_assessment.as_ref().map(|value| value.course_grounding),
            unsupported_from: from_assessment
                .as_ref()
                .map_or(0, |value| value.unsupported_count),
            unsupported_to: to_assessment
                .as_ref()
                .map_or(0, |value| value.unsupported_count),
        },
        teacher: TeacherComparisonDto {
            from: teacher_for(&from.id),
            to: teacher_for(&to.id),
        },
    })
}

// ───────────────────────── Главная ─────────────────────────

const WEEK_SECONDS: i64 = 7 * 24 * 60 * 60;

fn format_time(value: time::OffsetDateTime) -> Result<String, AppError> {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| BackendError::database(format!("cannot format time: {error}")))
}

/// Учебная неделя относительно начала семестра из правил автоматизации.
pub fn academic_week(
    rules: &AutomationRulesDto,
    now: time::OffsetDateTime,
) -> Result<AcademicWeekDto, AppError> {
    let start = time::OffsetDateTime::parse(
        &rules.semester_start,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|error| BackendError::database(format!("invalid semesterStart: {error}")))?;
    let elapsed = (now - start).whole_seconds();
    let index = if elapsed < 0 {
        0
    } else {
        elapsed / WEEK_SECONDS
    };
    let starts = start + time::Duration::seconds(index * WEEK_SECONDS);
    let ends = starts + time::Duration::seconds(WEEK_SECONDS - 1);
    Ok(AcademicWeekDto {
        number: index + 1,
        starts_at: format_time(starts)?,
        ends_at: format_time(ends)?,
    })
}

/// Главная: фиксированный набор запросов, без извлечённого текста.
pub fn dashboard(
    database: &crate::db::Database,
    now: time::OffsetDateTime,
    providers: Vec<ProviderStatusDto>,
) -> Result<DashboardDto, AppError> {
    let today = format_time(now)?;
    let repository = ReadRepository::new(database);
    let rules = repository.automation_rules()?;
    let inbox = repository.inbox_files()?;
    let homework = database.with_connection(|connection| load_homeworks(connection, None))?;
    let (recent_files, watcher_enabled, last_run) = database.with_connection(|connection| {
        Ok((
            recent_files(connection)?,
            watcher_enabled(connection)?,
            last_operation(connection)?,
        ))
    })?;
    let week = academic_week(&rules, now)?;
    let pending_inbox: Vec<_> = inbox
        .iter()
        .filter(|file| file.status != crate::dto::InboxStatus::Routed)
        .collect();
    let ambiguous = pending_inbox
        .iter()
        .filter(|file| file.status == crate::dto::InboxStatus::Ambiguous)
        .count() as u32;
    let active: Vec<_> = homework
        .into_iter()
        .filter(|item| item.stage != HomeworkStage::Submitted)
        .collect();
    // ДЗ с неизвестным сроком не попадают в ленту сроков: срок не выдумывается.
    let mut deadlines: Vec<DeadlineDto> = active
        .iter()
        .filter_map(|item| {
            Some(DeadlineDto {
                homework_id: item.id.clone(),
                subject_id: item.subject_id.clone(),
                title: item.title.clone(),
                deadline: item.deadline.clone()?,
                stage: item.stage,
            })
        })
        .collect();
    deadlines.sort_by(|left, right| {
        left.deadline
            .cmp(&right.deadline)
            .then(left.homework_id.cmp(&right.homework_id))
    });
    let in_week: Vec<_> = active
        .iter()
        .filter(|item| {
            item.deadline.as_deref().is_some_and(|deadline| {
                deadline >= week.starts_at.as_str() && deadline <= week.ends_at.as_str()
            })
        })
        .collect();
    let items: Vec<WeekProgressItemDto> = in_week
        .iter()
        .map(|item| WeekProgressItemDto {
            label: item.title.clone(),
            done: matches!(item.stage, HomeworkStage::Ready | HomeworkStage::Submitted),
        })
        .collect();
    Ok(DashboardDto {
        today: today.clone(),
        academic_week: week,
        inbox: DashboardInboxDto {
            total: pending_inbox.len() as u32,
            ambiguous,
            preview: pending_inbox
                .iter()
                .take(5)
                .map(|file| (*file).clone())
                .collect(),
        },
        week_progress: WeekProgressDto {
            done: items.iter().filter(|item| item.done).count() as u32,
            total: items.len() as u32,
            items,
        },
        active_homework: active,
        deadlines,
        recent_files,
        automation: DashboardAutomationDto {
            state: if watcher_enabled {
                AutomationState::Active
            } else {
                AutomationState::Paused
            },
            watched_path: "Загрузки".to_owned(),
            last_run_at: last_run.unwrap_or_else(|| today.clone()),
            pending: pending_inbox.len() as u32,
        },
        providers,
    })
}

fn recent_files(connection: &Connection) -> Result<Vec<crate::dto::FileRefDto>, AppError> {
    let mut statement = connection.prepare(
        "SELECT id, name, kind, size_bytes, coalesce(modified_at, updated_at, created_at), relative_path
         FROM files WHERE status = 'available'
         ORDER BY coalesce(modified_at, updated_at, created_at) DESC, id
         LIMIT 8",
    )?;
    let mut rows = statement.query([])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        output.push(hydrate_file(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        )?);
    }
    Ok(output)
}

fn watcher_enabled(connection: &Connection) -> Result<bool, AppError> {
    let raw: Option<String> = rusqlite::OptionalExtension::optional(connection.query_row(
        "SELECT value_json FROM app_settings WHERE key = 'downloads_watcher'",
        [],
        |row| row.get(0),
    ))?;
    let Some(raw) = raw else {
        return Ok(false);
    };
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        BackendError::database(format!("invalid JSON in downloads_watcher: {error}"))
    })?;
    value
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| BackendError::database("downloads_watcher.enabled is missing"))
}

fn last_operation(connection: &Connection) -> Result<Option<String>, AppError> {
    Ok(rusqlite::OptionalExtension::optional(
        connection.query_row(
            "SELECT started_at FROM operations ORDER BY started_at DESC, id DESC LIMIT 1",
            [],
            |row| row.get(0),
        ),
    )?)
}
