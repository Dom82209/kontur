//! Тесты persistence и hydration этапа 8A. Только in-memory SQLite; файловая
//! система не используется — содержимое сверяет подставной `ContentVerifier`.

use std::{
    collections::HashMap,
    sync::{Arc, Barrier},
    thread,
};

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::{
    db::Database,
    domain::{HomeworkWorkflowStage, RequirementStatus, VersionAuthor},
    dto::{FileKind, FileRefDto, ProviderId},
    error::ErrorCode,
    extraction::dto::MaterialKind,
};

use super::{
    bundle::{prepare_snapshot, write_snapshot, ContentCheck, ContentVerifier, PlannedItem},
    dto::{
        CriterionId, CriterionStatus, EvidenceReferenceDto, FindingSeverity, PromptKind,
        PromptStatus, Provenance, RecommendedActionDto, RecommendedActionKind, ReviewCriterionDto,
        ReviewFindingDto, ReviewRunStatus, ReviewVerdict, RunKind, SourceKind, StoredReviewResult,
        StyleDivergenceDto, TeacherCommentSource, WorkflowRole,
    },
    projections, repository,
};

pub(crate) const SHA_TASK: &str =
    "1111111111111111111111111111111111111111111111111111111111111111";
pub(crate) const SHA_LECTURE: &str =
    "2222222222222222222222222222222222222222222222222222222222222222";
pub(crate) const SHA_INBOX: &str =
    "3333333333333333333333333333333333333333333333333333333333333333";

/// Подставная сверка содержимого: по умолчанию всё совпадает.
#[derive(Default)]
pub(crate) struct FakeVerifier {
    pub checks: HashMap<String, ContentCheck>,
}

impl ContentVerifier for FakeVerifier {
    fn check(&self, item: &PlannedItem) -> ContentCheck {
        self.checks
            .get(&item.material.id)
            .cloned()
            .unwrap_or(ContentCheck::Same)
    }
}

pub(crate) fn database() -> Database {
    let database = Database::open_in_memory().unwrap();
    database.migrate().unwrap();
    database
}

/// ДЗ с условием, лекцией, двумя черновиками и файлом «Входящих», с извлечённым
/// текстом и подтверждённой подборкой [task, lecture, inbox].
pub(crate) fn seeded() -> Database {
    let database = database();
    database
        .with_connection(|connection| {
            seed(connection);
            Ok(())
        })
        .unwrap();
    database
}

pub(crate) fn seed(connection: &Connection) {
    connection
        .execute_batch(&format!(
            r#"
            INSERT INTO subjects(id, name, short_name, folder_path)
              VALUES ('subject-1', 'Основы бизнес-аналитики', 'ОБА', 'Предметы/ОБА');
            INSERT INTO weeks(id, subject_id, number, starts_at, ends_at, folder_path)
              VALUES ('week-7', 'subject-1', 7, '2026-10-19T00:00:00Z', '2026-10-25T23:59:59Z', 'Предметы/ОБА/Неделя 07');
            INSERT INTO homeworks(id, week_id, title, deadline, stage, workflow_stage, folder_path)
              VALUES ('hw-1', 'week-7', 'Моделирование процесса', '2026-10-25T20:00:00Z', 'task', 'materials',
                      'Предметы/ОБА/Неделя 07/03 ДЗ');
            INSERT INTO files(id, week_id, homework_id, category, name, kind, size_bytes, modified_at, relative_path)
              VALUES
              ('task-file', NULL, 'hw-1', 'homework_task', 'Условие.pdf', 'pdf', 400, '2026-10-19T10:00:00Z',
               'Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/Условие.pdf'),
              ('lecture-file', 'week-7', NULL, 'lecture', 'Лекция 7.pdf', 'pdf', 900, '2026-10-19T09:00:00Z',
               'Предметы/ОБА/Неделя 07/01 Лекции/Лекция 7.pdf'),
              ('draft-1', NULL, 'hw-1', 'homework_draft', 'Решение_v1.docx', 'doc', 512, '2026-10-20T10:00:00Z',
               'Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_v1.docx'),
              ('draft-2', NULL, 'hw-1', 'homework_draft', 'Решение_v2.docx', 'doc', 640, '2026-10-21T10:00:00Z',
               'Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_v2.docx'),
              ('appendix', NULL, 'hw-1', 'homework_draft', 'Приложение.xlsx', 'sheet', 128, '2026-10-21T11:00:00Z',
               'Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Приложение.xlsx');
            INSERT INTO inbox_files(id, name, kind, size_bytes, added_at, origin, status, relative_path, sha256,
                                    classification_json)
              VALUES ('inbox-1', 'Пример.pdf', 'pdf', 42, '2026-10-21T08:00:00Z', 'drop', 'classified',
                      'Предметы/00 Входящие/Пример.pdf', '{inbox}',
                      '{{"subjectId":"subject-1","weekNumber":7,"category":"material","confidence":0.9,"reasons":["имя"],"source":"auto"}}');
            INSERT INTO extractions(id, sha256, extractor_version, options_key, format, status, metadata_json,
                                    structure_json, blocks_truncated, text, text_chars, truncated, warnings_json,
                                    ocr_used, source_size_bytes, duration_ms, created_at)
              VALUES
              ('ext-task', '{task}', 1, 'pdf', 'pdf', 'complete', '{{}}', '[]', 0, 'Постройте модель процесса', 25, 0,
               '[]', 0, 400, 3, '2026-10-21T09:00:00Z'),
              ('ext-lecture', '{lecture}', 1, 'pdf', 'pdf', 'partial', '{{}}', '[]', 0, 'BPMN: события и шлюзы', 21, 1,
               '[{{"code":"page_unreadable","message":"Стр. 4 не прочитана","location":"стр. 4"}}]', 0, 900, 5,
               '2026-10-21T09:00:00Z'),
              ('ext-inbox', '{inbox}', 1, 'pdf', 'pdf', 'complete', '{{}}', '[]', 0, 'Пример диаграммы', 16, 0,
               '[]', 0, 42, 2, '2026-10-21T09:00:00Z');
            INSERT INTO material_extractions(material_kind, material_id, sha256, extraction_id, analyzed_at)
              VALUES ('file', 'task-file', '{task}', 'ext-task', '2026-10-21T09:00:00Z'),
                     ('file', 'lecture-file', '{lecture}', 'ext-lecture', '2026-10-21T09:00:00Z'),
                     ('inbox', 'inbox-1', '{inbox}', 'ext-inbox', '2026-10-21T09:00:00Z');
            INSERT INTO corpus_selections(id, homework_id, created_at, updated_at)
              VALUES ('selection-1', 'hw-1', '2026-10-21T09:30:00Z', '2026-10-21T09:30:00Z');
            INSERT INTO corpus_selection_items(selection_id, material_kind, material_id, sha256, position, added_at)
              VALUES ('selection-1', 'file', 'task-file', '{task}', 0, '2026-10-21T09:30:00Z'),
                     ('selection-1', 'file', 'lecture-file', '{lecture}', 1, '2026-10-21T09:30:00Z'),
                     ('selection-1', 'inbox', 'inbox-1', '{inbox}', 2, '2026-10-21T09:30:00Z');
            "#,
            task = SHA_TASK,
            lecture = SHA_LECTURE,
            inbox = SHA_INBOX,
        ))
        .unwrap();
}

