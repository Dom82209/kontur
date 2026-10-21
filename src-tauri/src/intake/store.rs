//! SQL-граница импорта: параметризованные запросы к `inbox_files`,
//! `operations`, `operation_results` и `app_settings`.

use rusqlite::{params, OptionalExtension};

use crate::{
    db::Database,
    dto::{DuplicateInfoDto, FileKind, OperationKind},
    error::{AppError, BackendError, ErrorCode},
    repositories::writes,
};

use super::roots::ManagedRootsConfig;

pub const MANAGED_ROOTS_KEY: &str = "managed_roots";
pub const RESULT_KIND_INBOX_IMPORT: &str = "inbox_import";

pub struct NewInboxRow<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub kind: FileKind,
    pub size_bytes: u64,
    pub relative_path: &'a str,
    pub sha256: &'a str,
    pub mime_type: &'a str,
    pub operation_id: &'a str,
    pub duplicate: Option<&'a DuplicateInfoDto>,
}

pub struct StoredInboxRow {
    pub id: String,
    pub relative_path: String,
}

pub fn read_managed_roots(database: &Database) -> Result<ManagedRootsConfig, AppError> {
    database.with_connection(|connection| {
        let raw: Option<String> = connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                [MANAGED_ROOTS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            None => Ok(ManagedRootsConfig::default()),
            Some(raw) => serde_json::from_str(&raw)
                .map_err(|_| BackendError::database("Настройки управляемых каталогов повреждены")),
        }
    })
}

pub fn write_managed_roots(
    database: &Database,
    config: &ManagedRootsConfig,
) -> Result<(), AppError> {
    let value_json =
        serde_json::to_string(config).map_err(|error| BackendError::database(error.to_string()))?;
    database.with_transaction(|transaction| {
        transaction.execute(
            "INSERT INTO app_settings(key, value_json, created_at, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET
               value_json = excluded.value_json,
               updated_at = CURRENT_TIMESTAMP",
            params![MANAGED_ROOTS_KEY, value_json],
        )?;
        Ok(())
    })
}

pub fn active_inbox_by_sha(
    database: &Database,
    sha256: &str,
) -> Result<Option<StoredInboxRow>, AppError> {
    database.with_connection(|connection| {
        Ok(connection
            .query_row(
                "SELECT id, relative_path FROM inbox_files
                 WHERE sha256 = ?1 AND status <> 'routed'",
                [sha256],
                |row| {
                    Ok(StoredInboxRow {
                        id: row.get(0)?,
                        relative_path: row.get(1)?,
                    })
                },
            )
            .optional()?)
    })
}

pub fn inbox_by_path(
    database: &Database,
    relative_path: &str,
) -> Result<Option<(String, Option<String>)>, AppError> {
    database.with_connection(|connection| {
        Ok(connection
            .query_row(
                "SELECT id, sha256 FROM inbox_files WHERE relative_path = ?1",
                [relative_path],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    })
}

pub fn inbox_relative_path(database: &Database, file_id: &str) -> Result<String, AppError> {
    database.with_connection(|connection| {
        connection
            .query_row(
                "SELECT relative_path FROM inbox_files WHERE id = ?1",
                [file_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| BackendError::not_found("Файл во «Входящих» не найден"))
    })
}

pub fn insert_inbox_file(database: &Database, row: &NewInboxRow<'_>) -> Result<(), AppError> {
    let duplicate_json = row
        .duplicate
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| BackendError::database(error.to_string()))?;
    let size = i64::try_from(row.size_bytes)
        .map_err(|_| BackendError::blocked("Файл слишком большой для импорта"))?;
    database.with_transaction(|transaction| {
        transaction.execute(
            "INSERT INTO inbox_files(
               id, name, kind, size_bytes, added_at, origin, status, relative_path,
               classification_json, duplicate_json, sha256, mime_type, import_operation_id
             ) VALUES (
               ?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'drop', 'new', ?5,
               NULL, ?6, ?7, ?8, ?9
             )",
            params![
                row.id,
                row.name,
                file_kind_str(row.kind),
                size,
                row.relative_path,
                duplicate_json,
                row.sha256,
                row.mime_type,
                row.operation_id,
            ],
        )?;
        Ok(())
    })
}

/// Состояние операции с данным id: отсутствует, есть итог (можно повторить ответ) или
/// существует без итога (выполняется или прервана).
pub enum OperationLookup {
    Missing,
    Finished(String),
    Unfinished,
}

pub fn lookup_operation(
    database: &Database,
    operation_id: &str,
) -> Result<OperationLookup, AppError> {
    database.with_connection(|connection| {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE id = ?1)",
            [operation_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(OperationLookup::Missing);
        }
        let result: Option<String> = connection
            .query_row(
                "SELECT result_json FROM operation_results WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match result {
            Some(json) => OperationLookup::Finished(json),
            None => OperationLookup::Unfinished,
        })
    })
}

