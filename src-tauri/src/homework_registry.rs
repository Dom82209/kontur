//! Production workflow, этап 4: у недели 0..N домашних заданий.
//!
//! Новое ДЗ недели создаётся только в базе: порядковый номер — следующий после
//! существующих, папка — `03 ДЗ` для первого и `03 ДЗ (N)` для следующих. В vault
//! ничего не пишется; папка появится при раскладке файлов или создании версии.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::Database,
    dto::HomeworkDto,
    error::{AppError, BackendError},
    file_service::PersistentPath,
    repositories::ReadRepository,
};

const MAX_TITLE_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewHomeworkInput {
    pub subject_id: String,
    pub week_number: i64,
    pub title: String,
    /// RFC 3339; `None` — срок неизвестен.
    pub deadline: Option<String>,
}

/// Папка ДЗ по порядковому номеру внутри папки недели.
pub fn homework_folder(week_folder: &str, ordinal: i64) -> String {
    if ordinal <= 1 {
        format!("{week_folder}/03 ДЗ")
    } else {
        format!("{week_folder}/03 ДЗ ({ordinal})")
    }
}

/// Нормализует срок в UTC RFC 3339; пустая строка или `None` — срок неизвестен.
pub fn normalize_deadline(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed =
        time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339)
            .map_err(|_| BackendError::blocked("Срок должен быть датой и временем (RFC 3339)"))?;
    let utc = parsed.to_offset(time::UtcOffset::UTC);
    Ok(Some(
        utc.format(&time::format_description::well_known::Rfc3339)
            .map_err(|error| BackendError::database(error.to_string()))?,
    ))
}

pub struct HomeworkRegistry<'a> {
    database: &'a Database,
}

impl<'a> HomeworkRegistry<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn create(&self, input: &NewHomeworkInput) -> Result<HomeworkDto, AppError> {
        let title = input.title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
            return Err(BackendError::blocked(format!(
                "Название ДЗ: от 1 до {MAX_TITLE_CHARS} символов"
            )));
        }
        let deadline = normalize_deadline(input.deadline.as_deref())?;
        let id = Uuid::new_v4().to_string();
        self.database.with_transaction(|transaction| {
            let subject: Option<bool> = transaction
                .query_row(
                    "SELECT archived_at IS NOT NULL FROM subjects WHERE id=?1",
                    [&input.subject_id],
                    |row| row.get(0),
                )
                .optional()?;
            match subject {
                None => return Err(BackendError::not_found("Предмет не найден")),
                Some(true) => {
                    return Err(BackendError::blocked(
                        "Предмет в архиве: верните его из архива, чтобы добавить ДЗ",
                    ))
                }
                Some(false) => {}
            }
            let (week_id, week_folder): (String, String) = transaction
                .query_row(
                    "SELECT id, folder_path FROM weeks WHERE subject_id=?1 AND number=?2",
                    params![input.subject_id, input.week_number],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| BackendError::not_found("Неделя не найдена: сначала создайте неделю"))?;
            let ordinal: i64 = transaction.query_row(
                "SELECT coalesce(max(ordinal), 0) + 1 FROM homeworks WHERE week_id=?1",
                [&week_id],
                |row| row.get(0),
            )?;
            let folder = homework_folder(&week_folder, ordinal);
            PersistentPath::new(&folder)?;
            let taken: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM homeworks WHERE lower(folder_path)=lower(?1))",
                [&folder],
                |row| row.get(0),
            )?;
            if taken {
                return Err(BackendError::conflict("Папка нового ДЗ уже занята"));
            }
            transaction.execute(
                "INSERT INTO homeworks(id,week_id,ordinal,title,deadline,deadline_source,stage,workflow_stage,folder_path)
                 VALUES (?1,?2,?3,?4,?5,CASE WHEN ?5 IS NULL THEN NULL ELSE 'manual' END,'task','materials',?6)",
                params![id, week_id, ordinal, title, deadline, folder],
            )?;
            Ok(())
        })?;
        ReadRepository::new(self.database).homework(&id)
    }
}

#[cfg(test)]
#[path = "homework_registry_tests.rs"]
mod tests;