pub(crate) fn snapshot_bundle(
    database: &Database,
    purpose: RunKind,
    version_id: Option<&str>,
) -> String {
    let plan = prepare_snapshot(database, "hw-1", purpose, &FakeVerifier::default()).unwrap();
    database
        .with_transaction(|transaction| {
            write_snapshot(transaction, "hw-1", purpose, version_id, &plan)
        })
        .unwrap()
}

fn file(connection: &Connection, id: &str) -> FileRefDto {
    repository::homework_file(connection, "hw-1", id).unwrap()
}

pub(crate) fn add_version(database: &Database, id: &str, file_id: &str) -> i64 {
    database
        .with_transaction(|transaction| {
            let main = file(transaction, file_id);
            repository::insert_version(
                transaction,
                &repository::NewVersion {
                    id,
                    homework_id: "hw-1",
                    main: &main,
                    files: std::slice::from_ref(&main),
                    note: None,
                    author: VersionAuthor::User,
                    prompt_id: None,
                    source_bundle_id: None,
                },
            )
        })
        .unwrap()
}

pub(crate) fn stored_result(
    verdict: ReviewVerdict,
    critical: usize,
    style: Option<u8>,
) -> StoredReviewResult {
    StoredReviewResult {
        verdict,
        summary: "Проверено вручную".into(),
        criteria: vec![ReviewCriterionDto {
            id: CriterionId::Correctness,
            label: "Корректность".into(),
            status: CriterionStatus::Pass,
            note: String::new(),
        }],
        critical: (0..critical)
            .map(|index| ReviewFindingDto {
                id: format!("finding-{index}"),
                severity: FindingSeverity::Critical,
                criterion: CriterionId::Correctness,
                text: format!("Ошибка {index}"),
                location: Some("стр. 2".into()),
                evidence: vec![EvidenceReferenceDto {
                    source_id: "task-file".into(),
                    file_name: "Условие.pdf".into(),
                    locator: "стр. 1".into(),
                }],
                introduced_in: "v1".into(),
                fixed_in: None,
            })
            .collect(),
        remarks: Vec::new(),
        unsupported_claims: Vec::new(),
        style: style.map(|score| StyleDivergenceDto {
            score,
            threshold: 4,
            reasons: Vec::new(),
        }),
        teacher_requirements: Vec::new(),
        recommended_action: RecommendedActionDto {
            kind: RecommendedActionKind::MarkReady,
            label: "Отметить готовность".into(),
        },
        missing: Vec::new(),
    }
}

pub(crate) fn completed_review(
    database: &Database,
    run_id: &str,
    version_id: &str,
    bundle_id: &str,
    verdict: ReviewVerdict,
) {
    database
        .with_transaction(|transaction| {
            repository::insert_run(
                transaction,
                &repository::NewRun {
                    id: run_id,
                    homework_id: "hw-1",
                    kind: RunKind::ReviewVersion,
                    role: WorkflowRole::Reviewer,
                    provider: ProviderId::Codex,
                    version_id: Some(version_id),
                    source_bundle_id: bundle_id,
                    sources_used: &[],
                    status: ReviewRunStatus::ManualHandoff,
                    step: "Ожидает ручного результата",
                    style_threshold: 4,
                },
            )?;
            let result = stored_result(verdict, 0, Some(3));
            repository::complete_run_manually(
                transaction,
                &repository::ManualCompletion {
                    run_id,
                    result: &result,
                    correctness: CriterionStatus::Pass,
                    task_fit: CriterionStatus::Pass,
                    course_grounding: CriterionStatus::Pass,
                    teacher: CriterionStatus::Na,
                    style_score: Some(3),
                },
            )
        })
        .unwrap();
}

// ───────────────────────── Hydration ─────────────────────────

#[test]
fn full_hydration_of_existing_homework() {
    let database = seeded();
    let analysis_bundle = snapshot_bundle(&database, RunKind::AnalyzeMaterials, None);
    add_version(&database, "version-1", "draft-1");
    let review_bundle = snapshot_bundle(&database, RunKind::ReviewVersion, Some("version-1"));
    completed_review(
        &database,
        "run-1",
        "version-1",
        &review_bundle,
        ReviewVerdict::NeedsFixes,
    );
    database
        .with_transaction(|transaction| {
            repository::insert_draft_prompt(
                transaction,
                &repository::NewPrompt {
                    id: "prompt-1",
                    homework_id: "hw-1",
                    kind: PromptKind::Revision,
                    text: "Исправить ошибку 0",
                    created_by: VersionAuthor::User,
                    based_on_version_id: Some("version-1"),
                    review_run_id: Some("run-1"),
                    addresses: &["finding-0".to_owned()],
                    sources: &[],
                },
            )?;
            repository::commit_prompt(transaction, "prompt-1")?;
            repository::insert_teacher_comment(
                transaction,
                &repository::NewTeacherComment {
                    id: "comment-1",
                    homework_id: "hw-1",
                    source: TeacherCommentSource::Chat,
                    date: "2026-10-21T12:00:00Z",
                    question: "Нужна ли BPMN-нотация?",
                    answer: "Да, обязательно",
                },
            )?;
            repository::insert_requirement(
                transaction,
                &repository::NewRequirement {
                    id: "requirement-1",
                    homework_id: "hw-1",
                    comment_id: "comment-1",
                    text: "Использовать BPMN",
                    origin: "Да, обязательно",
                    status: RequirementStatus::NotDone,
                    note: "",
                    checked_version_id: "version-1",
                },
            )?;
            repository::set_current_version(transaction, "hw-1", "version-1")?;
            Ok(())
        })
        .unwrap();

    let snapshot = database
        .with_connection(|connection| repository::load_snapshot(connection, "hw-1"))
        .unwrap();
    assert_eq!(snapshot.homework.versions.len(), 1);
    assert_eq!(
        snapshot.homework.final_version_id.as_deref(),
        Some("version-1")
    );
    assert_eq!(snapshot.bundles.len(), 2);
    assert_eq!(snapshot.bundles[0].id, analysis_bundle);
    assert_eq!(snapshot.bundles[1].version_id.as_deref(), Some("version-1"));
    assert_eq!(snapshot.runs.len(), 1);
    let run = &snapshot.runs[0];
    assert_eq!(run.provenance, Provenance::Manual);
    assert_eq!(
        run.source_bundle_id.as_deref(),
        Some(review_bundle.as_str())
    );
    assert_eq!(
        run.result.as_ref().unwrap().prompt.as_ref().unwrap().id,
        "prompt-1"
    );
    assert_eq!(snapshot.prompts[0].status, PromptStatus::Committed);
    assert_eq!(
        snapshot.teacher_comments[0].source,
        TeacherCommentSource::Chat
    );
    assert_eq!(
        snapshot.teacher_requirements[0].status,
        RequirementStatus::NotDone
    );
    assert!(snapshot.submission_package.is_none());
    assert!(snapshot.submission_receipt.is_none());

    let iterations = projections::iterations(&snapshot);
    assert_eq!(iterations.len(), 1);
    assert_eq!(iterations[0].review.as_ref().unwrap().id, "run-1");
    assert_eq!(
        iterations[0].revision_prompt.as_ref().unwrap().id,
        "prompt-1"
    );

    let readiness = projections::readiness(&snapshot);
    assert!(!readiness.ready);
    assert_eq!(readiness.review_run_id.as_deref(), Some("run-1"));
}