pub fn start_import_operation(
    database: &Database,
    operation_id: &str,
    title: &str,
    detail: &str,
) -> Result<(), AppError> {
    database.with_transaction(|transaction| {
        writes::insert_running_operation(transaction, operation_id, OperationKind::Import, title)?;
        transaction.execute(
            "UPDATE operations SET detail = ?2 WHERE id = ?1",
            params![operation_id, detail],
        )?;
        Ok(())
    })
}

pub fn update_operation_progress(
    database: &Database,
    operation_id: &str,
    progress: f64,
) -> Result<(), AppError> {
    let percent = (progress.clamp(0.0, 1.0) * 100.0).round() as i64;
    database.with_connection(|connection| {
        connection.execute(
            "UPDATE operations SET progress = ?2 WHERE id = ?1 AND status = 'running'",
            params![operation_id, percent],
        )?;
        Ok(())
    })
}

pub struct FinishImport<'a> {
    pub operation_id: &'a str,
    pub failed: bool,
    pub detail: &'a str,
    pub paths: &'a [String],
    pub error: Option<&'a AppError>,
    pub result_json: &'a str,
}

/// Атомарно закрывает операцию и сохраняет её итог.
pub fn finish_import_operation(
    database: &Database,
    finish: &FinishImport<'_>,
) -> Result<(), AppError> {
    let paths_json = serde_json::to_string(finish.paths)
        .map_err(|error| BackendError::database(error.to_string()))?;
    let error_json = finish
        .error
        .map(|error| {
            serde_json::to_string(&SafeError {
                code: error.code,
                message: "Ни один файл не импортирован",
            })
        })
        .transpose()
        .map_err(|error| BackendError::database(error.to_string()))?;
    database.with_transaction(|transaction| {
        let changed = transaction.execute(
            "UPDATE operations
             SET status = ?2, progress = 100, detail = ?3, paths_json = ?4, error_json = ?5,
                 undoable = 0, completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND status = 'running'",
            params![
                finish.operation_id,
                if finish.failed { "failed" } else { "completed" },
                finish.detail,
                paths_json,
                error_json,
            ],
        )?;
        if changed != 1 {
            return Err(BackendError::database(
                "Операция импорта не может быть завершена повторно",
            ));
        }
        transaction.execute(
            "INSERT INTO operation_results(operation_id, kind, result_json) VALUES (?1, ?2, ?3)",
            params![
                finish.operation_id,
                RESULT_KIND_INBOX_IMPORT,
                finish.result_json
            ],
        )?;
        Ok(())
    })
}

pub fn operation_result(
    database: &Database,
    operation_id: &str,
) -> Result<Option<(String, String)>, AppError> {
    database.with_connection(|connection| {
        Ok(connection
            .query_row(
                "SELECT kind, result_json FROM operation_results WHERE operation_id = ?1",
                [operation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    })
}

#[derive(serde::Serialize)]
struct SafeError<'a> {
    code: ErrorCode,
    message: &'a str,
}

fn file_kind_str(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Pdf => "pdf",
        FileKind::Doc => "doc",
        FileKind::Sheet => "sheet",
        FileKind::Slides => "slides",
        FileKind::Image => "image",
        FileKind::Notebook => "notebook",
        FileKind::Text => "text",
        FileKind::Archive => "archive",
        FileKind::Other => "other",
    }
}
