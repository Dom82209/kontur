//! Этап 7: отпечаток входных данных ДЗ.
//!
//! Проверка, промт исправлений, готовность и комплект запоминают отпечаток
//! входных данных, на которых созданы. Любое изменение материалов, условия,
//! выбранной версии, профиля стиля или официальных требований меняет текущий
//! отпечаток — и все ранее созданные результаты становятся устаревшими (stale).
//! Устаревшие записи не удаляются: они остаются в истории, но не действуют.
//!
//! Отпечаток вычисляется из SQLite при каждом чтении, поэтому его нельзя
//! «забыть» обновить в каком-то пути изменения данных.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

/// Отпечатки частей входных данных и общий отпечаток (SHA-256, hex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputsFingerprintDto {
    pub materials: String,
    pub task: String,
    pub version: String,
    pub style: String,
    pub requirements: String,
    pub combined: String,
}

impl InputsFingerprintDto {
    /// Результат с этим сохранённым отпечатком действует (не устарел).
    pub fn is_current(&self, stored: Option<&str>) -> bool {
        stored == Some(self.combined.as_str())
    }
}

fn digest(rows: &[String]) -> String {
    let mut hasher = Sha256::new();
    for row in rows {
        hasher.update((row.len() as u64).to_le_bytes());
        hasher.update(row.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn collect(connection: &Connection, sql: &str, homework_id: &str) -> Result<Vec<String>, AppError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement
        .query_map(params![homework_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Текущий отпечаток входных данных ДЗ.
pub fn current(
    connection: &Connection,
    homework_id: &str,
) -> Result<InputsFingerprintDto, AppError> {
    // Материалы недели и ДЗ (без условия, черновиков и итоговых файлов).
    let materials = collect(
        connection,
        "SELECT f.id || '|' || f.relative_path || '|' || coalesce(f.content_hash, '') || '|' ||
                coalesce(f.size_bytes, '') || '|' || f.status || '|' || f.category
           FROM files f JOIN homeworks h ON h.id = ?1
          WHERE (f.homework_id = h.id OR (f.homework_id IS NULL AND f.week_id = h.week_id))
            AND f.category IN ('lecture', 'seminar', 'material', 'homework_data')
          ORDER BY f.id",
        homework_id,
    )?;
    // Условие задания: файлы условия и ссылка на задание.
    let mut task = collect(
        connection,
        "SELECT f.id || '|' || f.relative_path || '|' || coalesce(f.content_hash, '') || '|' ||
                coalesce(f.size_bytes, '') || '|' || f.status
           FROM files f JOIN homeworks h ON h.id = ?1
          WHERE (f.homework_id = h.id OR (f.homework_id IS NULL AND f.week_id = h.week_id))
            AND f.category = 'homework_task'
          ORDER BY f.id",
        homework_id,
    )?;
    task.extend(collect(
        connection,
        "SELECT 'url|' || coalesce(assignment_url, '') FROM homeworks WHERE id = ?1",
        homework_id,
    )?);
    // Выбранная версия и её состав.
    let version = collect(
        connection,
        "SELECT coalesce(v.id, '-') || '|' || coalesce(v.file_path, '') || '|' ||
                coalesce(v.size_bytes, '') || '|' || coalesce(v.files_json, '')
           FROM homeworks h LEFT JOIN homework_versions v ON v.id = h.final_version_id
          WHERE h.id = ?1",
        homework_id,
    )?;
    // Профиль стиля: явно выбранный источник и его черты.
    // Профиль стиля общий для пространства (этап 9): последняя ревизия и
    // текущее состояние подтверждённых источников.
    let style = {
        let mut statement = connection.prepare(
            "SELECT 'rev|' || id || '|' || source_fingerprint || '|' || language
               FROM style_profile_revisions ORDER BY revision DESC LIMIT 1",
        )?;
        let mut rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut sources = connection.prepare(
            "SELECT 'src|' || id || '|' || coalesce(sha256, '') || '|' || read_status || '|' ||
                    confirmation || '|' || language
               FROM style_sources WHERE removed_at IS NULL ORDER BY id",
        )?;
        rows.extend(
            sources
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?,
        );
        rows
    };
    // Официальные комментарии и действующие требования.
    let mut requirements = collect(
        connection,
        "SELECT 'c|' || id || '|' || authority || '|' || question || '|' || answer
           FROM teacher_comments
          WHERE homework_id = ?1 AND official_confirmed_at IS NOT NULL
          ORDER BY id",
        homework_id,
    )?;
    requirements.extend(collect(
        connection,
        "SELECT 'r|' || id || '|' || coalesce(comment_id, '') || '|' || text
           FROM teacher_requirements
          WHERE homework_id = ?1 AND withdrawn_at IS NULL
          ORDER BY id",
        homework_id,
    )?);

    let parts = [
        digest(&materials),
        digest(&task),
        digest(&version),
        digest(&style),
        digest(&requirements),
    ];
    let combined = digest(&parts);
    let [materials, task, version, style, requirements] = parts;
    Ok(InputsFingerprintDto {
        materials,
        task,
        version,
        style,
        requirements,
        combined,
    })
}