#[test]
fn package_and_receipt_are_hydrated_with_typed_json() {
    let database = seeded();
    add_version(&database, "version-1", "draft-1");
    database
        .with_connection(|connection| {
            connection.execute(
                r#"INSERT INTO submission_packages(id, homework_id, final_version_id, status, package_path,
                       checklist_json, items_json, links_json, warnings_json, upload_list_json, built_at)
                   VALUES ('package-1', 'hw-1', 'version-1', 'ready', 'Предметы/ОБА/Неделя 07/03 ДЗ/03 На сдачу',
                       '[{"id":"final_selected","label":"Итоговая версия","done":true,"auto":true}]',
                       '[{"id":"item-1","role":"main","file":null,"finalName":"Гончарова_Доминика_Неделя7 ДЗ.docx","required":true,"present":false}]',
                       '[]', '["Нет ссылки на задание"]', '["Гончарова_Доминика_Неделя7 ДЗ.docx"]', '2026-10-22T10:00:00Z')"#,
                [],
            )?;
            connection.execute(
                "INSERT INTO submission_receipts(id, package_id, homework_id, submitted_version_id, snapshot_path,
                     snapshot_hash, confirmed_at, submitted_at, assignment_url, files_json)
                 VALUES ('receipt-1', 'package-1', 'hw-1', 'version-1', 'Предметы/ОБА/Неделя 07/03 ДЗ/04 Отправлено/v01',
                     'hash', '2026-10-22T11:00:00Z', '2026-10-22T10:59:00Z', 'https://my.centraluniversity.ru/a/1', '[]')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let snapshot = database
        .with_connection(|connection| repository::load_snapshot(connection, "hw-1"))
        .unwrap();
    let package = snapshot.submission_package.unwrap();
    assert_eq!(
        package.checklist[0].id,
        super::dto::PackageCheckId::FinalSelected
    );
    assert_eq!(package.items[0].role, super::dto::PackageItemRole::Main);
    let receipt = snapshot.submission_receipt.unwrap();
    assert_eq!(receipt.version_id, "version-1");
    assert_eq!(receipt.submitted_at, "2026-10-22T10:59:00Z");
}

// ───────────────────────── SourceBundle ─────────────────────────

#[test]
fn corpus_selection_becomes_immutable_source_bundle() {
    let database = seeded();
    let bundle_id = snapshot_bundle(&database, RunKind::AnalyzeMaterials, None);
    let (bundle, changes) = database
        .with_connection(|connection| repository::bundle(connection, &bundle_id))
        .unwrap();
    assert_eq!(bundle.selection_id.as_deref(), Some("selection-1"));
    assert_eq!(
        bundle.extractor_version,
        Some(crate::extraction::EXTRACTOR_VERSION)
    );
    assert!(bundle.complete, "условие задания есть в подборке");
    let ids: Vec<_> = bundle
        .items
        .iter()
        .map(|item| item.material.id.as_str())
        .collect();
    assert_eq!(ids, ["task-file", "lecture-file", "inbox-1"]);
    assert_eq!(bundle.items[0].kind, SourceKind::Task);
    assert!(bundle.items[0].required);
    assert_eq!(bundle.items[1].kind, SourceKind::Lecture);
    assert_eq!(bundle.items[2].material.kind, MaterialKind::Inbox);
    assert_eq!(bundle.items[0].sha256.as_deref(), Some(SHA_TASK));
    assert_eq!(
        bundle.items[1].extraction_id.as_deref(),
        Some("ext-lecture")
    );
    assert_eq!(bundle.items[1].warnings, ["page_unreadable"]);
    assert_eq!(bundle.total_chars, 25 + 21 + 16);
    let codes: Vec<_> = bundle
        .warnings
        .iter()
        .map(|item| item.code.as_str())
        .collect();
    assert_eq!(codes, ["partial_extraction", "text_truncated"]);
    assert_eq!(changes, [false, false, false]);

    database
        .with_connection(|connection| {
            assert!(connection
                .execute(
                    "UPDATE source_bundles SET complete = 0 WHERE id = ?1",
                    [&bundle_id]
                )
                .is_err());
            assert!(connection
                .execute("UPDATE source_bundle_items SET name = 'x'", [])
                .is_err());
            Ok(())
        })
        .unwrap();
}

#[test]
fn changing_selection_does_not_change_existing_bundle() {
    let database = seeded();
    let bundle_id = snapshot_bundle(&database, RunKind::AnalyzeMaterials, None);
    let before = database
        .with_connection(|connection| repository::bundle(connection, &bundle_id))
        .unwrap();
    database
        .with_connection(|connection| {
            connection.execute_batch(
                "DELETE FROM corpus_selection_items WHERE material_id = 'lecture-file';
                 UPDATE corpus_selections SET updated_at = '2026-10-22T10:00:00Z';",
            )?;
            Ok(())
        })
        .unwrap();
    let after = database
        .with_connection(|connection| repository::bundle(connection, &bundle_id))
        .unwrap();
    assert_eq!(before, after);
    let second = snapshot_bundle(&database, RunKind::AnalyzeMaterials, None);
    let (newer, _) = database
        .with_connection(|connection| repository::bundle(connection, &second))
        .unwrap();
    assert_eq!(newer.items.len(), 2);
    assert_eq!(after.0.items.len(), 3);
}

#[test]
fn changed_material_is_reported_on_old_snapshot() {
    let database = seeded();
    let bundle_id = snapshot_bundle(&database, RunKind::AnalyzeMaterials, None);
    database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE material_extractions SET sha256 = ?1 WHERE material_id = 'lecture-file'",
                ["4".repeat(64)],
            )?;
            Ok(())
        })
        .unwrap();
    let (_, changes) = database
        .with_connection(|connection| repository::bundle(connection, &bundle_id))
        .unwrap();
    assert_eq!(changes, [false, true, false]);
}

