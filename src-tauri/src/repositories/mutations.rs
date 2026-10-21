use std::path::{Component, Path};

use rusqlite::{params, Connection, Error as SqliteError, ErrorCode as SqliteErrorCode, Row};
use serde_json::Value;

use crate::{
    db::Database,
    domain::{
        Homework, HomeworkStage, HomeworkVersion, HomeworkWorkflowStage, InboxFile, NewHomework,
        NewHomeworkVersion, NewInboxFile, NewOperation, NewSourceBundle, NewSubject, NewWeek,
        OperationRecord, SourceBundle, Subject, VersionAuthor, Week,
    },
    error::BackendError,
};

pub struct Repositories<'a> {
    database: &'a Database,
}

impl<'a> Repositories<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn schema_version(&self) -> Result<u32, BackendError> {
        self.database.schema_version()
    }

    pub fn set_setting(&self, key: &str, value: &Value) -> Result<(), BackendError> {
        validate_setting_is_not_secret(key, value)?;
        let value_json = serde_json::to_string(value)
            .map_err(|error| BackendError::database(error.to_string()))?;
        self.database.with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO app_settings(key, value_json) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = CURRENT_TIMESTAMP",
                params![key, value_json],
            )?;
            Ok(())
        })
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<Value>, BackendError> {
        self.database.with_connection(|connection| {
            let value = query_optional_string(
                connection,
                "SELECT value_json FROM app_settings WHERE key = ?1",
                key,
            )?;
            value
                .map(|json| {
                    serde_json::from_str(&json)
                        .map_err(|error| BackendError::database(error.to_string()))
                })
                .transpose()
        })
    }

    pub fn create_subject(&self, subject: NewSubject<'_>) -> Result<Subject, BackendError> {
        validate_relative_path(subject.folder_path)?;
        if subject.short_name.trim().is_empty() {
            return Err(BackendError::conflict(
                "subject short name must not be empty",
            ));
        }
        self.database.with_transaction(|transaction| {
            insert_subject(transaction, &subject)?;
            Ok(Subject {
                id: subject.id.to_owned(),
                name: subject.name.to_owned(),
                short_name: subject.short_name.to_owned(),
                folder_path: subject.folder_path.to_owned(),
            })
        })
    }

    pub fn get_subject(&self, id: &str) -> Result<Subject, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, name, short_name, folder_path FROM subjects WHERE id = ?1",
                    [id],
                    |row| {
                        Ok(Subject {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            short_name: row.get(2)?,
                            folder_path: row.get(3)?,
                        })
                    },
                )
                .map_err(|error| map_read_error(error, "subject not found"))
        })
    }

    pub fn create_week(&self, week: NewWeek<'_>) -> Result<Week, BackendError> {
        validate_relative_path(week.folder_path)?;
        self.database.with_transaction(|transaction| {
            insert_week(transaction, &week)?;
            Ok(Week {
                id: week.id.to_owned(),
                subject_id: week.subject_id.to_owned(),
                number: week.number,
                starts_at: week.starts_at.to_owned(),
                ends_at: week.ends_at.to_owned(),
                topic: week.topic.map(str::to_owned),
                folder_path: week.folder_path.to_owned(),
            })
        })
    }

    pub fn get_week(&self, id: &str) -> Result<Week, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, subject_id, number, starts_at, ends_at, topic, folder_path FROM weeks WHERE id = ?1",
                    [id],
                    |row| {
                        Ok(Week {
                            id: row.get(0)?,
                            subject_id: row.get(1)?,
                            number: row.get(2)?,
                            starts_at: row.get(3)?,
                            ends_at: row.get(4)?,
                            topic: row.get(5)?,
                            folder_path: row.get(6)?,
                        })
                    },
                )
                .map_err(|error| map_read_error(error, "week not found"))
        })
    }

    pub fn create_homework(&self, homework: NewHomework<'_>) -> Result<Homework, BackendError> {
        validate_relative_path(homework.folder_path)?;
        self.database.with_transaction(|transaction| {
            insert_homework(transaction, &homework)?;
            homework_from_new(&homework)
        })
    }

    pub fn get_homework(&self, id: &str) -> Result<Homework, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, week_id, title, deadline, stage, workflow_stage, final_version_id,
                            final_file_name, submitted_at, folder_path, assignment_url
                     FROM homeworks WHERE id = ?1",
                    [id],
                    homework_from_row,
                )
                .map_err(|error| map_read_error(error, "homework not found"))
        })
    }

    pub fn update_homework_title(&self, id: &str, title: &str) -> Result<(), BackendError> {
        self.database.with_transaction(|transaction| {
            let stage: String = transaction
                .query_row("SELECT stage FROM homeworks WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .map_err(|error| map_read_error(error, "homework not found"))?;
            if stage == "submitted" {
                return Err(BackendError::immutable(
                    "submitted homework is an immutable snapshot",
                ));
            }
            transaction.execute(
                "UPDATE homeworks SET title = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![id, title],
            )?;
            Ok(())
        })
    }

    pub fn add_homework_version(
        &self,
        version: NewHomeworkVersion<'_>,
    ) -> Result<HomeworkVersion, BackendError> {
        validate_relative_path(version.file_path)?;
        self.database.with_transaction(|transaction| {
            insert_homework_version(transaction, &version)?;
            let created_at = transaction.query_row(
                "SELECT created_at FROM homework_versions WHERE id = ?1",
                [version.id],
                |row| row.get(0),
            )?;
            Ok(version_from_new(&version, created_at))
        })
    }

    pub fn list_homework_versions(
        &self,
        homework_id: &str,
    ) -> Result<Vec<HomeworkVersion>, BackendError> {
        self.database.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, homework_id, number, label, file_name, file_path, size_bytes, created_at,
                        source_file_id, note, sections_json, author, prompt_id, source_bundle_id,
                        files_json, frozen
                 FROM homework_versions WHERE homework_id = ?1 ORDER BY number",
            )?;
            let rows = statement.query_map([homework_id], |row| {
                let raw_author: String = row.get(11)?;
                Ok(HomeworkVersion {
                    id: row.get(0)?,
                    homework_id: row.get(1)?,
                    number: row.get(2)?,
                    label: row.get(3)?,
                    file_name: row.get(4)?,
                    file_path: row.get(5)?,
                    size_bytes: row.get(6)?,
                    created_at: row.get(7)?,
                    source_file_id: row.get(8)?,
                    note: row.get(9)?,
                    sections: json_from_column(row, 10)?,
                    author: VersionAuthor::try_from_db(&raw_author)
                        .map_err(|message| conversion_error(11, message))?,
                    prompt_id: row.get(12)?,
                    source_bundle_id: row.get(13)?,
                    files: json_from_column(row, 14)?,
                    frozen: row.get::<_, i64>(15)? == 1,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    pub fn create_homework_with_initial_version(
        &self,
        homework: NewHomework<'_>,
        version: NewHomeworkVersion<'_>,
    ) -> Result<(Homework, HomeworkVersion), BackendError> {
        validate_relative_path(homework.folder_path)?;
        validate_relative_path(version.file_path)?;
        if homework.id != version.homework_id {
            return Err(BackendError::conflict(
                "initial version must belong to the new homework",
            ));
        }
        self.database.with_transaction(|transaction| {
            insert_homework(transaction, &homework)?;
            insert_homework_version(transaction, &version)?;
            let created_at = transaction.query_row(
                "SELECT created_at FROM homework_versions WHERE id = ?1",
                [version.id],
                |row| row.get(0),
            )?;
            Ok((
                homework_from_new(&homework)?,
                version_from_new(&version, created_at),
            ))
        })
    }

    pub fn record_operation(
        &self,
        operation: NewOperation<'_>,
    ) -> Result<OperationRecord, BackendError> {
        if let Some(path) = operation.backup_path {
            validate_relative_path(path)?;
        }
        if let Some(paths) = operation.paths.as_array() {
            for path in paths.iter().filter_map(Value::as_str) {
                validate_relative_path(path)?;
            }
        }
        validate_json_paths(operation.error)?;
        validate_json_paths(operation.undo_payload)?;
        let error_json = optional_json_text(operation.error)?;
        let undo_payload_json = optional_json_text(operation.undo_payload)?;
        self.database.with_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO operations(
                        id, kind, title, detail, status, undoable, paths_json, backup_path, homework_id,
                        error_json, undo_payload_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        operation.id,
                        operation.kind,
                        operation.title,
                        operation.detail,
                        operation.status.as_db_str(),
                        i64::from(operation.undoable),
                        json_text(operation.paths)?,
                        operation.backup_path,
                        operation.homework_id,
                        error_json,
                        undo_payload_json,
                    ],
                )
                .map_err(|error| {
                    map_write_error(error, "operation already exists or is invalid")
                })?;
            Ok(OperationRecord {
                id: operation.id.to_owned(),
                kind: operation.kind.to_owned(),
                title: operation.title.to_owned(),
                detail: operation.detail.map(str::to_owned),
                status: operation.status,
                undoable: operation.undoable,
                paths: operation.paths.clone(),
                backup_path: operation.backup_path.map(str::to_owned),
                homework_id: operation.homework_id.map(str::to_owned),
                error: operation.error.cloned(),
                undo_payload: operation.undo_payload.cloned(),
            })
        })
    }

    pub fn add_inbox_file(&self, file: NewInboxFile<'_>) -> Result<InboxFile, BackendError> {
        validate_inbox_path(file.relative_path)?;
        validate_json_paths(file.classification)?;
        validate_json_paths(file.duplicate)?;
        let classification = optional_json_text(file.classification)?;
        let duplicate = optional_json_text(file.duplicate)?;
        self.database.with_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO inbox_files(
                        id, name, kind, size_bytes, added_at, origin, status, relative_path,
                        classification_json, duplicate_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        file.id,
                        file.name,
                        file.kind,
                        file.size_bytes,
                        file.added_at,
                        file.origin,
                        file.status,
                        file.relative_path,
                        classification,
                        duplicate
                    ],
                )
                .map_err(|error| {
                    map_write_error(error, "inbox file already exists or is invalid")
                })?;
            Ok(InboxFile {
                id: file.id.to_owned(),
                name: file.name.to_owned(),
                kind: file.kind.to_owned(),
                size_bytes: file.size_bytes,
                added_at: file.added_at.to_owned(),
                origin: file.origin.to_owned(),
                status: file.status.to_owned(),
                relative_path: file.relative_path.to_owned(),
                classification: file.classification.cloned(),
                duplicate: file.duplicate.cloned(),
            })
        })
    }

    pub fn get_inbox_file(&self, id: &str) -> Result<InboxFile, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, name, kind, size_bytes, added_at, origin, status, relative_path,
                            classification_json, duplicate_json
                     FROM inbox_files WHERE id = ?1",
                    [id],
                    |row| {
                        Ok(InboxFile {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            kind: row.get(2)?,
                            size_bytes: row.get(3)?,
                            added_at: row.get(4)?,
                            origin: row.get(5)?,
                            status: row.get(6)?,
                            relative_path: row.get(7)?,
                            classification: optional_json_from_column(row, 8)?,
                            duplicate: optional_json_from_column(row, 9)?,
                        })
                    },
                )
                .map_err(|error| map_read_error(error, "inbox file not found"))
        })
    }

    pub fn create_source_bundle(
        &self,
        bundle: NewSourceBundle<'_>,
    ) -> Result<SourceBundle, BackendError> {
        let missing_json = json_text(bundle.missing)?;
        self.database.with_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO source_bundles(
                        id, homework_id, purpose, version_id, complete, missing_json
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        bundle.id,
                        bundle.homework_id,
                        bundle.purpose,
                        bundle.version_id,
                        i64::from(bundle.complete),
                        missing_json
                    ],
                )
                .map_err(|error| map_write_error(error, "source bundle is invalid"))?;
            let created_at = transaction.query_row(
                "SELECT created_at FROM source_bundles WHERE id = ?1",
                [bundle.id],
                |row| row.get(0),
            )?;
            Ok(SourceBundle {
                id: bundle.id.to_owned(),
                homework_id: bundle.homework_id.to_owned(),
                purpose: bundle.purpose.to_owned(),
                version_id: bundle.version_id.map(str::to_owned),
                items: Vec::new(),
                complete: bundle.complete,
                missing: bundle.missing.clone(),
                created_at,
            })
        })
    }
}

