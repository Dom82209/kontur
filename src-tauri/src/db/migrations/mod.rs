use rusqlite::Connection;

use crate::error::BackendError;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

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
            "files",
            "homework_versions",
            "homeworks",
            "inbox_files",
            "operations",
            "review_runs",
            "revision_prompts",
            "source_bundle_items",
            "source_bundles",
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

        assert_eq!(migrate(&mut connection).unwrap(), 2);
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
}