fn assert_blocked(database: &Database, verifier: &FakeVerifier) {
    let error =
        prepare_snapshot(database, "hw-1", RunKind::AnalyzeMaterials, verifier).unwrap_err();
    assert!(
        matches!(error.code, ErrorCode::Blocked | ErrorCode::Database),
        "unexpected {error:?}"
    );
    let bundles: i64 =
        database
            .with_connection(|connection| {
                Ok(connection
                    .query_row("SELECT count(*) FROM source_bundles", [], |row| row.get(0))?)
            })
            .unwrap();
    assert_eq!(bundles, 0);
}

#[test]
fn damaged_or_changed_sources_block_the_bundle() {
    // Файл на диске изменился после анализа.
    let database = seeded();
    let mut verifier = FakeVerifier::default();
    verifier
        .checks
        .insert("task-file".into(), ContentCheck::Changed);
    assert_blocked(&database, &verifier);
    // Файл пропал.
    verifier
        .checks
        .insert("task-file".into(), ContentCheck::Missing);
    assert_blocked(&database, &verifier);
    // Хранилище недоступно для проверки.
    verifier.checks.insert(
        "task-file".into(),
        ContentCheck::Unavailable("нет хранилища".into()),
    );
    assert_blocked(&database, &verifier);

    let clean = FakeVerifier::default();
    for damage in [
        // Текст извлечён из другой версии файла.
        format!("UPDATE material_extractions SET sha256 = '{}' WHERE material_id = 'task-file'", "5".repeat(64)),
        // Результат удалён из cache.
        "DELETE FROM extractions WHERE id = 'ext-task'".to_owned(),
        // Устаревшие правила извлечения.
        "UPDATE extractions SET extractor_version = 99 WHERE id = 'ext-task'".to_owned(),
        // Из файла не извлечён текст.
        "UPDATE extractions SET status = 'needs_ocr' WHERE id = 'ext-task'".to_owned(),
        // Файл «Входящих» изменился после выбора.
        format!("UPDATE inbox_files SET sha256 = '{}' WHERE id = 'inbox-1'", "6".repeat(64)),
        // Повреждённый JSON предупреждений извлечения.
        "PRAGMA ignore_check_constraints = ON; UPDATE extractions SET warnings_json = '[{' WHERE id = 'ext-task'".to_owned(),
        // Материал удалён: триггер этапа 7 снимает его из подборки — остаётся пустая/неполная.
        "DELETE FROM corpus_selection_items".to_owned(),
    ] {
        let database = seeded();
        database
            .with_connection(|connection| {
                connection.execute_batch(&damage)?;
                Ok(())
            })
            .unwrap();
        assert_blocked(&database, &clean);
    }
}

