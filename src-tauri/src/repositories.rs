use std::path::{Component, Path};

use rusqlite::{params, Connection, Error as SqliteError, ErrorCode as SqliteErrorCode, Row};
use serde_json::Value;

use crate::{
    db::Database,
    domain::{
        Homework, HomeworkStage, HomeworkVersion, NewHomework, NewHomeworkVersion, NewOperation,
        NewSubject, NewWeek, OperationRecord, Subject, Week,
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
        self.database.with_transaction(|transaction| {
            insert_subject(transaction, &subject)?;
            Ok(Subject {
                id: subject.id.to_owned(),
                name: subject.name.to_owned(),
                folder_path: subject.folder_path.to_owned(),
            })
        })
    }

    pub fn get_subject(&self, id: &str) -> Result<Subject, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, name, folder_path FROM subjects WHERE id = ?1",
                    [id],
                    |row| {
                        Ok(Subject {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            folder_path: row.get(2)?,
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
                folder_path: week.folder_path.to_owned(),
            })
        })
    }

    pub fn get_week(&self, id: &str) -> Result<Week, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, subject_id, number, folder_path FROM weeks WHERE id = ?1",
                    [id],
                    |row| {
                        Ok(Week {
                            id: row.get(0)?,
                            subject_id: row.get(1)?,
                            number: row.get(2)?,
                            folder_path: row.get(3)?,
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
                    "SELECT id, week_id, title, stage, workflow_stage, folder_path FROM homeworks WHERE id = ?1",
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
            Ok(version_from_new(&version))
        })
    }

    pub fn list_homework_versions(
        &self,
        homework_id: &str,
    ) -> Result<Vec<HomeworkVersion>, BackendError> {
        self.database.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, homework_id, number, label, file_path, author, frozen
                 FROM homework_versions WHERE homework_id = ?1 ORDER BY number",
            )?;
            let rows = statement.query_map([homework_id], |row| {
                Ok(HomeworkVersion {
                    id: row.get(0)?,
                    homework_id: row.get(1)?,
                    number: row.get(2)?,
                    label: row.get(3)?,
                    file_path: row.get(4)?,
                    author: row.get(5)?,
                    frozen: row.get::<_, i64>(6)? == 1,
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
            Ok((homework_from_new(&homework)?, version_from_new(&version)))
        })
    }

    pub fn record_operation(
        &self,
        operation: NewOperation<'_>,
    ) -> Result<OperationRecord, BackendError> {
        self.database.with_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO operations(id, kind, status, undoable) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        operation.id,
                        operation.kind,
                        operation.status,
                        i64::from(operation.undoable)
                    ],
                )
                .map_err(|error| {
                    map_write_error(error, "operation already exists or is invalid")
                })?;
            Ok(OperationRecord {
                id: operation.id.to_owned(),
                kind: operation.kind.to_owned(),
                status: operation.status.to_owned(),
                undoable: operation.undoable,
            })
        })
    }
}

fn insert_subject(connection: &Connection, subject: &NewSubject<'_>) -> Result<(), BackendError> {
    connection
        .execute(
            "INSERT INTO subjects(id, name, folder_path) VALUES (?1, ?2, ?3)",
            params![subject.id, subject.name, subject.folder_path],
        )
        .map_err(|error| map_write_error(error, "subject already exists or is invalid"))?;
    Ok(())
}

fn insert_week(connection: &Connection, week: &NewWeek<'_>) -> Result<(), BackendError> {
    connection
        .execute(
            "INSERT INTO weeks(id, subject_id, number, folder_path) VALUES (?1, ?2, ?3, ?4)",
            params![week.id, week.subject_id, week.number, week.folder_path],
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
            "INSERT INTO homeworks(id, week_id, title, stage, workflow_stage, folder_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                homework.id,
                homework.week_id,
                homework.title,
                homework.stage.as_str(),
                homework.workflow_stage,
                homework.folder_path
            ],
        )
        .map_err(|error| map_write_error(error, "homework already exists or is invalid"))?;
    Ok(())
}

fn insert_homework_version(
    connection: &Connection,
    version: &NewHomeworkVersion<'_>,
) -> Result<(), BackendError> {
    connection
        .execute(
            "INSERT INTO homework_versions(id, homework_id, number, label, file_path, author)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                version.id,
                version.homework_id,
                version.number,
                version.label,
                version.file_path,
                version.author
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
        stage: homework.stage,
        workflow_stage: homework.workflow_stage.to_owned(),
        folder_path: homework.folder_path.to_owned(),
    })
}