fn insert_subject(connection: &Connection, subject: &NewSubject<'_>) -> Result<(), BackendError> {
    connection
        .execute(
            "INSERT INTO subjects(id, name, short_name, folder_path) VALUES (?1, ?2, ?3, ?4)",
            params![
                subject.id,
                subject.name,
                subject.short_name,
                subject.folder_path
            ],
        )
        .map_err(|error| map_write_error(error, "subject already exists or is invalid"))?;
    Ok(())
}

fn insert_week(connection: &Connection, week: &NewWeek<'_>) -> Result<(), BackendError> {
    connection
        .execute(
            "INSERT INTO weeks(
                id, subject_id, number, starts_at, ends_at, topic, folder_path
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                week.id,
                week.subject_id,
                week.number,
                week.starts_at,
                week.ends_at,
                week.topic,
                week.folder_path
            ],
        )
        .map_err(|error| map_write_error(error, "week already exists or is invalid"))?;
    Ok(())
}

fn insert_homework(
    connection: &Connection,
    homework: &NewHomework<'_>,
) -> Result<(), BackendError> {
    connection
        .execute(
            "INSERT INTO homeworks(
                id, week_id, title, deadline, stage, workflow_stage, folder_path, assignment_url
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                homework.id,
                homework.week_id,
                homework.title,
                homework.deadline,
                homework.stage.as_db_str(),
                homework.workflow_stage.as_db_str(),
                homework.folder_path,
                homework.assignment_url
            ],
        )
        .map_err(|error| map_write_error(error, "homework already exists or is invalid"))?;
    Ok(())
}