#[test]
fn snapshot_write_conflicts_when_selection_changed_after_verification() {
    let database = seeded();
    let plan = prepare_snapshot(
        &database,
        "hw-1",
        RunKind::AnalyzeMaterials,
        &FakeVerifier::default(),
    )
    .unwrap();
    database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE corpus_selections SET updated_at = '2026-10-22T12:00:00Z'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let error = database
        .with_transaction(|transaction| {
            write_snapshot(transaction, "hw-1", RunKind::AnalyzeMaterials, None, &plan)
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
}

#[test]
fn bundle_without_task_is_incomplete_and_bundle_of_other_homework_version_is_rejected() {
    let database = seeded();
    database
        .with_connection(|connection| {
            connection.execute(
                "DELETE FROM corpus_selection_items WHERE material_id = 'task-file'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let bundle_id = snapshot_bundle(&database, RunKind::AnalyzeMaterials, None);
    let (bundle, _) = database
        .with_connection(|connection| repository::bundle(connection, &bundle_id))
        .unwrap();
    assert!(!bundle.complete);
    assert!(bundle
        .missing
        .iter()
        .any(|item| item.kind == SourceKind::Task && item.required));

    let direct = database.with_connection(|connection| {
        connection.execute(
            "INSERT INTO source_bundles(id, homework_id, purpose, version_id, complete, origin, selection_id,
                                        extractor_version)
             VALUES ('legacy-like', 'hw-1', 'analyze_materials', NULL, 1, 'legacy', NULL, NULL)",
            [],
        )?;
        Ok(())
    });
    assert!(direct.is_err(), "новые наборы — только снимки подборки");
}

// ───────────────────────── Версии ─────────────────────────

#[test]
fn versions_are_append_only_with_backend_numbering() {
    let database = seeded();
    assert_eq!(add_version(&database, "version-1", "draft-1"), 1);
    assert_eq!(add_version(&database, "version-2", "draft-2"), 2);
    let versions = database
        .with_connection(|connection| repository::versions(connection, "hw-1"))
        .unwrap();
    assert_eq!(
        versions
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["v01", "v02"]
    );
    assert_eq!(versions[1].files[0].id, "draft-2");
    let duplicate = database.with_transaction(|transaction| {
        let main = file(transaction, "draft-1");
        repository::insert_version(
            transaction,
            &repository::NewVersion {
                id: "version-3",
                homework_id: "hw-1",
                main: &main,
                files: std::slice::from_ref(&main),
                note: None,
                author: VersionAuthor::User,
                prompt_id: None,
                source_bundle_id: None,
            },
        )
    });
    assert_eq!(duplicate.unwrap_err().code, ErrorCode::Conflict);
    let candidates = database
        .with_connection(|connection| repository::version_candidates(connection, "hw-1"))
        .unwrap();
    assert_eq!(
        candidates
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["appendix"]
    );
    database
        .with_connection(|connection| {
            assert!(connection
                .execute(
                    "UPDATE homework_versions SET note = 'x' WHERE id = 'version-1'",
                    []
                )
                .is_err());
            Ok(())
        })
        .unwrap();
}

#[test]
fn concurrent_version_creation_never_reuses_a_number() {
    let database = Arc::new(seeded());
    database
        .with_connection(|connection| {
            for index in 0..6 {
                connection.execute(
                    "INSERT INTO files(id, homework_id, category, name, kind, size_bytes, modified_at, relative_path)
                     VALUES (?1, 'hw-1', 'homework_draft', ?2, 'doc', 1, '2026-10-21T10:00:00Z', ?3)",
                    params![
                        format!("parallel-{index}"),
                        format!("p{index}.docx"),
                        format!("Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/p{index}.docx"),
                    ],
                )?;
            }
            Ok(())
        })
        .unwrap();
    let barrier = Arc::new(Barrier::new(6));
    let handles: Vec<_> = (0..6)
        .map(|index| {
            let database = Arc::clone(&database);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                add_version(
                    &database,
                    &format!("parallel-version-{index}"),
                    &format!("parallel-{index}"),
                )
            })
        })
        .collect();
    let mut numbers: Vec<i64> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    numbers.sort_unstable();
    assert_eq!(numbers, [1, 2, 3, 4, 5, 6]);
}

// ───────────────────────── ReviewRun и промты ─────────────────────────

#[test]
fn review_keeps_applied_threshold_and_links() {
    let database = seeded();
    add_version(&database, "version-1", "draft-1");
    let bundle = snapshot_bundle(&database, RunKind::ReviewVersion, Some("version-1"));
    completed_review(
        &database,
        "run-1",
        "version-1",
        &bundle,
        ReviewVerdict::Pass,
    );
    let run = database
        .with_connection(|connection| repository::run(connection, "run-1"))
        .unwrap();
    assert_eq!(run.style_threshold, 4);
    assert_eq!(run.version_id.as_deref(), Some("version-1"));
    assert_eq!(run.source_bundle_id.as_deref(), Some(bundle.as_str()));
    let assessment = run.assessment.unwrap();
    assert_eq!(assessment.verdict, ReviewVerdict::Pass);
    assert_eq!(assessment.style_score, Some(3));
    database
        .with_connection(|connection| {
            for statement in [
                "UPDATE review_runs SET style_threshold = 9 WHERE id = 'run-1'",
                "UPDATE review_runs SET verdict = 'needs_fixes' WHERE id = 'run-1'",
                "UPDATE review_runs SET version_id = NULL WHERE id = 'run-1'",
                "DELETE FROM review_runs WHERE id = 'run-1'",
            ] {
                assert!(connection.execute(statement, []).is_err(), "{statement}");
            }
            repository::pin_run(connection, "run-1")?;
            assert!(connection
                .execute(
                    "UPDATE review_runs SET saved_as_iteration = 0 WHERE id = 'run-1'",
                    []
                )
                .is_err());
            Ok(())
        })
        .unwrap();
    // Новые items в уже использованный набор не добавляются.
    let late_item = database.with_connection(|connection| {
        connection.execute(
            "INSERT INTO source_bundle_items(id, source_bundle_id, material_kind, material_id, position, name,
                 file_kind, size_bytes, modified_at, relative_path, sha256, extractor_version)
             VALUES ('late', ?1, 'file', 'draft-2', 9, 'x', 'doc', 1, 'now', 'Предметы/x.docx', ?2, 1)",
            params![bundle, SHA_TASK],
        )?;
        Ok(())
    });
    assert!(late_item.is_err());
}

#[test]
fn prompts_are_drafts_until_committed_then_immutable() {
    let database = seeded();
    let bundle = snapshot_bundle(&database, RunKind::AnalyzeMaterials, None);
    database
        .with_transaction(|transaction| {
            repository::insert_run(
                transaction,
                &repository::NewRun {
                    id: "analysis-1",
                    homework_id: "hw-1",
                    kind: RunKind::AnalyzeMaterials,
                    role: WorkflowRole::Analyst,
                    provider: ProviderId::Codex,
                    version_id: None,
                    source_bundle_id: &bundle,
                    sources_used: &[],
                    status: ReviewRunStatus::WaitingForProvider,
                    step: "Ожидает провайдера",
                    style_threshold: 4,
                },
            )?;
            let result = stored_result(ReviewVerdict::Pass, 0, None);
            repository::complete_run_manually(
                transaction,
                &repository::ManualCompletion {
                    run_id: "analysis-1",
                    result: &result,
                    correctness: CriterionStatus::Na,
                    task_fit: CriterionStatus::Na,
                    course_grounding: CriterionStatus::Na,
                    teacher: CriterionStatus::Na,
                    style_score: None,
                },
            )?;
            repository::insert_draft_prompt(
                transaction,
                &repository::NewPrompt {
                    id: "prompt-1",
                    homework_id: "hw-1",
                    kind: PromptKind::Initial,
                    text: "Черновик",
                    created_by: VersionAuthor::User,
                    based_on_version_id: None,
                    review_run_id: Some("analysis-1"),
                    addresses: &[],
                    sources: &[],
                },
            )?;
            repository::update_draft_text(transaction, "prompt-1", "Решите задачу по условию")?;
            Ok(())
        })
        .unwrap();
    // Второй черновик того же ДЗ запрещён.
    let second = database.with_transaction(|transaction| {
        repository::insert_draft_prompt(
            transaction,
            &repository::NewPrompt {
                id: "prompt-2",
                homework_id: "hw-1",
                kind: PromptKind::Initial,
                text: "Другой",
                created_by: VersionAuthor::User,
                based_on_version_id: None,
                review_run_id: None,
                addresses: &[],
                sources: &[],
            },
        )
    });
    assert_eq!(second.unwrap_err().code, ErrorCode::Conflict);
    database
        .with_transaction(|transaction| {
            repository::commit_prompt(transaction, "prompt-1")?;
            repository::mark_prompt_handed_off(transaction, "prompt-1")
        })
        .unwrap();
    let edit = database.with_transaction(|transaction| {
        repository::update_draft_text(transaction, "prompt-1", "Подмена")
    });
    assert_eq!(edit.unwrap_err().code, ErrorCode::Immutable);
    database
        .with_connection(|connection| {
            assert!(connection
                .execute(
                    "UPDATE revision_prompts SET text = 'x' WHERE id = 'prompt-1'",
                    []
                )
                .is_err());
            assert!(connection
                .execute(
                    "UPDATE revision_prompts SET sent_at = 'later' WHERE id = 'prompt-1'",
                    []
                )
                .is_err());
            assert!(connection
                .execute("DELETE FROM revision_prompts WHERE id = 'prompt-1'", [])
                .is_err());
            Ok(())
        })
        .unwrap();
    let prompt = database
        .with_connection(|connection| repository::prompt(connection, "prompt-1"))
        .unwrap();
    assert_eq!(prompt.text, "Решите задачу по условию");
    assert!(prompt.sent_at.is_some());
    assert_eq!(prompt.provenance, Provenance::Manual);
}

#[test]
fn revision_prompt_must_reference_completed_review_of_same_version() {
    let database = seeded();
    add_version(&database, "version-1", "draft-1");
    add_version(&database, "version-2", "draft-2");
    let bundle = snapshot_bundle(&database, RunKind::ReviewVersion, Some("version-1"));
    completed_review(
        &database,
        "run-1",
        "version-1",
        &bundle,
        ReviewVerdict::NeedsFixes,
    );
    let mismatch = database.with_transaction(|transaction| {
        repository::insert_draft_prompt(
            transaction,
            &repository::NewPrompt {
                id: "prompt-x",
                homework_id: "hw-1",
                kind: PromptKind::Revision,
                text: "x",
                created_by: VersionAuthor::User,
                based_on_version_id: Some("version-2"),
                review_run_id: Some("run-1"),
                addresses: &[],
                sources: &[],
            },
        )
    });
    assert!(mismatch.is_err());
}

// ───────────────────────── Ошибки данных ─────────────────────────

#[test]
fn corrupted_json_and_unknown_enum_are_database_errors_without_panic() {
    let database = seeded();
    add_version(&database, "version-1", "draft-1");
    let bundle = snapshot_bundle(&database, RunKind::ReviewVersion, Some("version-1"));
    completed_review(
        &database,
        "run-1",
        "version-1",
        &bundle,
        ReviewVerdict::Pass,
    );
    for (damage, field) in [
        (
            "DROP TRIGGER review_runs_terminal_immutable; DROP TRIGGER review_runs_json_relative_only_update; UPDATE review_runs SET result_json = '{' WHERE id = 'run-1'",
            "review_runs.result_json",
        ),
        (
            "DROP TRIGGER review_runs_terminal_immutable; UPDATE review_runs SET status = 'magic' WHERE id = 'run-1'",
            "review_runs.status",
        ),
        (
            "DROP TRIGGER source_bundle_items_no_update; UPDATE source_bundle_items SET warnings_json = 'nope'",
            "source_bundle_items.warnings_json",
        ),
        (
            "DROP TRIGGER source_bundles_no_update; UPDATE source_bundles SET missing_json = '[{\"kind\":1}]'",
            "source_bundles.missing_json",
        ),
        (
            "DROP TRIGGER source_bundle_items_no_update; UPDATE source_bundle_items SET relative_path = 'a/../../etc'",
            "source_bundle_items.relative_path",
        ),
        (
            "DROP TRIGGER review_runs_terminal_immutable; DROP TRIGGER review_runs_json_relative_only_update; UPDATE review_runs SET result_json = '{\"verdict\":\"pass\"}' WHERE id = 'run-1'",
            "review_runs.result_json",
        ),
    ] {
        let copy = seeded();
        add_version(&copy, "version-1", "draft-1");
        let bundle = snapshot_bundle(&copy, RunKind::ReviewVersion, Some("version-1"));
        completed_review(&copy, "run-1", "version-1", &bundle, ReviewVerdict::Pass);
        copy.with_connection(|connection| {
            connection.execute_batch(&format!("PRAGMA ignore_check_constraints = ON; {damage};"))?;
            Ok(())
        })
        .unwrap();
        let error = copy
            .with_connection(|connection| repository::load_snapshot(connection, "hw-1"))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Database, "{damage}");
        assert!(error.message.contains(field), "{damage}: {}", error.message);
    }
    drop(database);
}

