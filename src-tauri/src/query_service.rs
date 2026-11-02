use crate::{
    db::Database,
    domain::BackendHealth,
    dto::{AutomationRulesDto, HomeworkDto, InboxFileDto, OperationDto, SubjectDto, WeekDto},
    error::AppError,
    repositories::ReadRepository,
};

pub struct QueryService<'a> {
    database: &'a Database,
}

impl<'a> QueryService<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn health(&self) -> Result<BackendHealth, AppError> {
        self.database.ping()?;
        Ok(BackendHealth {
            ready: true,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            schema_version: self.repository().schema_version()?,
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            database_exists: self.database.exists(),
        })
    }

    pub fn subjects(&self) -> Result<Vec<SubjectDto>, AppError> {
        self.repository().subjects()
    }

    pub fn weeks(&self, subject_id: &str) -> Result<Vec<WeekDto>, AppError> {
        self.repository().weeks(Some(subject_id))
    }

    pub fn week(&self, subject_id: &str, week_number: i64) -> Result<WeekDto, AppError> {
        self.repository().week(subject_id, week_number)
    }

    pub fn homework_list(&self) -> Result<Vec<HomeworkDto>, AppError> {
        self.repository().homework_list()
    }

    pub fn homework(&self, homework_id: &str) -> Result<HomeworkDto, AppError> {
        self.repository().homework(homework_id)
    }

    pub fn inbox_files(&self) -> Result<Vec<InboxFileDto>, AppError> {
        self.repository().inbox_files()
    }

    pub fn automation_rules(&self) -> Result<AutomationRulesDto, AppError> {
        self.repository().automation_rules()
    }

    pub fn activity_log(&self) -> Result<Vec<OperationDto>, AppError> {
        self.repository().activity_log()
    }

    fn repository(&self) -> ReadRepository<'_> {
        ReadRepository::new(self.database)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::{
        db::{migrations::CURRENT_SCHEMA_VERSION, Database},
        domain::{HomeworkStage, HomeworkWorkflowStage},
        error::ErrorCode,
    };

    use super::QueryService;

    fn migrated_database() -> Database {
        let database = Database::open_in_memory().expect("open database");
        assert_eq!(
            database.migrate().expect("migrate database"),
            CURRENT_SCHEMA_VERSION
        );
        database
    }

    fn fixture_database() -> Database {
        let database = migrated_database();
        seed_fixture(&database);
        database
    }

    fn seed_fixture(database: &Database) {
        database
            .with_connection(|connection| {
                connection.execute_batch(
                    r#"
                    INSERT INTO subjects(id, name, short_name, folder_path)
                    VALUES ('subject-oba', 'Основы бизнес-аналитики', 'ОБА', 'Предметы/ОБА');

                    INSERT INTO weeks(id, subject_id, number, starts_at, ends_at, topic, folder_path)
                    VALUES
                      ('week-6', 'subject-oba', 6, '2026-10-05T00:00:00Z', '2026-10-11T23:59:59Z', NULL, 'Предметы/ОБА/Неделя 06'),
                      ('week-7', 'subject-oba', 7, '2026-10-12T00:00:00Z', '2026-10-18T23:59:59Z', 'BPMN', 'Предметы/ОБА/Неделя 07');

                    INSERT INTO homeworks(
                      id, week_id, title, deadline, stage, workflow_stage, folder_path, assignment_url
                    ) VALUES (
                      'homework-7', 'week-7', 'Моделирование процесса', '2026-10-20T18:00:00Z',
                      'in_progress', 'review', 'Предметы/ОБА/Неделя 07/03 ДЗ', 'https://lms.example/assignments/7'
                    );

                    INSERT INTO files(id, week_id, homework_id, category, name, kind, size_bytes, modified_at, relative_path)
                    VALUES
                      ('lecture-7', 'week-7', NULL, 'lecture', 'Лекция 7.pdf', 'pdf', 100, '2026-10-12T10:00:00Z', 'Предметы/ОБА/Неделя 07/01 Лекции/Лекция 7.pdf'),
                      ('seminar-7', 'week-7', NULL, 'seminar', 'Семинар 7.pdf', 'pdf', 200, '2026-10-13T10:00:00Z', 'Предметы/ОБА/Неделя 07/02 Семинары/Семинар 7.pdf'),
                      ('material-7', 'week-7', NULL, 'material', 'Пример.bpmn', 'other', 300, '2026-10-14T10:00:00Z', 'Предметы/ОБА/Неделя 07/04 Дополнительно/Пример.bpmn'),
                      ('task-7', NULL, 'homework-7', 'homework_task', 'Условие.pdf', 'pdf', 400, '2026-10-15T10:00:00Z', 'Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/Условие.pdf');

                    INSERT INTO homework_versions(
                      id, homework_id, number, label, file_path, size_bytes, author, metadata_json,
                      file_name, source_file_id, note, sections_json, prompt_id, source_bundle_id, files_json
                    ) VALUES (
                      'version-1', 'homework-7', 1, 'v01',
                      'Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_v01.docx', 512, 'claude', '{}',
                      'Решение_v01.docx', 'draft-source-1', 'Первая версия',
                      '[{"id":"intro","title":"Введение","hash":"abc"}]', NULL, NULL,
                      '[{"id":"version-file-1","name":"Решение_v01.docx","kind":"doc","sizeBytes":512,"modifiedAt":"2026-10-16T10:00:00Z","relativePath":"Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_v01.docx"}]'
                    );

                    INSERT INTO inbox_files(
                      id, name, kind, size_bytes, added_at, origin, status, relative_path,
                      classification_json, duplicate_json
                    ) VALUES (
                      'inbox-1', 'новый.pdf', 'pdf', 42, '2026-10-18T10:00:00Z', 'drop', 'classified',
                      'Предметы/00 Входящие/новый.pdf',
                      '{"subjectId":"subject-oba","weekNumber":7,"category":"homework_data","confidence":0.9,"reasons":["имя файла"],"source":"auto"}',
                      NULL
                    );

                    INSERT INTO operations(
                      id, kind, status, progress, undoable, started_at, completed_at,
                      title, detail, paths_json, backup_path, homework_id
                    ) VALUES
                      ('operation-a', 'route', 'completed', 100, 1, '2026-10-18T12:00:00Z', '2026-10-18T12:01:00Z', 'Маршрутизация A', NULL, '["Предметы/ОБА/Неделя 07"]', 'Резервные копии/op-a', 'homework-7'),
                      ('operation-b', 'route', 'completed', 100, 0, '2026-10-18T12:00:00Z', '2026-10-18T12:02:00Z', 'Маршрутизация B', 'готово', '[]', NULL, NULL);

                    INSERT INTO app_settings(key, value_json) VALUES (
                      'automation_rules',
                      '{"studentName":{"lastName":"Гончарова","firstName":"Доминика"},"workflow":{"roles":{"analyst":"claude","author":"codex","reviewer":"claude"},"styleThreshold":6}}'
                    );
                    "#,
                )?;
                Ok(())
            })
            .expect("seed fixture");
    }

    #[test]
    fn serializes_dtos_with_exact_camel_case_keys() {
        let database = fixture_database();
        let subject = QueryService::new(&database).subjects().unwrap().remove(0);
        let value = serde_json::to_value(subject).unwrap();
        assert_eq!(
            value,
            json!({
                "id": "subject-oba",
                "name": "Основы бизнес-аналитики",
                "shortName": "ОБА",
                "currentWeek": 7,
                "weekCount": 2,
                "courseUrl": null,
                "folderPath": "Предметы/ОБА",
                "keywords": [],
                "archived": false,
                "mergedIntoId": null,
                "revision": 0
            })
        );
    }

    #[test]
    fn serializes_all_homework_stages() {
        let values = [
            HomeworkStage::Task,
            HomeworkStage::InProgress,
            HomeworkStage::Ready,
            HomeworkStage::Submitted,
        ]
        .map(|stage| serde_json::to_value(stage).unwrap());
        assert_eq!(
            values,
            [
                json!("task"),
                json!("in_progress"),
                json!("ready"),
                json!("submitted")
            ]
        );
    }

    #[test]
    fn serializes_all_nine_workflow_stages() {
        let values = [
            HomeworkWorkflowStage::MaterialsCollected,
            HomeworkWorkflowStage::CodexAnalysis,
            HomeworkWorkflowStage::PromptReady,
            HomeworkWorkflowStage::ClaudeSolving,
            HomeworkWorkflowStage::CodexReview,
            HomeworkWorkflowStage::NeedsFixes,
            HomeworkWorkflowStage::TeacherReview,
            HomeworkWorkflowStage::ReadyToSubmit,
            HomeworkWorkflowStage::Submitted,
        ]
        .map(|stage| serde_json::to_value(stage).unwrap());
        assert_eq!(
            values,
            [
                json!("materials_collected"),
                json!("codex_analysis"),
                json!("prompt_ready"),
                json!("claude_solving"),
                json!("codex_review"),
                json!("needs_fixes"),
                json!("teacher_review"),
                json!("ready_to_submit"),
                json!("submitted"),
            ]
        );
    }

    #[test]
    fn hydrates_complete_subject() {
        let database = fixture_database();
        let subjects = QueryService::new(&database).subjects().unwrap();
        assert_eq!(subjects.len(), 1);
        assert_eq!(subjects[0].short_name, "ОБА");
        assert_eq!(subjects[0].current_week, 7);
        assert_eq!(subjects[0].week_count, 2);
    }

    #[test]
    fn hydrates_complete_week_with_files_and_homework() {
        let database = fixture_database();
        let week = QueryService::new(&database).week("subject-oba", 7).unwrap();
        assert_eq!(week.topic.as_deref(), Some("BPMN"));
        assert_eq!(week.lectures[0].name, "Лекция 7.pdf");
        assert_eq!(week.seminars[0].name, "Семинар 7.pdf");
        assert_eq!(
            week.materials[0].relative_path,
            "Предметы/ОБА/Неделя 07/04 Дополнительно/Пример.bpmn"
        );
        assert_eq!(week.homework_ids, vec!["homework-7".to_owned()]);
    }

    #[test]
    fn hydrates_complete_homework_with_task_files_and_versions() {
        let database = fixture_database();
        let homework = QueryService::new(&database).homework("homework-7").unwrap();
        assert_eq!(homework.subject_id, "subject-oba");
        assert_eq!(homework.week_number, 7);
        assert_eq!(homework.task_files[0].id, "task-7");
        assert_eq!(homework.versions[0].file_name, "Решение_v01.docx");
        assert_eq!(homework.versions[0].sections[0].hash, "abc");
        assert_eq!(homework.versions[0].files[0].id, "version-file-1");
        assert!(homework.versions[0].frozen);
    }

    #[test]
    fn inbox_dto_never_serializes_persistent_source_path() {
        let database = fixture_database();
        let inbox = QueryService::new(&database).inbox_files().unwrap();
        let value = serde_json::to_value(&inbox[0]).unwrap();
        assert_eq!(value["name"], "новый.pdf");
        assert!(value.get("sourcePath").is_none());
        assert!(value.get("relativePath").is_none());
        assert_eq!(value["classification"]["weekNumber"], 7);
    }

    #[test]
    fn routed_files_are_not_returned_as_waiting_in_inbox() {
        let database = fixture_database();
        database
            .with_connection(|connection| {
                connection.execute(
                    "UPDATE inbox_files SET status = 'routed' WHERE id = 'inbox-1'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        assert!(QueryService::new(&database)
            .inbox_files()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn reads_saved_automation_rules_and_merges_safe_defaults() {
        let database = fixture_database();
        let rules = QueryService::new(&database).automation_rules().unwrap();
        assert_eq!(rules.student_name.last_name, "Гончарова");
        assert_eq!(rules.workflow.style_threshold, 6);
        assert_eq!(
            serde_json::to_value(rules.workflow.roles.author).unwrap(),
            "codex"
        );
        assert!(rules.confirm.mark_submitted);
        assert!(!rules.confirm.allow_overwrite);
    }

    #[test]
    fn uses_safe_automation_defaults_when_settings_are_absent() {
        let database = migrated_database();
        let rules = QueryService::new(&database).automation_rules().unwrap();
        assert_eq!(rules.workflow.style_threshold, 4);
        assert_eq!(rules.inbox_path, "Предметы/00 Входящие");
        assert!(rules.confirm.low_confidence_route && rules.confirm.mark_final);
    }

    #[test]
    fn legacy_rules_with_retired_fields_still_load() {
        let database = migrated_database();
        database
            .with_connection(|c| {
                c.execute(
                    "INSERT INTO app_settings(key, value_json) VALUES ('automation_rules', ?1)",
                    [r#"{"inboxPath":"Куда-то","downloadsWatchPath":"~/Downloads","obsidianVaultPath":".","versionPattern":"v{Версия}","weekDetection":"calendar","semesterStart":"1970-01-05T00:00:00Z","backupBeforeChange":false,"autoRouteHighConfidence":true,"lowConfidenceThreshold":0.8,"finalNamePattern":"{Неделя}","confirm":{"lowConfidenceRoute":false,"markFinal":false,"markSubmitted":true,"allowOverwrite":false}}"#],
                )?;
                Ok(())
            })
            .unwrap();
        let rules = QueryService::new(&database).automation_rules().unwrap();
        assert_eq!(rules.low_confidence_threshold, 0.8);
        assert_eq!(rules.final_name_pattern, "{Неделя}");
        // Производные значения и инварианты — из контракта, а не из JSON.
        assert_eq!(rules.inbox_path, "Предметы/00 Входящие");
        assert!(rules.confirm.low_confidence_route && rules.confirm.mark_final);
        let json = serde_json::to_value(&rules).unwrap();
        for retired in [
            "semesterStart",
            "weekDetection",
            "backupBeforeChange",
            "versionPattern",
        ] {
            assert!(
                json.get(retired).is_none(),
                "{retired} must not be in the contract"
            );
        }
    }

    #[test]
    fn activity_log_has_deterministic_newest_first_order() {
        let database = fixture_database();
        let operations = QueryService::new(&database).activity_log().unwrap();
        assert_eq!(
            operations
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["operation-b", "operation-a"]
        );
        assert_eq!(operations[1].paths, ["Предметы/ОБА/Неделя 07"]);
    }

    #[test]
    fn missing_ids_return_not_found() {
        let database = fixture_database();
        let service = QueryService::new(&database);
        assert_eq!(
            service.week("subject-oba", 99).unwrap_err().code,
            ErrorCode::NotFound
        );
        assert_eq!(
            service.homework("missing").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn corrupted_json_returns_database_error_without_panic() {
        let database = fixture_database();
        database.with_connection(|connection| {
            connection.execute_batch("DROP TRIGGER homework_versions_no_update; PRAGMA ignore_check_constraints = ON;")?;
            connection.execute("UPDATE homework_versions SET files_json = '{' WHERE id = 'version-1'", [])?;
            Ok(())
        }).unwrap();
        let error = QueryService::new(&database)
            .homework("homework-7")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Database);
        assert!(error.message.contains("homework_versions.files_json"));
    }

    #[test]
    fn legacy_empty_version_links_use_documented_safe_fallbacks() {
        let database = fixture_database();
        database.with_connection(|connection| {
            connection.execute_batch("DROP TRIGGER homework_versions_no_update;")?;
            connection.execute(
                "UPDATE homework_versions SET file_name = '', source_file_id = '', files_json = '[]', prompt_id = NULL, source_bundle_id = NULL WHERE id = 'version-1'",
                [],
            )?;
            Ok(())
        }).unwrap();
        let version = QueryService::new(&database)
            .homework("homework-7")
            .unwrap()
            .versions
            .remove(0);
        assert_eq!(version.file_name, "Решение_v01.docx");
        assert_eq!(version.source_file_id, "legacy:version-1");
        assert_eq!(
            version.files[0].relative_path,
            "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/Решение_v01.docx"
        );
        assert_eq!(version.prompt_id, None);
        assert_eq!(version.source_bundle_id, None);
    }

    #[test]
    fn integration_fixture_migrates_and_reads_real_aggregates() {
        let database = fixture_database();
        let service = QueryService::new(&database);
        assert_eq!(service.subjects().unwrap()[0].id, "subject-oba");
        assert_eq!(service.weeks("subject-oba").unwrap().len(), 2);
        assert_eq!(service.homework_list().unwrap()[0].versions.len(), 1);
        assert_eq!(service.inbox_files().unwrap()[0].id, "inbox-1");
    }

    #[test]
    fn backend_health_reports_sqlite_and_schema_version_without_path() {
        let database = fixture_database();
        let health = QueryService::new(&database).health().unwrap();
        assert!(health.ready);
        assert!(health.database_exists);
        assert_eq!(health.schema_version, CURRENT_SCHEMA_VERSION);
        let value: Value = serde_json::to_value(health).unwrap();
        assert!(value.get("databasePath").is_none());
    }

    #[test]
    fn scenario_a_file_backed_state_survives_restart() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("kontur.sqlite3");
        {
            let database = Database::open_file(&path).unwrap();
            assert_eq!(database.migrate().unwrap(), CURRENT_SCHEMA_VERSION);
            seed_fixture(&database);
            database
                .with_connection(|connection| {
                    connection.execute(
                        "INSERT INTO teacher_comments(id, homework_id, question, answer, source, date, added_at, authority, official_confirmed_at) VALUES ('comment-1', 'homework-7', 'Что исправить?', 'Добавить шлюз', 'chat', '2026-10-20', '2026-10-20', 'teacher_official', '2026-10-20')",
                        [],
                    )?;
                    connection.execute(
                        "INSERT INTO teacher_requirements(id, homework_id, comment_id, text, origin, status, note, checked_version_id) VALUES ('requirement-1', 'homework-7', 'comment-1', 'Добавить шлюз', 'ответ', 'not_done', '', 'version-1')",
                        [],
                    )?;
                    connection.execute(
                        "INSERT INTO submission_packages(id, homework_id, final_version_id, status, package_path, checklist_json, items_json, links_json, warnings_json, upload_list_json, manifest_json, built_at) VALUES ('package-1', 'homework-7', 'version-1', 'draft', 'Предметы/ОБА/Неделя 07/03 ДЗ/03 На сдачу', '[]', '[]', '[]', '[]', '[]', '[]', '2026-10-20T12:00:00Z')",
                        [],
                    )?;
                    Ok(())
                })
                .unwrap();
        }

        let reopened = Database::open_file(&path).unwrap();
        assert_eq!(reopened.migrate().unwrap(), CURRENT_SCHEMA_VERSION);
        let service = QueryService::new(&reopened);
        assert_eq!(service.subjects().unwrap().len(), 1);
        assert_eq!(service.weeks("subject-oba").unwrap().len(), 2);
        assert_eq!(service.homework("homework-7").unwrap().versions.len(), 1);
        assert_eq!(service.activity_log().unwrap().len(), 2);
        assert_eq!(
            service.automation_rules().unwrap().workflow.style_threshold,
            6
        );
        reopened
            .with_connection(|connection| {
                for table in [
                    "teacher_comments",
                    "teacher_requirements",
                    "submission_packages",
                ] {
                    let count: i64 = connection.query_row(
                        &format!("SELECT count(*) FROM {table}"),
                        [],
                        |row| row.get(0),
                    )?;
                    assert_eq!(count, 1, "{table} must survive restart");
                }
                Ok(())
            })
            .unwrap();
    }
}
