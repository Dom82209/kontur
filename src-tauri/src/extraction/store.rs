//! SQL этапа 7: cache результатов, связи с материалами, итоги пакетов,
//! подборки материалов и настройки OCR.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::Database,
    dto::{FileKind, OperationKind},
    error::{AppError, BackendError, ErrorCode},
    repositories::writes,
};

use super::{
    dto::{MaterialKind, MaterialRef},
    model::{
        Block, Extracted, ExtractionMetadata, ExtractionStatus, ExtractionWarning, SourceFormat,
        EXTRACTOR_VERSION,
    },
};

const SETTINGS_KEY: &str = "extraction";

/// Сохранённый результат извлечения.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredExtraction {
    pub id: String,
    pub sha256: String,
    pub extractor_version: u32,
    pub options_key: String,
    pub extracted: Extracted,
    pub source_size_bytes: u64,
    pub duration_ms: u64,
    pub created_at: String,
}

/// Материал, из которого извлекается текст (запись «Входящих» или файл недели/ДЗ).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialInfo {
    pub material: MaterialRef,
    pub name: String,
    pub kind: FileKind,
    pub size_bytes: u64,
    /// Путь относительно корня хранилища. Не выдаётся во frontend.
    pub relative_path: String,
    /// SHA-256 из журнала импорта (для «Входящих»); для файлов недели — `None`.
    pub sha256: Option<String>,
    pub context: String,
}

fn db_error(error: impl std::fmt::Display) -> AppError {
    BackendError::database(error.to_string())
}

fn kind_of(raw: &str) -> FileKind {
    FileKind::try_from_db(raw).unwrap_or(FileKind::Other)
}

pub fn inbox_material(database: &Database, id: &str) -> Result<Option<MaterialInfo>, AppError> {
    database.with_connection(|connection| {
        Ok(connection
            .query_row(
                "SELECT id, name, kind, size_bytes, relative_path, sha256
                 FROM inbox_files WHERE id = ?1",
                [id],
                inbox_row,
            )
            .optional()?)
    })
}

fn inbox_row(row: &Row<'_>) -> rusqlite::Result<MaterialInfo> {
    let kind: String = row.get(2)?;
    let size: i64 = row.get(3)?;
    Ok(MaterialInfo {
        material: MaterialRef {
            kind: MaterialKind::Inbox,
            id: row.get(0)?,
        },
        name: row.get(1)?,
        kind: kind_of(&kind),
        size_bytes: size.max(0) as u64,
        relative_path: row.get(4)?,
        sha256: row.get(5)?,
        context: "Входящие".into(),
    })
}

const FILE_SELECT: &str = "SELECT f.id, f.name, f.kind, f.size_bytes, f.relative_path,
            COALESCE(s.short_name, hs.short_name), COALESCE(w.number, hw.number), f.category
     FROM files f
     LEFT JOIN weeks w ON w.id = f.week_id
     LEFT JOIN subjects s ON s.id = w.subject_id
     LEFT JOIN homeworks h ON h.id = f.homework_id
     LEFT JOIN weeks hw ON hw.id = h.week_id
     LEFT JOIN subjects hs ON hs.id = hw.subject_id";

fn file_row(row: &Row<'_>) -> rusqlite::Result<MaterialInfo> {
    let kind: String = row.get(2)?;
    let size: i64 = row.get(3)?;
    let subject: Option<String> = row.get(5)?;
    let week: Option<i64> = row.get(6)?;
    let category: String = row.get(7)?;
    let category = match category.as_str() {
        "lecture" => "Лекции",
        "seminar" => "Семинары",
        "material" => "Дополнительно",
        "homework_task" | "homework_data" => "ДЗ · условие и данные",
        "homework_draft" => "ДЗ · работа",
        "homework_final" => "ДЗ · на сдачу",
        _ => "Материал",
    };
    let mut context = Vec::new();
    if let Some(subject) = subject {
        context.push(subject);
    }
    if let Some(week) = week {
        context.push(format!("Неделя {week}"));
    }
    context.push(category.to_owned());
    Ok(MaterialInfo {
        material: MaterialRef {
            kind: MaterialKind::File,
            id: row.get(0)?,
        },
        name: row.get(1)?,
        kind: kind_of(&kind),
        size_bytes: size.max(0) as u64,
        relative_path: row.get(4)?,
        sha256: None,
        context: context.join(" · "),
    })
}