fn insert_homework_version(
    connection: &Connection,
    version: &NewHomeworkVersion<'_>,
) -> Result<(), BackendError> {
    validate_json_paths(Some(version.files))?;
    let sections_json = json_text(version.sections)?;
    let files_json = json_text(version.files)?;
    connection
        .execute(
            "INSERT INTO homework_versions(
                id, homework_id, number, label, file_name, file_path, size_bytes, source_file_id,
                note, sections_json, author, prompt_id, source_bundle_id, files_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                version.id,
                version.homework_id,
                version.number,
                version.label,
                version.file_name,
                version.file_path,
                version.size_bytes,
                version.source_file_id,
                version.note,
                sections_json,
                version.author.as_str(),
                version.prompt_id,
                version.source_bundle_id,
                files_json
            ],
        )
        .map_err(|error| map_write_error(error, "homework version already exists or is invalid"))?;
    Ok(())
}

fn homework_from_new(homework: &NewHomework<'_>) -> Result<Homework, BackendError> {
    Ok(Homework {
        id: homework.id.to_owned(),
        week_id: homework.week_id.to_owned(),
        title: homework.title.to_owned(),
        deadline: Some(homework.deadline.to_owned()),
        stage: homework.stage,
        workflow_stage: homework.workflow_stage,
        final_version_id: None,
        final_file_name: None,
        submitted_at: None,
        folder_path: homework.folder_path.to_owned(),
        assignment_url: homework.assignment_url.map(str::to_owned),
    })
}