#[test]
fn json_writes_reject_absolute_paths() {
    let value = json!({ "evidence": [{ "fileName": "/Users/me/secret.pdf" }] });
    assert!(repository::reject_absolute_strings(&value, "x").is_err());
    for bad in [
        "~/x",
        "file:///x",
        "C:\\x",
        "../x",
        "a/../../x",
        "\\\\server\\x",
    ] {
        assert!(
            repository::reject_absolute_strings(&json!(bad), "x").is_err(),
            "{bad}"
        );
    }
    assert!(repository::reject_absolute_strings(&json!("Предметы/ОБА/Лекция.pdf"), "x").is_ok());
    assert!(repository::reject_absolute_strings(&json!("стр. 3: 10/2"), "x").is_ok());
}

// ───────────────────────── Контракт DTO ─────────────────────────

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn dto_keys_are_exact_camel_case() {
    let database = seeded();
    add_version(&database, "version-1", "draft-1");
    let bundle = snapshot_bundle(&database, RunKind::ReviewVersion, Some("version-1"));
    completed_review(
        &database,
        "run-1",
        "version-1",
        &bundle,
        ReviewVerdict::Pass,
    );
    let snapshot = database
        .with_connection(|connection| repository::load_snapshot(connection, "hw-1"))
        .unwrap();
    let bundle = serde_json::to_value(&snapshot.bundles[0]).unwrap();
    assert_eq!(
        keys(&bundle),
        [
            "complete",
            "createdAt",
            "extractorVersion",
            "homeworkId",
            "id",
            "items",
            "limitChars",
            "missing",
            "origin",
            "purpose",
            "selectionId",
            "totalChars",
            "versionId",
            "warnings"
        ]
    );
    assert_eq!(
        keys(&bundle["items"][0]),
        [
            "extractionId",
            "extractorVersion",
            "file",
            "id",
            "included",
            "kind",
            "material",
            "required",
            "sha256",
            "textChars",
            "warnings"
        ]
    );
    let run = serde_json::to_value(&snapshot.runs[0]).unwrap();
    assert_eq!(
        keys(&run),
        [
            "assessment",
            "errorMessage",
            "finishedAt",
            "homeworkId",
            "id",
            "kind",
            "progress",
            "provenance",
            "provider",
            "result",
            "role",
            "savedAsIteration",
            "sourceBundleId",
            "sourcesUsed",
            "startedAt",
            "status",
            "step",
            "styleThreshold",
            "versionId"
        ]
    );
    assert_eq!(run["status"], "completed");
    assert_eq!(run["provenance"], "manual");
    assert_eq!(
        keys(&run["result"]),
        [
            "criteria",
            "critical",
            "missing",
            "prompt",
            "recommendedAction",
            "remarks",
            "style",
            "summary",
            "teacherRequirements",
            "unsupportedClaims",
            "verdict"
        ]
    );
    let homework = serde_json::to_value(&snapshot.homework).unwrap();
    assert_eq!(homework["revision"], 0);
    assert_eq!(homework["workflowStage"], "materials_collected");
    let serialized = serde_json::to_string(&(bundle, run, homework)).unwrap();
    assert!(!serialized.contains("/Users/"));
    assert!(!serialized.contains("file://"));
}

