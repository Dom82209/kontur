//! SQL-граница этапа 6: настройка watcher, уже обработанные загрузки и
//! ссылки LMS. Только параметризованные запросы; абсолютные пути не хранятся.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    db::Database,
    dto::ProvenanceDto,
    error::{AppError, BackendError},
};

pub const WATCHER_SETTINGS_KEY: &str = "downloads_watcher";

/// Настройка watcher. По умолчанию выключен.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatcherSettings {
    pub enabled: bool,
}

pub fn read_watcher_settings(database: &Database) -> Result<WatcherSettings, AppError> {
    database.with_connection(|connection| {
        let raw: Option<String> = connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                [WATCHER_SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            None => Ok(WatcherSettings::default()),
            Some(raw) => serde_json::from_str(&raw).map_err(|_| {
                BackendError::database("Настройка наблюдения за загрузками повреждена")
            }),
        }
    })
}

pub fn write_watcher_settings(
    database: &Database,
    settings: WatcherSettings,
) -> Result<(), AppError> {
    let value_json = serde_json::to_string(&settings)
        .map_err(|error| BackendError::database(error.to_string()))?;
    database.with_transaction(|transaction| {
        transaction.execute(
            "INSERT INTO app_settings(key, value_json, created_at, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET
               value_json = excluded.value_json,
               updated_at = CURRENT_TIMESTAMP",
            params![WATCHER_SETTINGS_KEY, value_json],
        )?;
        Ok(())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeenOutcome {
    Imported,
    Dismissed,
}

/// Файл с этим содержимым уже импортирован, лежит во «Входящих» или скрыт пользователем.
pub fn is_known_content(database: &Database, sha256: &str) -> Result<bool, AppError> {
    database.with_connection(|connection| {
        Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM download_seen WHERE sha256 = ?1)
                 OR EXISTS(SELECT 1 FROM inbox_files WHERE sha256 = ?1)",
            [sha256],
            |row| row.get(0),
        )?)
    })
}

pub fn mark_seen(
    database: &Database,
    sha256: &str,
    name: &str,
    size_bytes: u64,
    outcome: SeenOutcome,
    operation_id: Option<&str>,
) -> Result<(), AppError> {
    let size = i64::try_from(size_bytes).unwrap_or(i64::MAX);
    let name: String = name
        .chars()
        .map(|ch| if matches!(ch, '/' | '\\') { '-' } else { ch })
        .take(255)
        .collect();
    let outcome = match outcome {
        SeenOutcome::Imported => "imported",
        SeenOutcome::Dismissed => "dismissed",
    };
    database.with_transaction(|transaction| {
        transaction.execute(
            "INSERT INTO download_seen(sha256, name, size_bytes, outcome, operation_id, first_seen_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(sha256) DO UPDATE SET
               outcome = excluded.outcome,
               operation_id = coalesce(excluded.operation_id, download_seen.operation_id),
               updated_at = excluded.updated_at",
            params![sha256, name, size, outcome, operation_id],
        )?;
        Ok(())
    })
}

pub fn subject_exists(database: &Database, subject_id: &str) -> Result<bool, AppError> {
    database.with_connection(|connection| {
        Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM subjects WHERE id = ?1)",
            [subject_id],
            |row| row.get(0),
        )?)
    })
}

pub fn set_course_url(
    database: &Database,
    subject_id: &str,
    url: Option<&str>,
) -> Result<(), AppError> {
    database.with_transaction(|transaction| {
        let changed = transaction.execute(
            "UPDATE subjects SET course_url = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![subject_id, url],
        )?;
        if changed == 0 {
            return Err(BackendError::not_found("Предмет не найден"));
        }
        Ok(())
    })
}

pub fn course_url(database: &Database, subject_id: &str) -> Result<Option<String>, AppError> {
    database.with_connection(|connection| {
        connection
            .query_row(
                "SELECT course_url FROM subjects WHERE id = ?1",
                [subject_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| BackendError::not_found("Предмет не найден"))
    })
}

pub fn set_assignment_url(
    database: &Database,
    homework_id: &str,
    url: Option<&str>,
) -> Result<(), AppError> {
    let result = database.with_transaction(|transaction| {
        let stage: Option<String> = transaction
            .query_row(
                "SELECT stage FROM homeworks WHERE id = ?1",
                [homework_id],
                |row| row.get(0),
            )
            .optional()?;
        match stage.as_deref() {
            None => return Err(BackendError::not_found("Домашнее задание не найдено")),
            Some("submitted") => {
                return Err(BackendError::immutable(
                    "Отправленную работу изменить нельзя",
                ))
            }
            Some(_) => {}
        }
        transaction.execute(
            "UPDATE homeworks SET assignment_url = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![homework_id, url],
        )?;
        Ok(())
    });
    result
}

pub fn assignment_url(database: &Database, homework_id: &str) -> Result<Option<String>, AppError> {
    database.with_connection(|connection| {
        connection
            .query_row(
                "SELECT assignment_url FROM homeworks WHERE id = ?1",
                [homework_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| BackendError::not_found("Домашнее задание не найдено"))
    })
}

pub fn inbox_provenance(
    database: &Database,
    file_id: &str,
) -> Result<Option<ProvenanceDto>, AppError> {
    let raw: Option<String> = database.with_connection(|connection| {
        connection
            .query_row(
                "SELECT provenance_json FROM inbox_files WHERE id = ?1",
                [file_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or_else(|| BackendError::not_found("Файл во «Входящих» не найден"))
    })?;
    raw.map(|raw| {
        serde_json::from_str(&raw)
            .map_err(|_| BackendError::database("Происхождение файла повреждено"))
    })
    .transpose()
}