pub fn file_material(database: &Database, id: &str) -> Result<Option<MaterialInfo>, AppError> {
    database.with_connection(|connection| {
        Ok(connection
            .query_row(&format!("{FILE_SELECT} WHERE f.id = ?1"), [id], file_row)
            .optional()?)
    })
}

pub fn material(
    database: &Database,
    material: &MaterialRef,
) -> Result<Option<MaterialInfo>, AppError> {
    match material.kind {
        MaterialKind::Inbox => inbox_material(database, &material.id),
        MaterialKind::File => file_material(database, &material.id),
    }
}

/// Все материалы: «Входящие» (новые сверху) и файлы недель/ДЗ.
pub fn all_materials(database: &Database) -> Result<Vec<MaterialInfo>, AppError> {
    database.with_connection(|connection| {
        let mut materials = Vec::new();
        let mut statement = connection.prepare(
            "SELECT id, name, kind, size_bytes, relative_path, sha256
             FROM inbox_files ORDER BY added_at DESC, id",
        )?;
        for row in statement.query_map([], inbox_row)? {
            materials.push(row?);
        }
        let mut statement = connection.prepare(&format!(
            "{FILE_SELECT} WHERE f.status = 'available' ORDER BY f.relative_path"
        ))?;
        for row in statement.query_map([], file_row)? {
            materials.push(row?);
        }
        Ok(materials)
    })
}

// ───────────────────────── Cache ─────────────────────────

const EXTRACTION_COLUMNS: &str = "id, sha256, extractor_version, options_key, format, status,
    status_message, metadata_json, structure_json, blocks_truncated, text, text_chars,
    truncated, warnings_json, ocr_used, source_size_bytes, duration_ms, created_at";

fn extraction_row(row: &Row<'_>) -> rusqlite::Result<Result<StoredExtraction, AppError>> {
    let format: String = row.get(4)?;
    let status: String = row.get(5)?;
    let metadata: String = row.get(7)?;
    let structure: String = row.get(8)?;
    let warnings: String = row.get(13)?;
    let version: i64 = row.get(2)?;
    let text_chars: i64 = row.get(11)?;
    let size: i64 = row.get(15)?;
    let duration: i64 = row.get(16)?;
    let id: String = row.get(0)?;
    let sha256: String = row.get(1)?;
    let options_key: String = row.get(3)?;
    let status_message: Option<String> = row.get(6)?;
    let blocks_truncated: i64 = row.get(9)?;
    let text: String = row.get(10)?;
    let truncated: i64 = row.get(12)?;
    let ocr_used: i64 = row.get(14)?;
    let created_at: String = row.get(17)?;
    Ok((|| {
        let format = SourceFormat::try_from_db(&format)
            .ok_or_else(|| BackendError::database("Неизвестный формат извлечения"))?;
        let status = ExtractionStatus::try_from_db(&status)
            .ok_or_else(|| BackendError::database("Неизвестный статус извлечения"))?;
        let metadata: ExtractionMetadata = serde_json::from_str(&metadata).map_err(db_error)?;
        let blocks: Vec<Block> = serde_json::from_str(&structure).map_err(db_error)?;
        let warnings: Vec<ExtractionWarning> = serde_json::from_str(&warnings).map_err(db_error)?;
        Ok(StoredExtraction {
            id,
            sha256,
            extractor_version: version.max(0) as u32,
            options_key,
            extracted: Extracted {
                format,
                status,
                status_message,
                metadata,
                blocks,
                blocks_truncated: blocks_truncated != 0,
                text,
                text_chars: text_chars.max(0) as usize,
                truncated: truncated != 0,
                warnings,
                ocr_used: ocr_used != 0,
            },
            source_size_bytes: size.max(0) as u64,
            duration_ms: duration.max(0) as u64,
            created_at,
        })
    })())
}

pub fn find_cached(
    database: &Database,
    sha256: &str,
    options_key: &str,
) -> Result<Option<StoredExtraction>, AppError> {
    database.with_connection(|connection| {
        connection
            .query_row(
                &format!(
                    "SELECT {EXTRACTION_COLUMNS} FROM extractions
                     WHERE sha256 = ?1 AND extractor_version = ?2 AND options_key = ?3"
                ),
                params![sha256, EXTRACTOR_VERSION, options_key],
                extraction_row,
            )
            .optional()?
            .transpose()
    })
}

pub fn extraction_by_id(
    database: &Database,
    id: &str,
) -> Result<Option<StoredExtraction>, AppError> {
    database.with_connection(|connection| {
        connection
            .query_row(
                &format!("SELECT {EXTRACTION_COLUMNS} FROM extractions WHERE id = ?1"),
                [id],
                extraction_row,
            )
            .optional()?
            .transpose()
    })
}