#[test]
fn every_workflow_enum_round_trips_its_database_value() {
    fn round_trip<T: Copy + PartialEq + std::fmt::Debug + serde::Serialize>(
        all: &[T],
        as_db: fn(T) -> &'static str,
        parse: fn(&str) -> Result<T, String>,
    ) {
        for value in all {
            assert_eq!(parse(as_db(*value)), Ok(*value));
            assert_eq!(serde_json::to_value(value).unwrap(), json!(as_db(*value)));
        }
        assert!(parse("legacy-unknown").is_err());
    }
    round_trip(
        ReviewRunStatus::ALL,
        ReviewRunStatus::as_db_str,
        ReviewRunStatus::try_from_db,
    );
    round_trip(RunKind::ALL, RunKind::as_db_str, RunKind::try_from_db);
    round_trip(
        SourceKind::ALL,
        SourceKind::as_db_str,
        SourceKind::try_from_db,
    );
    round_trip(
        PromptKind::ALL,
        PromptKind::as_db_str,
        PromptKind::try_from_db,
    );
    round_trip(
        PromptStatus::ALL,
        PromptStatus::as_db_str,
        PromptStatus::try_from_db,
    );
    round_trip(
        Provenance::ALL,
        Provenance::as_db_str,
        Provenance::try_from_db,
    );
    round_trip(
        CriterionStatus::ALL,
        CriterionStatus::as_db_str,
        CriterionStatus::try_from_db,
    );
    round_trip(
        ReviewVerdict::ALL,
        ReviewVerdict::as_db_str,
        ReviewVerdict::try_from_db,
    );
    round_trip(
        TeacherCommentSource::ALL,
        TeacherCommentSource::as_db_str,
        TeacherCommentSource::try_from_db,
    );
    assert_eq!(ReviewRunStatus::ALL.len(), 7);
}

// ───────────────────────── Главная ─────────────────────────

