use rusqlite::Connection;

use crate::error::BackendError;

pub const CURRENT_SCHEMA_VERSION: u32 = 8;

struct Migration {
    version: u32,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("v1.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("v2.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("v3.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("v4.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("v5.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("v6.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("v7.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("v8.sql"),
    },
];

pub fn schema_version(connection: &Connection) -> Result<u32, BackendError> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version)
}

pub fn migrate(connection: &mut Connection) -> Result<u32, BackendError> {
    let current = schema_version(connection)?;
    if current > CURRENT_SCHEMA_VERSION {
        return Err(BackendError::conflict(format!(
            "database schema version {current} is newer than supported version {CURRENT_SCHEMA_VERSION}"
        )));
    }

    for migration in MIGRATIONS.iter().filter(|item| item.version > current) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.pragma_update(None, "user_version", migration.version)?;
        transaction.commit()?;
    }

    schema_version(connection)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{migrate, schema_version, CURRENT_SCHEMA_VERSION};
    use crate::error::ErrorCode;

    #[test]
    fn migrates_empty_database() {
        let mut connection = Connection::open_in_memory().unwrap();
        assert_eq!(migrate(&mut connection).unwrap(), CURRENT_SCHEMA_VERSION);
        let expected = [
            "app_settings",
            "corpus_selection_items",
            "corpus_selections",
            "download_seen",
            "extraction_batches",
            "extractions",
            "files",
            "homework_versions",
            "homeworks",
            "inbox_files",
            "material_extractions",
            "managed_notes",
            "operation_results",
            "operations",
            "review_runs",
            "route_operation_items",
            "revision_prompts",
            "source_bundle_items",
            "source_bundles",
            "style_confirmations",
            "style_profile_sources",
            "subjects",
            "submission_packages",
            "submission_receipts",
            "teacher_comments",
            "teacher_requirements",
            "weeks",
        ];
        for table in expected {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing table: {table}");
        }
    }

    #[test]
    fn migration_is_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        migrate(&mut connection).unwrap();
        assert_eq!(schema_version(&connection).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn rejects_newer_schema() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        let error = migrate(&mut connection).unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
    }

    #[test]
    fn migrates_populated_v1_database_without_losing_records() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        connection.execute_batch(include_str!("v1.sql")).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute_batch(
                "
                INSERT INTO app_settings(key, value_json) VALUES ('automation', '{}');
                INSERT INTO subjects(id, name, folder_path) VALUES ('s1', 'ОБА', 'Предметы/ОБА');
                INSERT INTO weeks(id, subject_id, number, folder_path)
                    VALUES ('w1', 's1', 7, 'Предметы/ОБА/Неделя 07');
                INSERT INTO homeworks(id, week_id, title, deadline, stage, workflow_stage, folder_path)
                    VALUES ('h1', 'w1', 'ДЗ', '2026-10-25T20:00:00Z', 'in_progress', 'review', 'Предметы/ОБА/Неделя 07/03 ДЗ');
                INSERT INTO files(id, homework_id, category, name, kind, size_bytes, relative_path)
                    VALUES ('f1', 'h1', 'homework_task', 'task.pdf', 'pdf', 10, 'Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/task.pdf');
                INSERT INTO homework_versions(id, homework_id, number, label, file_path, author)
                    VALUES ('v1', 'h1', 1, 'v01', 'Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md', 'user');
                INSERT INTO source_bundles(id, homework_id, version_id, status)
                    VALUES ('b1', 'h1', 'v1', 'ready');
                INSERT INTO source_bundle_items(id, source_bundle_id, file_id, required, included)
                    VALUES ('bi1', 'b1', 'f1', 1, 1);
                INSERT INTO review_runs(id, homework_id, version_id, provider, status, style_threshold)
                    VALUES ('r1', 'h1', 'v1', 'codex', 'completed', 4);
                INSERT INTO revision_prompts(id, homework_id, review_run_id, content, status)
                    VALUES ('p1', 'h1', 'r1', 'Исправить', 'draft');
                INSERT INTO teacher_comments(id, homework_id, question, answer, source)
                    VALUES ('c1', 'h1', 'Вопрос', 'Ответ', 'chat');
                INSERT INTO teacher_requirements(id, homework_id, comment_id, text, status)
                    VALUES ('t1', 'h1', 'c1', 'Требование', 'met');
                INSERT INTO submission_packages(id, homework_id, final_version_id, status, package_path)
                    VALUES ('sp1', 'h1', 'v1', 'ready', 'Предметы/ОБА/Неделя 07/03 ДЗ/03 На сдачу');
                INSERT INTO submission_receipts(id, package_id, homework_id, submitted_version_id, snapshot_path, snapshot_hash, confirmed_at)
                    VALUES ('sr1', 'sp1', 'h1', 'v1', 'Предметы/ОБА/Неделя 07/03 ДЗ/04 Отправлено/v01', 'hash', '2026-10-25T21:00:00Z');
                INSERT INTO operations(id, kind, status) VALUES ('o1', 'route', 'completed');
                ",
            )
            .unwrap();