pub struct NewExtraction<'a> {
    pub sha256: &'a str,
    pub options_key: &'a str,
    pub extracted: &'a Extracted,
    pub source_size_bytes: u64,
    pub duration_ms: u64,
}

/// Сохраняет результат (перестроение заменяет прежний под тем же ключом) и
/// привязывает его к материалу — одной транзакцией.
pub fn save_extraction(
    database: &Database,
    material: &MaterialRef,
    row: &NewExtraction<'_>,
) -> Result<StoredExtraction, AppError> {
    let metadata = serde_json::to_string(&row.extracted.metadata).map_err(db_error)?;
    let structure = serde_json::to_string(&row.extracted.blocks).map_err(db_error)?;
    let warnings = serde_json::to_string(&row.extracted.warnings).map_err(db_error)?;
    let status_message = row
        .extracted
        .status_message
        .as_ref()
        .map(|message| message.chars().take(2000).collect::<String>());
    let id = database.with_transaction(|transaction| {
        let id: String = transaction.query_row(
            "INSERT INTO extractions(
                id, sha256, extractor_version, options_key, format, status, status_message,
                metadata_json, structure_json, blocks_truncated, text, text_chars, truncated,
                warnings_json, ocr_used, source_size_bytes, duration_ms, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                       strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             ON CONFLICT(sha256, extractor_version, options_key) DO UPDATE SET
                format = excluded.format,
                status = excluded.status,
                status_message = excluded.status_message,
                metadata_json = excluded.metadata_json,
                structure_json = excluded.structure_json,
                blocks_truncated = excluded.blocks_truncated,
                text = excluded.text,
                text_chars = excluded.text_chars,
                truncated = excluded.truncated,
                warnings_json = excluded.warnings_json,
                ocr_used = excluded.ocr_used,
                source_size_bytes = excluded.source_size_bytes,
                duration_ms = excluded.duration_ms,
                created_at = excluded.created_at
             RETURNING id",
            params![
                Uuid::new_v4().to_string(),
                row.sha256,
                EXTRACTOR_VERSION,
                row.options_key,
                row.extracted.format.as_db_str(),
                row.extracted.status.as_db_str(),
                status_message,
                metadata,
                structure,
                row.extracted.blocks_truncated,
                row.extracted.text,
                row.extracted.text_chars as i64,
                row.extracted.truncated,
                warnings,
                row.extracted.ocr_used,
                row.source_size_bytes as i64,
                row.duration_ms as i64,
            ],
            |result| result.get(0),
        )?;
        link_in(transaction, material, row.sha256, &id)?;
        Ok(id)
    })?;
    extraction_by_id(database, &id)?
        .ok_or_else(|| BackendError::database("Результат извлечения не сохранён"))
}

fn link_in(
    transaction: &rusqlite::Transaction<'_>,
    material: &MaterialRef,
    sha256: &str,
    extraction_id: &str,
) -> Result<(), AppError> {
    transaction.execute(
        "INSERT INTO material_extractions(material_kind, material_id, sha256, extraction_id, analyzed_at)
         VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(material_kind, material_id) DO UPDATE SET
            sha256 = excluded.sha256,
            extraction_id = excluded.extraction_id,
            analyzed_at = excluded.analyzed_at",
        params![material.kind.as_db_str(), material.id, sha256, extraction_id],
    )?;
    Ok(())
}

/// Привязывает уже сохранённый (cache) результат к материалу.
pub fn link(
    database: &Database,
    material: &MaterialRef,
    sha256: &str,
    extraction_id: &str,
) -> Result<(), AppError> {
    database.with_transaction(|transaction| link_in(transaction, material, sha256, extraction_id))
}