fn version_from_new(version: &NewHomeworkVersion<'_>, created_at: String) -> HomeworkVersion {
    HomeworkVersion {
        id: version.id.to_owned(),
        homework_id: version.homework_id.to_owned(),
        number: version.number,
        label: version.label.to_owned(),
        file_name: version.file_name.to_owned(),
        file_path: version.file_path.to_owned(),
        size_bytes: version.size_bytes,
        created_at,
        source_file_id: version.source_file_id.to_owned(),
        note: version.note.map(str::to_owned),
        sections: version.sections.clone(),
        author: version.author,
        prompt_id: version.prompt_id.map(str::to_owned),
        source_bundle_id: version.source_bundle_id.map(str::to_owned),
        files: version.files.clone(),
        frozen: true,
    }
}

fn homework_from_row(row: &Row<'_>) -> rusqlite::Result<Homework> {
    let raw_stage: String = row.get(4)?;
    let stage =
        HomeworkStage::try_from_db(&raw_stage).map_err(|message| conversion_error(4, message))?;
    let raw_workflow_stage: String = row.get(5)?;
    let workflow_stage = HomeworkWorkflowStage::try_from_db(&raw_workflow_stage)
        .map_err(|message| conversion_error(5, message))?;
    Ok(Homework {
        id: row.get(0)?,
        week_id: row.get(1)?,
        title: row.get(2)?,
        deadline: row.get(3)?,
        stage,
        workflow_stage,
        final_version_id: row.get(6)?,
        final_file_name: row.get(7)?,
        submitted_at: row.get(8)?,
        folder_path: row.get(9)?,
        assignment_url: row.get(10)?,
    })
}