        assert_eq!(migrate(&mut connection).unwrap(), CURRENT_SCHEMA_VERSION);
        for table in [
            "app_settings",
            "subjects",
            "weeks",
            "homeworks",
            "files",
            "homework_versions",
            "source_bundles",
            "source_bundle_items",
            "review_runs",
            "revision_prompts",
            "teacher_comments",
            "teacher_requirements",
            "submission_packages",
            "submission_receipts",
            "operations",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1, "record lost from {table}");
        }
        let requirement_status: String = connection
            .query_row(
                "SELECT status FROM teacher_requirements WHERE id = 't1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(requirement_status, "done");
    }

    fn v2_database_with_inbox_row() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        connection.execute_batch(include_str!("v1.sql")).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute_batch(include_str!("v2.sql")).unwrap();
        transaction.pragma_update(None, "user_version", 2).unwrap();
        transaction.commit().unwrap();
        connection
            .execute(
                "INSERT INTO inbox_files(id, name, kind, size_bytes, added_at, origin, status, relative_path)
                 VALUES ('legacy', 'Лекция.pdf', 'pdf', 10, '2026-10-20T10:00:00Z', 'drop', 'new',
                         'Предметы/00 Входящие/Лекция.pdf')",
                [],
            )
            .unwrap();
        connection
    }

    #[test]
    fn v3_keeps_legacy_inbox_rows_without_hash() {
        let mut connection = v2_database_with_inbox_row();
        assert_eq!(migrate(&mut connection).unwrap(), CURRENT_SCHEMA_VERSION);
        let (name, sha): (String, Option<String>) = connection
            .query_row(
                "SELECT name, sha256 FROM inbox_files WHERE id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "Лекция.pdf");
        assert!(sha.is_none());
    }

    #[test]
    fn v3_rejects_two_active_inbox_rows_with_same_content() {
        let mut connection = v2_database_with_inbox_row();
        migrate(&mut connection).unwrap();
        let sha = "a".repeat(64);
        let insert = |id: &str, path: &str, status: &str| {
            connection.execute(
                "INSERT INTO inbox_files(id, name, kind, size_bytes, added_at, origin, status, relative_path, sha256)
                 VALUES (?1, 'x.pdf', 'pdf', 1, '2026-10-21T10:00:00Z', 'drop', ?3, ?2, ?4)",
                rusqlite::params![id, path, status, sha],
            )
        };
        insert("a", "Предметы/00 Входящие/a.pdf", "new").unwrap();
        assert!(insert("b", "Предметы/00 Входящие/b.pdf", "new").is_err());
        insert("c", "Предметы/00 Входящие/c.pdf", "routed").unwrap();
        assert!(connection
            .execute(
                "UPDATE inbox_files SET sha256 = 'XYZ' WHERE id = 'legacy'",
                [],
            )
            .is_err());
    }

    #[test]
    fn v3_operation_results_reject_absolute_paths_and_updates() {
        let mut connection = v2_database_with_inbox_row();
        migrate(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO operations(id, kind, status) VALUES ('o1', 'import', 'completed')",
                [],
            )
            .unwrap();
        let insert = |json: &str| {
            connection.execute(
                "INSERT INTO operation_results(operation_id, kind, result_json) VALUES ('o1', 'inbox_import', ?1)",
                [json],
            )
        };
        assert!(insert(r#"{"items":[{"name":"/Users/me/secret.pdf"}]}"#).is_err());
        assert!(insert(r#"{"items":[{"name":"file:///tmp/x"}]}"#).is_err());
        assert!(insert(r#"{"items":[{"name":"~/Downloads/x"}]}"#).is_err());
        insert(r#"{"items":[{"name":"Лекция.pdf","path":"Предметы/00 Входящие/Лекция.pdf"}]}"#)
            .unwrap();
        assert!(connection
            .execute("UPDATE operation_results SET result_json = '{}'", [])
            .is_err());
    }

    #[test]
    fn v4_keeps_v3_inbox_rows_and_adds_nullable_provenance() {
        let mut connection = v2_database_with_inbox_row();
        migrate(&mut connection).unwrap();
        let (provenance, course): (Option<String>, i64) = connection
            .query_row(
                "SELECT provenance_json, (SELECT count(*) FROM subjects WHERE course_url IS NOT NULL)
                 FROM inbox_files WHERE id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(provenance.is_none());
        assert_eq!(course, 0);
        assert_eq!(schema_version(&connection).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn v4_provenance_rejects_absolute_paths_on_insert_and_update() {
        let mut connection = v2_database_with_inbox_row();
        migrate(&mut connection).unwrap();
        let insert = |id: &str, json: &str| {
            connection.execute(
                "INSERT INTO inbox_files(id, name, kind, size_bytes, added_at, origin, status, relative_path, provenance_json)
                 VALUES (?1, 'x.pdf', 'pdf', 1, '2026-10-21T10:00:00Z', 'lms', 'new', ?2, ?3)",
                rusqlite::params![id, format!("Предметы/00 Входящие/{id}.pdf"), json],
            )
        };
        for bad in [
            r#"{"source":"cu_export","entryPath":"/Users/me/x.pdf"}"#,
            r#"{"source":"cu_export","entryPath":"../x.pdf"}"#,
            r#"{"source":"cu_export","entryPath":"a/../../x.pdf"}"#,
            r#"{"source":"cu_export","archiveName":"C:\\x.zip"}"#,
            r#"{"source":"cu_export","archiveName":"~/Downloads/x.zip"}"#,
            r#"{"source":"cu_export","archiveName":"file:///x.zip"}"#,
        ] {
            assert!(insert("bad", bad).is_err(), "accepted {bad}");
        }
        insert(
            "ok",
            r#"{"source":"cu_export","archiveName":"Курс.zip","entryPath":"Неделя 3/Лекция.pdf","courseUrl":"https://my.centraluniversity.ru/learn/courses/1"}"#,
        )
        .unwrap();
        assert!(connection
            .execute(
                "UPDATE inbox_files SET provenance_json = '{\"entryPath\":\"/etc/passwd\"}' WHERE id = 'ok'",
                [],
            )
            .is_err());
    }

    #[test]
    fn v4_course_url_requires_https_and_download_seen_rejects_paths() {
        let mut connection = v2_database_with_inbox_row();
        migrate(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO subjects(id, name, short_name, folder_path) VALUES ('s1', 'ОБА', 'ОБА', 'Предметы/ОБА')",
                [],
            )
            .unwrap();
        assert!(connection
            .execute(
                "UPDATE subjects SET course_url = 'http://x' WHERE id = 's1'",
                []
            )
            .is_err());
        connection
            .execute(
                "UPDATE subjects SET course_url = 'https://my.centraluniversity.ru/learn/courses/1' WHERE id = 's1'",
                [],
            )
            .unwrap();
        let seen = |name: &str| {
            connection.execute(
                "INSERT INTO download_seen(sha256, name, size_bytes, outcome, first_seen_at, updated_at)
                 VALUES (?1, ?2, 1, 'imported', 'now', 'now')",
                rusqlite::params!["b".repeat(64), name],
            )
        };
        assert!(seen("Downloads/x.zip").is_err());
        assert!(seen("a\\b.zip").is_err());
        seen("Курс.zip").unwrap();
    }

    #[test]
    fn v5_keeps_earlier_rows_and_starts_with_empty_extraction_tables() {
        let mut connection = v2_database_with_inbox_row();
        assert_eq!(migrate(&mut connection).unwrap(), CURRENT_SCHEMA_VERSION);
        let (inbox, extractions, links, selections): (i64, i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT count(*) FROM inbox_files), (SELECT count(*) FROM extractions),
                        (SELECT count(*) FROM material_extractions), (SELECT count(*) FROM corpus_selections)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!((inbox, extractions, links, selections), (1, 0, 0, 0));
        let applied: i64 = connection
            .query_row(
                "SELECT count(*) FROM schema_migrations WHERE version = 5",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(applied, 1);
    }

    #[test]
    fn v5_constraints_protect_cache_reports_and_selections() {
        let mut connection = v2_database_with_inbox_row();
        migrate(&mut connection).unwrap();
        let insert = |id: &str, sha: &str, status: &str| {
            connection.execute(
                "INSERT INTO extractions(id, sha256, extractor_version, options_key, format, status,
                    metadata_json, structure_json, blocks_truncated, text, text_chars, truncated,
                    warnings_json, ocr_used, source_size_bytes, duration_ms, created_at)
                 VALUES (?1, ?2, 1, 'pdf', 'pdf', ?3, '{}', '[]', 0, '', 0, 0, '[]', 0, 0, 0, 'now')",
                rusqlite::params![id, sha, status],
            )
        };
        assert!(insert("e0", "XYZ", "complete").is_err());
        assert!(insert("e0", &"a".repeat(64), "magic").is_err());
        insert("e1", &"a".repeat(64), "complete").unwrap();
        assert!(
            insert("e2", &"a".repeat(64), "partial").is_err(),
            "one row per cache key"
        );

        connection
            .execute(
                "INSERT INTO operations(id, kind, status) VALUES ('op', 'extract', 'completed')",
                [],
            )
            .unwrap();
        let report = |json: &str| {
            connection.execute(
                "INSERT INTO extraction_batches(operation_id, report_json) VALUES ('op', ?1)",
                [json],
            )
        };
        assert!(report(r#"{"items":[{"name":"/Users/me/secret.pdf"}]}"#).is_err());
        assert!(report(r#"{"items":[{"name":"~/x.pdf"}]}"#).is_err());
        report(r#"{"items":[{"name":"Лекция.pdf"}]}"#).unwrap();
        assert!(connection
            .execute("UPDATE extraction_batches SET report_json = '{}'", [])
            .is_err());

        connection
            .execute_batch(&format!(
                "INSERT INTO material_extractions(material_kind, material_id, sha256, extraction_id, analyzed_at)
                     VALUES ('inbox', 'legacy', '{sha}', 'e1', 'now');
                 INSERT INTO corpus_selections(id, homework_id, created_at, updated_at)
                     VALUES ('sel', NULL, 'now', 'now');
                 INSERT INTO corpus_selection_items(selection_id, material_kind, material_id, sha256, position, added_at)
                     VALUES ('sel', 'inbox', 'legacy', '{sha}', 0, 'now');",
                sha = "a".repeat(64)
            ))
            .unwrap();
        assert!(connection
            .execute(
                "INSERT INTO corpus_selections(id, homework_id, created_at, updated_at) VALUES ('sel2', NULL, 'now', 'now')",
                [],
            )
            .is_err(), "only one general selection");
        connection
            .execute("DELETE FROM inbox_files WHERE id = 'legacy'", [])
            .unwrap();
        let (links, items, cached): (i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT count(*) FROM material_extractions),
                        (SELECT count(*) FROM corpus_selection_items),
                        (SELECT count(*) FROM extractions)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (links, items),
            (0, 0),
            "deleted material drops links and selection items"
        );
        assert_eq!(cached, 1, "content cache survives material deletion");
    }
}