#[test]
fn dashboard_is_built_without_extraction_text_or_absolute_paths() {
    let database = seeded();
    let now = time::OffsetDateTime::parse(
        "2026-10-21T12:00:00Z",
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    database
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO app_settings(key, value_json) VALUES ('automation_rules', '{\"semesterStart\":\"2026-09-01T00:00:00Z\"}')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    // Этап 9: статусы провайдеров вычисляет AiService; без подключённого CLI —
    // ручная передача (CLI не ищется в системных каталогах теста).
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime = crate::ai::ProviderRuntime::for_tests(
        tmp.path().to_path_buf(),
        crate::ai::locate::CliLocator::with_dirs(Vec::new(), Vec::new(), None),
        None,
        std::time::Duration::from_secs(5),
    );
    let providers = crate::ai::AiService::new(&database, &runtime)
        .statuses()
        .unwrap();
    let dashboard = projections::dashboard(&database, now, providers).unwrap();
    assert_eq!(dashboard.academic_week.number, 8);
    assert_eq!(dashboard.active_homework.len(), 1);
    assert_eq!(dashboard.inbox.total, 1);
    assert_eq!(dashboard.providers.len(), 2);
    assert!(dashboard.providers.iter().all(|provider| provider.adapter
        == super::dto::ProviderAdapterDto::ManualHandoff
        && provider.availability == crate::ai::model::ProviderAvailability::Manual));
    let serialized = serde_json::to_string(&dashboard).unwrap();
    assert!(
        !serialized.contains("Постройте модель процесса"),
        "текст извлечения не грузится"
    );
    assert!(!serialized.contains("/Users/"));
    assert_eq!(
        dashboard.active_homework[0].workflow_stage,
        HomeworkWorkflowStage::MaterialsCollected
    );
    let _ = FileKind::Pdf;
}

// ───────────────────────── Миграция v5 → v6 ─────────────────────────

fn populated_v5() -> Connection {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    for (version, sql) in [
        (1, include_str!("../db/migrations/v1.sql")),
        (2, include_str!("../db/migrations/v2.sql")),
        (3, include_str!("../db/migrations/v3.sql")),
        (4, include_str!("../db/migrations/v4.sql")),
        (5, include_str!("../db/migrations/v5.sql")),
    ] {
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch(sql).unwrap();
        transaction
            .pragma_update(None, "user_version", version)
            .unwrap();
        transaction.commit().unwrap();
    }
    connection
        .execute_batch(
            r#"
            INSERT INTO subjects(id, name, short_name, folder_path) VALUES ('s1', 'ОБА', 'ОБА', 'Предметы/ОБА');
            INSERT INTO weeks(id, subject_id, number, starts_at, ends_at, folder_path)
              VALUES ('w1', 's1', 7, '2026-10-19T00:00:00Z', '2026-10-25T23:59:59Z', 'Предметы/ОБА/Неделя 07');
            INSERT INTO homeworks(id, week_id, title, deadline, stage, workflow_stage, folder_path)
              VALUES ('h1', 'w1', 'ДЗ', '2026-10-25T20:00:00Z', 'in_progress', 'review', 'Предметы/ОБА/Неделя 07/03 ДЗ');
            INSERT INTO files(id, homework_id, category, name, kind, size_bytes, modified_at, relative_path, content_hash)
              VALUES ('f1', 'h1', 'homework_task', 'task.pdf', 'pdf', 10, '2026-10-19T00:00:00Z',
                      'Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/task.pdf', NULL),
                     ('f2', 'h1', 'homework_task', 'data.csv', 'sheet', 20, '2026-10-19T00:00:00Z',
                      'Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/data.csv',
                      'abababababababababababababababababababababababababababababababab');
            INSERT INTO homework_versions(id, homework_id, number, label, file_path, author, file_name, source_file_id)
              VALUES ('v1', 'h1', 1, 'v01', 'Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md', 'user', 'v01.md', 'src');
            INSERT INTO source_bundles(id, homework_id, purpose, version_id, complete, missing_json, created_at)
              VALUES ('b1', 'h1', 'review_version', 'v1', 1, '[]', '2026-10-20 10:00:00');
            INSERT INTO source_bundle_items(id, source_bundle_id, file_id, source_kind, required, included, created_at)
              VALUES ('bi1', 'b1', 'f1', 'task', 1, 1, '2026-10-20 10:00:00'),
                     ('bi2', 'b1', 'f2', 'data', 0, 0, '2026-10-20 10:00:00');
            INSERT INTO review_runs(id, homework_id, kind, role, provider, version_id, source_bundle_id, status,
                                    progress, step, started_at, finished_at, style_score, style_threshold, created_at)
              VALUES ('r1', 'h1', 'review_version', 'reviewer', 'codex', 'v1', 'b1', 'completed', 100, 'готово',
                      '2026-10-20T10:01:00Z', '2026-10-20T10:02:00Z', 6, 5, '2026-10-20 10:01:00');
            INSERT INTO revision_prompts(id, homework_id, kind, text, created_by, based_on_version_id, review_run_id,
                                         sent_at, created_at, updated_at)
              VALUES ('p1', 'h1', 'revision', 'Исправить', 'codex', 'v1', 'r1', '2026-10-20T11:00:00Z',
                      '2026-10-20 10:03:00', '2026-10-20 10:03:00');
            INSERT INTO teacher_comments(id, homework_id, question, answer, source, date, added_at)
              VALUES ('c1', 'h1', 'Вопрос', 'Ответ', 'chat', '2026-10-20', '2026-10-20');
            INSERT INTO teacher_requirements(id, homework_id, comment_id, text, origin, status, note, checked_version_id)
              VALUES ('t1', 'h1', 'c1', 'Требование', '', 'done', '', 'v1');
            INSERT INTO operations(id, kind, status) VALUES ('o1', 'import', 'completed');
            INSERT INTO operation_results(operation_id, kind, result_json) VALUES ('o1', 'inbox_import', '{"items":[]}');
            "#,
        )
        .unwrap();
    connection
}

#[test]
fn populated_v5_migrates_to_v6_without_losing_rows() {
    let mut connection = populated_v5();
    let before: Vec<(String, i64)> = [
        "homeworks",
        "homework_versions",
        "source_bundles",
        "source_bundle_items",
        "review_runs",
        "revision_prompts",
        "teacher_comments",
        "teacher_requirements",
        "operations",
        "operation_results",
        "files",
    ]
    .iter()
    .map(|table| {
        (
            (*table).to_owned(),
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap(),
        )
    })
    .collect();
    assert_eq!(
        crate::db::migrations::migrate(&mut connection).unwrap(),
        crate::db::migrations::CURRENT_SCHEMA_VERSION
    );
    for (table, count) in &before {
        let after: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(after, *count, "rows lost in {table}");
    }
    let snapshot = repository::load_snapshot(&connection, "h1").unwrap();
    assert_eq!(snapshot.homework.revision, 0);
    let bundle = &snapshot.bundles[0];
    assert_eq!(bundle.origin, super::dto::BundleOrigin::Legacy);
    assert_eq!(bundle.selection_id, None);
    assert_eq!(bundle.items.len(), 2);
    assert_eq!(bundle.items[0].material.kind, MaterialKind::File);
    assert_eq!(bundle.items[0].file.name, "task.pdf");
    assert_eq!(
        bundle.items[0].sha256, None,
        "legacy без хэша не получает выдуманный SHA-256"
    );
    assert_eq!(
        bundle.items[1].sha256.as_deref(),
        Some("abababababababababababababababababababababababababababababababab")
    );
    assert!(
        !bundle.items[1].included,
        "исключённый legacy-источник сохраняет флаг"
    );
    let run = &snapshot.runs[0];
    assert_eq!(run.provenance, Provenance::Legacy);
    assert_eq!(run.status, ReviewRunStatus::Completed);
    assert_eq!(run.style_threshold, 5, "применённый порог сохранён");
    assert!(run.assessment.is_none(), "legacy-оценки не выдумываются");
    let prompt = &snapshot.prompts[0];
    assert_eq!(prompt.status, PromptStatus::Committed);
    assert_eq!(prompt.provenance, Provenance::Legacy);
    assert_eq!(prompt.sent_at.as_deref(), Some("2026-10-20T11:00:00Z"));
    assert_eq!(
        snapshot.teacher_requirements[0].status,
        RequirementStatus::Done
    );
    let replay: (String, Option<String>) = connection
        .query_row(
            "SELECT kind, request_hash FROM operation_results WHERE operation_id = 'o1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(replay, ("inbox_import".to_owned(), None));
    // Защитные триггеры старых схем продолжают действовать.
    assert!(connection
        .execute("UPDATE homework_versions SET label = 'x'", [])
        .is_err());
    assert!(connection
        .execute("UPDATE operation_results SET result_json = '{}'", [])
        .is_err());
    // Legacy-промт после миграции неизменяем.
    assert!(connection
        .execute("UPDATE revision_prompts SET text = 'x'", [])
        .is_err());
}

#[test]
fn submitted_homework_rejects_all_new_workflow_rows() {
    let database = seeded();
    add_version(&database, "version-1", "draft-1");
    let bundle = snapshot_bundle(&database, RunKind::ReviewVersion, Some("version-1"));
    completed_review(
        &database,
        "run-1",
        "version-1",
        &bundle,
        ReviewVerdict::Pass,
    );
    database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE homeworks SET stage = 'submitted', workflow_stage = 'submitted',
                        submitted_at = '2026-10-22T10:00:00Z' WHERE id = 'hw-1'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let attempts: Vec<Result<(), crate::error::AppError>> = vec![
        database.with_transaction(|transaction| {
            repository::bump_revision(transaction, "hw-1", 0).map(|_| ())
        }),
        database.with_transaction(|transaction| {
            repository::insert_teacher_comment(
                transaction,
                &repository::NewTeacherComment {
                    id: "late",
                    homework_id: "hw-1",
                    source: TeacherCommentSource::Email,
                    date: "2026-10-23",
                    question: "q",
                    answer: "a",
                },
            )
        }),
        database.with_transaction(|transaction| {
            let run = repository::run(transaction, "run-1")?;
            repository::insert_style_confirmation(transaction, "confirmation", &run, 3)
        }),
    ];
    for attempt in attempts {
        assert!(attempt.is_err());
    }
}