fn version_from_new(version: &NewHomeworkVersion<'_>) -> HomeworkVersion {
    HomeworkVersion {
        id: version.id.to_owned(),
        homework_id: version.homework_id.to_owned(),
        number: version.number,
        label: version.label.to_owned(),
        file_path: version.file_path.to_owned(),
        author: version.author.to_owned(),
        frozen: true,
    }
}

fn homework_from_row(row: &Row<'_>) -> rusqlite::Result<Homework> {
    let raw_stage: String = row.get(3)?;
    let stage = HomeworkStage::try_from(raw_stage.as_str()).map_err(|message| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                message,
            )),
        )
    })?;
    Ok(Homework {
        id: row.get(0)?,
        week_id: row.get(1)?,
        title: row.get(2)?,
        stage,
        workflow_stage: row.get(4)?,
        folder_path: row.get(5)?,
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
                folder_path: "Предметы/ОБА",
            })
            .unwrap();
        repositories
            .create_week(NewWeek {
                id: "week-1",
                subject_id: "subject-1",
                number: 7,
                folder_path: "Предметы/ОБА/Неделя 07",
            })
            .unwrap();
    }

    fn homework<'a>(id: &'a str) -> NewHomework<'a> {
        NewHomework {
            id,
            week_id: "week-1",
            title: "Домашняя работа",
            stage: HomeworkStage::InProgress,
            workflow_stage: "solution",
            folder_path: "Предметы/ОБА/Неделя 07/03 ДЗ",
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
    fn creates_and_reads_subject() {
        let database = database();
        let repositories = Repositories::new(&database);
        repositories
            .create_subject(NewSubject {
                id: "subject-1",
                name: "ОБА",
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
        repositories
            .add_homework_version(NewHomeworkVersion {
                id: "version-1",
                homework_id: "homework-1",
                number: 1,
                label: "v01",
                file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md",
                author: "claude",
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
                    file_path: if number == 1 {
                        "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md"
                    } else {
                        "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v02.md"
                    },
                    author: "claude",
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
        let first_version = NewHomeworkVersion {
            id: "version-1",
            homework_id: "homework-1",
            number: 1,
            label: "v01",
            file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v01.md",
            author: "claude",
        };
        repositories.add_homework_version(first_version).unwrap();
        let error = repositories
            .add_homework_version(NewHomeworkVersion {
                id: "version-another-id",
                homework_id: "homework-1",
                number: 1,
                label: "v01-alternative",
                file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/alternative.md",
                author: "codex",
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
    fn transaction_rolls_back_partial_chain() {
        let database = database();
        seed_subject_week(&database);
        let repositories = Repositories::new(&database);
        let error = repositories
            .create_homework_with_initial_version(
                homework("homework-1"),
                NewHomeworkVersion {
                    id: "version-1",
                    homework_id: "homework-1",
                    number: 0,
                    label: "v00",
                    file_path: "Предметы/ОБА/Неделя 07/03 ДЗ/02 Работа/v00.md",
                    author: "claude",
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
        submitted.workflow_stage = "submitted";
        database
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO homeworks(id, week_id, title, stage, workflow_stage, folder_path, submitted_at)
                     VALUES (?1, ?2, ?3, 'submitted', 'submitted', ?4, CURRENT_TIMESTAMP)",
                    params![submitted.id, submitted.week_id, submitted.title, submitted.folder_path],
                )?;
                Ok(())
            })
            .unwrap();
        let error = repositories
            .update_homework_title("homework-1", "Changed")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Immutable);
    }

    #[test]
    fn operation_is_recorded() {
        let database = database();
        let repositories = Repositories::new(&database);
        let operation = repositories
            .record_operation(NewOperation {
                id: "operation-1",
                kind: "route_files",
                status: "started",
                undoable: true,
            })
            .unwrap();
        assert!(operation.undoable);
        assert_eq!(operation.status, "started");
    }

    #[test]
    fn rejects_absolute_persistent_paths() {
        let database = database();
        let error = Repositories::new(&database)
            .create_subject(NewSubject {
                id: "subject-1",
                name: "ОБА",
                folder_path: "/Users/example/Предметы/ОБА",
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
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