/// Связь материала с результатом: (SHA-256 при анализе, id результата, время анализа).
pub fn material_link(
    database: &Database,
    material: &MaterialRef,
) -> Result<Option<(String, Option<String>, String)>, AppError> {
    database.with_connection(|connection| {
        Ok(connection
            .query_row(
                "SELECT sha256, extraction_id, analyzed_at FROM material_extractions
                 WHERE material_kind = ?1 AND material_id = ?2",
                params![material.kind.as_db_str(), material.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?)
    })
}

/// Краткие сведения о результате без текста и структуры.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkSummary {
    pub material: MaterialRef,
    pub sha256: String,
    pub analyzed_at: String,
    pub extraction_id: Option<String>,
    pub format: Option<SourceFormat>,
    pub status: Option<ExtractionStatus>,
    pub status_message: Option<String>,
    pub text_chars: u64,
    pub truncated: bool,
    pub warning_count: u64,
    pub ocr_used: bool,
}

pub fn link_summaries(database: &Database) -> Result<Vec<LinkSummary>, AppError> {
    database.with_connection(|connection| {
        let mut statement = connection.prepare(
            "SELECT m.material_kind, m.material_id, m.sha256, m.analyzed_at, e.id, e.format,
                    e.status, e.status_message, e.text_chars, e.truncated,
                    json_array_length(e.warnings_json), e.ocr_used
             FROM material_extractions m LEFT JOIN extractions e ON e.id = m.extraction_id",
        )?;
        let rows = statement.query_map([], |row| {
            let kind: String = row.get(0)?;
            let format: Option<String> = row.get(5)?;
            let status: Option<String> = row.get(6)?;
            let chars: Option<i64> = row.get(8)?;
            let truncated: Option<i64> = row.get(9)?;
            let warnings: Option<i64> = row.get(10)?;
            let ocr_used: Option<i64> = row.get(11)?;
            Ok(LinkSummary {
                material: MaterialRef {
                    kind: MaterialKind::try_from_db(&kind).unwrap_or(MaterialKind::Inbox),
                    id: row.get(1)?,
                },
                sha256: row.get(2)?,
                analyzed_at: row.get(3)?,
                extraction_id: row.get(4)?,
                format: format.as_deref().and_then(SourceFormat::try_from_db),
                status: status.as_deref().and_then(ExtractionStatus::try_from_db),
                status_message: row.get(7)?,
                text_chars: chars.unwrap_or(0).max(0) as u64,
                truncated: truncated.unwrap_or(0) != 0,
                warning_count: warnings.unwrap_or(0).max(0) as u64,
                ocr_used: ocr_used.unwrap_or(0) != 0,
            })
        })?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row?);
        }
        Ok(summaries)
    })
}

/// Полностью очищает cache: результаты и их связи. Подборки сохраняются, но их
/// материалы придётся проанализировать заново.
pub fn clear_cache(database: &Database) -> Result<u64, AppError> {
    database.with_transaction(|transaction| {
        transaction.execute("DELETE FROM material_extractions", [])?;
        let removed = transaction.execute("DELETE FROM extractions", [])?;
        Ok(removed as u64)
    })
}

// ───────────────────────── Настройки ─────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtractionSettings {
    /// Распознавать текст на изображениях локальным OCR. По умолчанию выключено.
    pub ocr_enabled: bool,
}

pub fn read_settings(database: &Database) -> Result<ExtractionSettings, AppError> {
    database.with_connection(|connection| {
        let raw: Option<String> = connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                [SETTINGS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            None => Ok(ExtractionSettings::default()),
            Some(raw) => serde_json::from_str(&raw)
                .map_err(|_| BackendError::database("Настройка извлечения текста повреждена")),
        }
    })
}

pub fn write_settings(database: &Database, settings: ExtractionSettings) -> Result<(), AppError> {
    let value_json = serde_json::to_string(&settings).map_err(db_error)?;
    database.with_transaction(|transaction| {
        transaction.execute(
            "INSERT INTO app_settings(key, value_json, created_at, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET
               value_json = excluded.value_json,
               updated_at = CURRENT_TIMESTAMP",
            params![SETTINGS_KEY, value_json],
        )?;
        Ok(())
    })
}

// ───────────────────────── Пакетный анализ ─────────────────────────

pub enum BatchLookup {
    Missing,
    Finished(String),
    Unfinished,
}

pub fn lookup_batch(database: &Database, operation_id: &str) -> Result<BatchLookup, AppError> {
    database.with_connection(|connection| {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE id = ?1)",
            [operation_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(BatchLookup::Missing);
        }
        let report: Option<String> = connection
            .query_row(
                "SELECT report_json FROM extraction_batches WHERE operation_id = ?1",
                [operation_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match report {
            Some(json) => BatchLookup::Finished(json),
            None => BatchLookup::Unfinished,
        })
    })
}

pub fn start_batch(database: &Database, operation_id: &str, detail: &str) -> Result<(), AppError> {
    database.with_transaction(|transaction| {
        writes::insert_running_operation(
            transaction,
            operation_id,
            OperationKind::Extract,
            "Извлечение текста",
        )?;
        transaction.execute(
            "UPDATE operations SET detail = ?2 WHERE id = ?1",
            params![operation_id, detail],
        )?;
        Ok(())
    })
}