fn query_optional_string(
    connection: &Connection,
    sql: &str,
    parameter: &str,
) -> Result<Option<String>, BackendError> {
    match connection.query_row(sql, [parameter], |row| row.get(0)) {
        Ok(value) => Ok(Some(value)),
        Err(SqliteError::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn map_read_error(error: SqliteError, message: &str) -> BackendError {
    match error {
        SqliteError::QueryReturnedNoRows => BackendError::not_found(message),
        other => other.into(),
    }
}

fn map_write_error(error: SqliteError, message: &str) -> BackendError {
    match &error {
        SqliteError::SqliteFailure(details, _)
            if details.code == SqliteErrorCode::ConstraintViolation =>
        {
            BackendError::conflict(message)
        }
        _ => error.into(),
    }
}

fn validate_relative_path(value: &str) -> Result<(), BackendError> {
    let path = Path::new(value);
    let valid = !value.trim().is_empty()
        && !path.is_absolute()
        && !value.starts_with('\\')
        && !value.contains(':')
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if valid {
        Ok(())
    } else {
        Err(BackendError::conflict(
            "persistent file paths must be safe relative paths",
        ))
    }
}

fn validate_inbox_path(value: &str) -> Result<(), BackendError> {
    validate_relative_path(value)?;
    if value.starts_with("Предметы/00 Входящие/") {
        Ok(())
    } else {
        Err(BackendError::conflict(
            "inbox files must be stored below Предметы/00 Входящие",
        ))
    }
}

fn json_text(value: &Value) -> Result<String, BackendError> {
    serde_json::to_string(value).map_err(|error| BackendError::database(error.to_string()))
}

fn optional_json_text(value: Option<&Value>) -> Result<Option<String>, BackendError> {
    value.map(json_text).transpose()
}

fn json_from_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    let raw: String = row.get(index)?;
    serde_json::from_str(&raw).map_err(|error| conversion_error(index, error.to_string()))
}

fn optional_json_from_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<Value>> {
    let raw: Option<String> = row.get(index)?;
    raw.map(|json| {
        serde_json::from_str(&json).map_err(|error| conversion_error(index, error.to_string()))
    })
    .transpose()
}

fn conversion_error(index: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

const FORBIDDEN_SETTING_FRAGMENTS: &[&str] = &[
    "apikey",
    "password",
    "cookie",
    "accesstoken",
    "refreshtoken",
    "sessionsecret",
];

fn validate_setting_is_not_secret(key: &str, value: &Value) -> Result<(), BackendError> {
    if is_secret_key(key) || json_contains_secret_key(value) {
        return Err(BackendError::conflict(
            "app_settings must not contain passwords, cookies, API keys, tokens, or session secrets",
        ));
    }
    Ok(())
}

fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    FORBIDDEN_SETTING_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

fn json_contains_secret_key(value: &Value) -> bool {
    match value {
        Value::Object(object) => object
            .iter()
            .any(|(key, nested)| is_secret_key(key) || json_contains_secret_key(nested)),
        Value::Array(items) => items.iter().any(json_contains_secret_key),
        _ => false,
    }
}

fn validate_json_paths(value: Option<&Value>) -> Result<(), BackendError> {
    fn visit(value: &Value) -> Result<(), BackendError> {
        match value {
            Value::Object(object) => {
                for (key, nested) in object {
                    if key.to_ascii_lowercase().ends_with("path") {
                        if let Some(path) = nested.as_str() {
                            validate_relative_path(path)?;
                        }
                    }
                    visit(nested)?;
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    value.map(visit).unwrap_or(Ok(()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{db::migrations::CURRENT_SCHEMA_VERSION, error::ErrorCode};

    fn database() -> Database {
        let database = Database::open_in_memory().unwrap();
        database.migrate().unwrap();
        database
    }

    fn seed_subject_week(database: &Database) {
        let repositories = Repositories::new(database);
        repositories
            .create_subject(NewSubject {
                id: "subject-1",
                name: "ОБА",
                short_name: "ОБА",
                folder_path: "Предметы/ОБА",
            })
            .unwrap();
        repositories
            .create_week(NewWeek {
                id: "week-1",
                subject_id: "subject-1",
                number: 7,
                starts_at: "2026-10-19T00:00:00Z",
                ends_at: "2026-10-25T23:59:59Z",
                topic: Some("Оптимизация"),
                folder_path: "Предметы/ОБА/Неделя 07",
            })
            .unwrap();
    }

    fn homework<'a>(id: &'a str) -> NewHomework<'a> {
        NewHomework {
            id,
            week_id: "week-1",
            title: "Домашняя работа",
            deadline: "2026-10-25T20:00:00Z",
            stage: HomeworkStage::InProgress,
            workflow_stage: HomeworkWorkflowStage::ClaudeSolving,
            folder_path: "Предметы/ОБА/Неделя 07/03 ДЗ",
            assignment_url: Some("https://university.example/assignment/7"),
        }
    }

    #[test]
    fn reports_schema_version() {
        let database = database();
        assert_eq!(
            Repositories::new(&database).schema_version().unwrap(),
            CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn settings_round_trip() {
        let database = database();
        let repositories = Repositories::new(&database);
        let value = json!({"reviewer": "codex", "styleThreshold": 4});
        repositories.set_setting("automation", &value).unwrap();
        assert_eq!(repositories.get_setting("automation").unwrap(), Some(value));
    }

    #[test]
    fn settings_reject_secret_keys_at_any_depth() {
        let database = database();
        let repositories = Repositories::new(&database);
        for (key, value) in [
            ("providerApiKey", json!("secret")),
            (
                "workflow",
                json!({"credentials": {"access_token": "secret"}}),
            ),
            ("session_secret", json!("secret")),
            ("cookies", json!(["secret"])),
        ] {
            assert_eq!(
                repositories.set_setting(key, &value).unwrap_err().code,
                ErrorCode::Conflict
            );
        }
    }

    #[test]
    fn inbox_file_round_trip_keeps_classification_without_source_path() {
        let database = database();
        let repositories = Repositories::new(&database);
        let classification = json!({
            "subjectId": "subject-1",
            "weekNumber": 7,
            "category": "homework_task",
            "confidence": 0.91,
            "reasons": ["имя файла"],
            "source": "auto"
        });
        let duplicate = json!({
            "existingPath": "Предметы/ОБА/task.pdf",
            "sameContent": false,
            "suggestedName": "task (2).pdf"
        });
        repositories
            .add_inbox_file(NewInboxFile {
                id: "inbox-1",
                name: "task.pdf",
                kind: "pdf",
                size_bytes: 42,
                added_at: "2026-10-21T08:00:00Z",
                origin: "drop",
                status: "classified",
                relative_path: "Предметы/00 Входящие/task.pdf",
                classification: Some(&classification),
                duplicate: Some(&duplicate),
            })
            .unwrap();
        let restored = repositories.get_inbox_file("inbox-1").unwrap();
        assert_eq!(restored.classification, Some(classification));
        assert_eq!(restored.duplicate, Some(duplicate));
        assert_eq!(restored.relative_path, "Предметы/00 Входящие/task.pdf");
    }

    #[test]
    fn initial_source_bundle_and_review_need_no_version() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        repositories
            .create_homework(homework("homework-1"))
            .unwrap();
        database
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO files(
                        id, homework_id, category, name, kind, size_bytes, modified_at, relative_path
                     ) VALUES ('task-file', 'homework-1', 'homework_task', 'task.pdf', 'pdf', 42,
                               '2026-10-21T08:00:00Z',
                               'Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/task.pdf')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let missing = json!([]);
        let bundle = repositories
            .create_source_bundle(NewSourceBundle {
                id: "bundle-1",
                homework_id: "homework-1",
                purpose: "analyze_materials",
                version_id: None,
                complete: true,
                missing: &missing,
            })
            .unwrap();
        assert_eq!(bundle.version_id, None);

        database
            .with_transaction(|transaction| {
                transaction.execute(
                    "INSERT INTO source_bundle_items(
                        id, source_bundle_id, file_id, source_kind, required, included
                     ) VALUES ('bundle-item-1', 'bundle-1', 'task-file', 'task', 1, 1)",
                    [],
                )?;
                transaction.execute(
                    "INSERT INTO review_runs(
                        id, homework_id, kind, role, provider, version_id, source_bundle_id,
                        status, progress, step, started_at, style_threshold
                     ) VALUES (
                        'review-1', 'homework-1', 'analyze_materials', 'analyst', 'codex', NULL,
                        'bundle-1', 'queued', 0, 'Подготовка', '2026-10-21T08:00:00Z', 4
                     )",
                    [],
                )?;
                transaction.execute(
                    "INSERT INTO revision_prompts(
                        id, homework_id, kind, text, created_by, based_on_version_id, review_run_id
                     ) VALUES ('prompt-1', 'homework-1', 'initial', 'Создать решение', 'codex', NULL, NULL)",
                    [],
                )?;
                let item_count: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM source_bundle_items WHERE source_bundle_id = 'bundle-1'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(item_count, 1);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn invalid_json_is_rejected_by_schema() {
        let database = database();
        let result = database.with_connection(|connection| {
            connection.execute(
                "INSERT INTO inbox_files(
                    id, name, kind, size_bytes, added_at, origin, status, relative_path, classification_json
                 ) VALUES ('bad-json', 'bad.pdf', 'pdf', 1, '2026-10-21T08:00:00Z', 'drop', 'new',
                           'Предметы/00 Входящие/bad.pdf', '{')",
                [],
            )?;
            Ok(())
        });
        assert!(result.is_err());
    }

    #[test]
    fn teacher_requirement_contract_statuses_are_persisted() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        repositories
            .create_homework(homework("homework-1"))
            .unwrap();
        let empty = json!([]);
        repositories
            .add_homework_version(NewHomeworkVersion {
                id: "version-1",
                homework_id: "homework-1",
                number: 1,
                label: "v01",
                file_name: "v01.md",
                file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md",
                size_bytes: 100,
                source_file_id: "source-1",
                note: None,
                sections: &empty,
                author: VersionAuthor::User,
                prompt_id: None,
                source_bundle_id: None,
                files: &empty,
            })
            .unwrap();
        database
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO teacher_comments(
                        id, homework_id, question, answer, source, date, added_at
                     ) VALUES ('comment-1', 'homework-1', 'Вопрос', 'Ответ', 'chat',
                               '2026-10-20T08:00:00Z', '2026-10-21T08:00:00Z')",
                    [],
                )?;
                for status in ["done", "not_done", "manual", "na"] {
                    connection.execute(
                        "INSERT INTO teacher_requirements(
                            id, homework_id, comment_id, text, origin, status, note, checked_version_id
                         ) VALUES (?1, 'homework-1', 'comment-1', 'Требование', 'Ответ', ?2, '', 'version-1')",
                        params![format!("requirement-{status}"), status],
                    )?;
                }
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn creates_and_reads_subject() {
        let database = database();
        let repositories = Repositories::new(&database);
        repositories
            .create_subject(NewSubject {
                id: "subject-1",
                name: "ОБА",
                short_name: "ОБА",
                folder_path: "Предметы/ОБА",
            })
            .unwrap();
        assert_eq!(repositories.get_subject("subject-1").unwrap().name, "ОБА");
    }

    #[test]
    fn enforces_foreign_keys() {
        let database = database();
        let error = Repositories::new(&database)
            .create_week(NewWeek {
                id: "week-1",
                subject_id: "missing",
                number: 7,
                starts_at: "2026-10-19T00:00:00Z",
                ends_at: "2026-10-25T23:59:59Z",
                topic: None,
                folder_path: "Предметы/ОБА/Неделя 07",
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
    }

    #[test]
    fn enforces_unique_week_per_subject() {
        let database = database();
        seed_subject_week(&database);
        let error = Repositories::new(&database)
            .create_week(NewWeek {
                id: "week-duplicate",
                subject_id: "subject-1",
                number: 7,
                starts_at: "2026-10-19T00:00:00Z",
                ends_at: "2026-10-25T23:59:59Z",
                topic: None,
                folder_path: "Предметы/ОБА/Неделя 07 duplicate",
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
    }

    #[test]
    fn creates_subject_week_homework_chain() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        repositories
            .create_homework(homework("homework-1"))
            .unwrap();
        let empty = json!([]);
        repositories
            .add_homework_version(NewHomeworkVersion {
                id: "version-1",
                homework_id: "homework-1",
                number: 1,
                label: "v01",
                file_name: "v01.md",
                file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md",
                size_bytes: 100,
                source_file_id: "source-1",
                note: None,
                sections: &empty,
                author: VersionAuthor::Claude,
                prompt_id: None,
                source_bundle_id: None,
                files: &empty,
            })
            .unwrap();
        assert_eq!(repositories.get_week("week-1").unwrap().number, 7);
        assert_eq!(
            repositories.get_homework("homework-1").unwrap().stage,
            HomeworkStage::InProgress
        );
        assert_eq!(
            repositories.list_homework_versions("homework-1").unwrap()[0].label,
            "v01"
        );
    }

    #[test]
    fn versions_are_immutable_and_ordered() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        repositories
            .create_homework(homework("homework-1"))
            .unwrap();
        let empty = json!([]);
        for number in [2, 1] {
            repositories
                .add_homework_version(NewHomeworkVersion {
                    id: if number == 1 {
                        "version-1"
                    } else {
                        "version-2"
                    },
                    homework_id: "homework-1",
                    number,
                    label: if number == 1 { "v01" } else { "v02" },
                    file_name: if number == 1 { "v01.md" } else { "v02.md" },
                    file_path: if number == 1 {
                        "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md"
                    } else {
                        "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v02.md"
                    },
                    size_bytes: 100,
                    source_file_id: "source-1",
                    note: None,
                    sections: &empty,
                    author: VersionAuthor::Claude,
                    prompt_id: None,
                    source_bundle_id: None,
                    files: &empty,
                })
                .unwrap();
        }
        let versions = repositories.list_homework_versions("homework-1").unwrap();
        assert_eq!(
            versions.iter().map(|item| item.number).collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(versions.iter().all(|item| item.frozen));
    }

    #[test]
    fn duplicate_version_does_not_overwrite() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        repositories
            .create_homework(homework("homework-1"))
            .unwrap();
        let empty = json!([]);
        let first_version = NewHomeworkVersion {
            id: "version-1",
            homework_id: "homework-1",
            number: 1,
            label: "v01",
            file_name: "v01.md",
            file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md",
            size_bytes: 100,
            source_file_id: "source-1",
            note: None,
            sections: &empty,
            author: VersionAuthor::Claude,
            prompt_id: None,
            source_bundle_id: None,
            files: &empty,
        };
        repositories.add_homework_version(first_version).unwrap();
        let error = repositories
            .add_homework_version(NewHomeworkVersion {
                id: "version-another-id",
                homework_id: "homework-1",
                number: 1,
                label: "v01-alternative",
                file_name: "alternative.md",
                file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/alternative.md",
                size_bytes: 100,
                source_file_id: "source-2",
                note: None,
                sections: &empty,
                author: VersionAuthor::Codex,
                prompt_id: None,
                source_bundle_id: None,
                files: &empty,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(
            repositories
                .list_homework_versions("homework-1")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn immutable_version_rejects_direct_update_and_delete() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        repositories
            .create_homework(homework("homework-1"))
            .unwrap();
        let empty = json!([]);
        repositories
            .add_homework_version(NewHomeworkVersion {
                id: "version-1",
                homework_id: "homework-1",
                number: 1,
                label: "v01",
                file_name: "v01.md",
                file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md",
                size_bytes: 100,
                source_file_id: "source-1",
                note: None,
                sections: &empty,
                author: VersionAuthor::User,
                prompt_id: None,
                source_bundle_id: None,
                files: &empty,
            })
            .unwrap();
        database
            .with_connection(|connection| {
                assert!(connection
                    .execute(
                        "UPDATE homework_versions SET label = 'changed' WHERE id = 'version-1'",
                        []
                    )
                    .is_err());
                assert!(connection
                    .execute("DELETE FROM homework_versions WHERE id = 'version-1'", [])
                    .is_err());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn immutable_receipt_rejects_direct_update_and_delete() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        repositories
            .create_homework(homework("homework-1"))
            .unwrap();
        let empty = json!([]);
        repositories
            .add_homework_version(NewHomeworkVersion {
                id: "version-1",
                homework_id: "homework-1",
                number: 1,
                label: "v01",
                file_name: "v01.md",
                file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md",
                size_bytes: 100,
                source_file_id: "source-1",
                note: None,
                sections: &empty,
                author: VersionAuthor::User,
                prompt_id: None,
                source_bundle_id: None,
                files: &empty,
            })
            .unwrap();
        database
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO submission_packages(
                        id, homework_id, final_version_id, status, package_path, built_at
                     ) VALUES ('package-1', 'homework-1', 'version-1', 'ready',
                               'Предметы/ОБА/Неделя 07/03 ДЗ/03 На сдачу', '2026-10-21T07:58:00Z')",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO submission_receipts(
                        id, package_id, homework_id, submitted_version_id, snapshot_path,
                        snapshot_hash, confirmed_at, submitted_at, assignment_url
                     ) VALUES ('receipt-1', 'package-1', 'homework-1', 'version-1',
                               'Предметы/ОБА/Неделя 07/03 ДЗ/04 Отправлено/v01', 'hash',
                               '2026-10-21T08:00:00Z', '2026-10-21T07:59:00Z',
                               'https://university.example/assignment/7')",
                    [],
                )?;
                assert!(connection
                    .execute(
                        "UPDATE submission_receipts SET snapshot_hash = 'changed'",
                        []
                    )
                    .is_err());
                assert!(connection
                    .execute("DELETE FROM submission_receipts", [])
                    .is_err());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn transaction_rolls_back_partial_chain() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        let empty = json!([]);
        let error = repositories
            .create_homework_with_initial_version(
                homework("homework-1"),
                NewHomeworkVersion {
                    id: "version-1",
                    homework_id: "homework-1",
                    number: 0,
                    label: "v00",
                    file_name: "v00.md",
                    file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v00.md",
                    size_bytes: 100,
                    source_file_id: "source-1",
                    note: None,
                    sections: &empty,
                    author: VersionAuthor::Claude,
                    prompt_id: None,
                    source_bundle_id: None,
                    files: &empty,
                },
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(
            repositories.get_homework("homework-1").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn submitted_homework_is_immutable() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        let mut submitted = homework("homework-1");
        submitted.stage = HomeworkStage::Submitted;
        submitted.workflow_stage = HomeworkWorkflowStage::Submitted;
        database
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO homeworks(
                        id, week_id, title, deadline, stage, workflow_stage, folder_path, submitted_at
                     ) VALUES (?1, ?2, ?3, ?4, 'submitted', 'submitted', ?5, CURRENT_TIMESTAMP)",
                    params![
                        submitted.id,
                        submitted.week_id,
                        submitted.title,
                        submitted.deadline,
                        submitted.folder_path
                    ],
                )?;
                Ok(())
            })
            .unwrap();
        let error = repositories
            .update_homework_title("homework-1", "Changed")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Immutable);
        database
            .with_connection(|connection| {
                assert!(connection
                    .execute("DELETE FROM homeworks WHERE id = 'homework-1'", [])
                    .is_err());
                assert!(connection
                    .execute(
                        "INSERT INTO files(
                            id, homework_id, category, name, kind, size_bytes, relative_path
                         ) VALUES ('late-file', 'homework-1', 'homework_task', 'late.pdf', 'pdf', 1,
                                   'Предметы/ОБА/Неделя 07/03 ДЗ/01 Условие и данные/late.pdf')",
                        [],
                    )
                    .is_err());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn operation_is_recorded() {
        let database = database();
        let repositories = Repositories::new(&database);
        let operation = repositories
            .record_operation(NewOperation {
                id: "operation-1",
                kind: "route_files",
                title: "Распределение файлов",
                detail: None,
                status: crate::domain::OperationStatus::Running,
                undoable: true,
                paths: &json!([]),
                backup_path: None,
                homework_id: None,
                error: None,
                undo_payload: Some(&json!({"fileIds": ["file-1"]})),
            })
            .unwrap();
        assert!(operation.undoable);
        assert_eq!(operation.status, crate::domain::OperationStatus::Running);
    }

    #[test]
    fn rejects_absolute_persistent_paths() {
        let database = database();
        let repositories = Repositories::new(&database);
        let error = repositories
            .create_subject(NewSubject {
                id: "subject-1",
                name: "ОБА",
                short_name: "ОБА",
                folder_path: "/Users/example/Предметы/ОБА",
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        let error = repositories
            .add_inbox_file(NewInboxFile {
                id: "inbox-1",
                name: "task.pdf",
                kind: "pdf",
                size_bytes: 1,
                added_at: "2026-10-21T08:00:00Z",
                origin: "drop",
                status: "new",
                relative_path: "/Users/example/task.pdf",
                classification: None,
                duplicate: None,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        let paths = json!(["/Users/example/task.pdf"]);
        let error = repositories
            .record_operation(NewOperation {
                id: "operation-absolute",
                kind: "route",
                title: "Route",
                detail: None,
                status: crate::domain::OperationStatus::Running,
                undoable: false,
                paths: &paths,
                backup_path: None,
                homework_id: None,
                error: None,
                undo_payload: None,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        let direct_result = database.with_connection(|connection| {
            connection.execute(
                "INSERT INTO operations(id, kind, status, paths_json)
                 VALUES ('direct-absolute', 'route', 'running', '[\"/Users/example/task.pdf\"]')",
                [],
            )?;
            Ok(())
        });
        assert!(direct_result.is_err());
    }

    #[test]
    fn test_database_is_memory_only() {
        let database = database();
        assert!(database.path().is_none());
        let backing_file: String = database
            .with_connection(|connection| {
                connection
                    .query_row("PRAGMA database_list", [], |row| row.get(2))
                    .map_err(Into::into)
            })
            .unwrap();
        assert!(backing_file.is_empty());
    }
}