pub fn batch_progress(
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

#[derive(Serialize)]
struct SafeError<'a> {
    code: ErrorCode,
    message: &'a str,
}

pub fn finish_batch(
    database: &Database,
    operation_id: &str,
    failure: Option<&AppError>,
    detail: &str,
    report_json: &str,
) -> Result<(), AppError> {
    let error_json = failure
        .map(|error| {
            serde_json::to_string(&SafeError {
                code: error.code,
                message: &error.message,
            })
        })
        .transpose()
        .map_err(db_error)?;
    database.with_transaction(|transaction| {
        let changed = transaction.execute(
            "UPDATE operations
             SET status = ?2, progress = 100, detail = ?3, error_json = ?4, undoable = 0,
                 completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND status = 'running'",
            params![
                operation_id,
                if failure.is_some() {
                    "failed"
                } else {
                    "completed"
                },
                detail,
                error_json,
            ],
        )?;
        if changed != 1 {
            return Err(BackendError::database(
                "Операция извлечения не может быть завершена повторно",
            ));
        }
        transaction.execute(
            "INSERT INTO extraction_batches(operation_id, report_json) VALUES (?1, ?2)",
            params![operation_id, report_json],
        )?;
        Ok(())
    })
}

// ───────────────────────── Подборки ─────────────────────────

pub struct SelectionRow {
    pub id: String,
    pub updated_at: String,
    pub items: Vec<(MaterialRef, String)>,
}

pub fn homework_exists(database: &Database, homework_id: &str) -> Result<bool, AppError> {
    database.with_connection(|connection| {
        Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM homeworks WHERE id = ?1)",
            [homework_id],
            |row| row.get(0),
        )?)
    })
}

pub fn homework_submitted(database: &Database, homework_id: &str) -> Result<bool, AppError> {
    database.with_connection(|connection| {
        Ok(connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM homeworks WHERE id = ?1 AND stage = 'submitted')",
            [homework_id],
            |row| row.get(0),
        )?)
    })
}

pub fn read_selection(
    database: &Database,
    homework_id: Option<&str>,
) -> Result<Option<SelectionRow>, AppError> {
    database.with_connection(|connection| {
        let selection: Option<(String, String)> = connection
            .query_row(
                "SELECT id, updated_at FROM corpus_selections
                 WHERE (homework_id IS NULL AND ?1 IS NULL) OR homework_id = ?1",
                params![homework_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((id, updated_at)) = selection else {
            return Ok(None);
        };
        let mut statement = connection.prepare(
            "SELECT material_kind, material_id, sha256 FROM corpus_selection_items
             WHERE selection_id = ?1 ORDER BY position, material_id",
        )?;
        let rows = statement.query_map([&id], |row| {
            let kind: String = row.get(0)?;
            Ok((
                MaterialRef {
                    kind: MaterialKind::try_from_db(&kind).unwrap_or(MaterialKind::Inbox),
                    id: row.get(1)?,
                },
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(Some(SelectionRow {
            id,
            updated_at,
            items,
        }))
    })
}

/// Заменяет состав подборки одной транзакцией.
pub fn replace_selection(
    database: &Database,
    homework_id: Option<&str>,
    items: &[(MaterialRef, String)],
) -> Result<(), AppError> {
    database.with_transaction(|transaction| {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM corpus_selections
                 WHERE (homework_id IS NULL AND ?1 IS NULL) OR homework_id = ?1",
                params![homework_id],
                |row| row.get(0),
            )
            .optional()?;
        let id = match existing {
            Some(id) => {
                transaction.execute(
                    "UPDATE corpus_selections SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     WHERE id = ?1",
                    [&id],
                )?;
                id
            }
            None => {
                let id = Uuid::new_v4().to_string();
                transaction.execute(
                    "INSERT INTO corpus_selections(id, homework_id, created_at, updated_at)
                     VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                             strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                    params![id, homework_id],
                )?;
                id
            }
        };
        transaction.execute(
            "DELETE FROM corpus_selection_items WHERE selection_id = ?1",
            [&id],
        )?;
        for (position, (material, sha256)) in items.iter().enumerate() {
            transaction.execute(
                "INSERT INTO corpus_selection_items(
                    selection_id, material_kind, material_id, sha256, position, added_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![
                    id,
                    material.kind.as_db_str(),
                    material.id,
                    sha256,
                    position as i64
                ],
            )?;
        }
        Ok(())
    })
}
