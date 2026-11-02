//! SQL-граница рабочего цикла. Все функции принимают `&Connection`, поэтому
//! одинаково работают в обычном чтении и внутри транзакции application service.
//! Списки одного Homework собираются фиксированным числом запросов: каждая
//! сущность — один запрос, без N+1.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    domain::{HomeworkStage, HomeworkWorkflowStage, RequirementStatus, VersionAuthor},
    dto::{FileKind, FileRefDto, HomeworkDto, HomeworkVersionDto, ProviderId},
    error::{AppError, BackendError},
    extraction::{dto::MaterialKind, MaterialRef},
    repositories::{hydrate_file, load_homework, require_relative_path},
};

use super::dto::{
    BundleOrigin, BundleWarningDto, CommentAuthority, CriterionStatus, EvidenceReferenceDto,
    MissingSourceDto, PackageCheckDto, PackageItemDto, PackageLinkDto, PromptKind, PromptStatus,
    Provenance, ReviewAssessmentDto, ReviewMissingItemDto, ReviewResultDto, ReviewRunDto,
    ReviewRunStatus, ReviewVerdict, RevisionPromptDto, RunKind, SourceBundleDto, SourceItemDto,
    SourceKind, StoredReviewResult, StyleConfirmationDto, SubmissionPackageDto,
    SubmissionReceiptDto, TeacherCommentDto, TeacherCommentSource, TeacherRequirementDto,
    UnsupportedClaimDto, WorkflowRole,
};

// ───────────────────────── Общие помощники ─────────────────────────

pub(crate) fn parse_column<T: DeserializeOwned>(raw: &str, field: &str) -> Result<T, AppError> {
    serde_json::from_str(raw)
        .map_err(|error| BackendError::database(format!("invalid JSON in {field}: {error}")))
}

pub(crate) fn json_text<T: Serialize>(value: &T, field: &str) -> Result<String, AppError> {
    let text = serde_json::to_string(value)
        .map_err(|error| BackendError::database(format!("cannot serialize {field}: {error}")))?;
    let parsed: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| BackendError::database(format!("invalid JSON in {field}: {error}")))?;
    reject_absolute_strings(&parsed, field)?;
    Ok(text)
}

/// Повторная проверка перед записью: ни одно строковое значение вложенного JSON
/// не может быть абсолютным/home/`file:`-путём или выходить за корень через `..`.
pub(crate) fn reject_absolute_strings(
    value: &serde_json::Value,
    field: &str,
) -> Result<(), AppError> {
    match value {
        serde_json::Value::String(text) => {
            let lowered = text.to_ascii_lowercase();
            let unsafe_path = text.starts_with('/')
                || text.starts_with('\\')
                || text.starts_with("~/")
                || lowered.starts_with("file:")
                || text.contains("/../")
                || text.starts_with("../")
                || (text.len() > 2
                    && text.as_bytes()[1] == b':'
                    && text.as_bytes()[0].is_ascii_alphabetic()
                    && matches!(text.as_bytes()[2], b'\\' | b'/'));
            if unsafe_path {
                return Err(BackendError::conflict(format!(
                    "{field} must not contain absolute or escaping paths"
                )));
            }
            Ok(())
        }
        serde_json::Value::Array(items) => items
            .iter()
            .try_for_each(|item| reject_absolute_strings(item, field)),
        serde_json::Value::Object(object) => object
            .values()
            .try_for_each(|item| reject_absolute_strings(item, field)),
        _ => Ok(()),
    }
}

fn db_enum<T>(
    value: &str,
    field: &str,
    parse: fn(&str) -> Result<T, String>,
) -> Result<T, AppError> {
    parse(value)
        .map_err(|message| BackendError::database(format!("invalid enum in {field}: {message}")))
}

fn optional_enum<T>(
    value: Option<String>,
    field: &str,
    parse: fn(&str) -> Result<T, String>,
) -> Result<Option<T>, AppError> {
    value.map(|raw| db_enum(&raw, field, parse)).transpose()
}

fn provider_from_db(value: &str) -> Result<ProviderId, String> {
    match value {
        "codex" => Ok(ProviderId::Codex),
        "claude" => Ok(ProviderId::Claude),
        other => Err(format!("unknown provider: {other}")),
    }
}

pub(crate) fn provider_db_str(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Claude => "claude",
    }
}

fn material_kind_from_db(value: &str) -> Result<MaterialKind, String> {
    MaterialKind::try_from_db(value).ok_or_else(|| format!("unknown material kind: {value}"))
}

fn small_u8(value: i64, field: &str) -> Result<u8, AppError> {
    u8::try_from(value)
        .map_err(|_| BackendError::database(format!("{field} is outside the 0..255 range")))
}

fn non_negative(value: i64, field: &str) -> Result<u64, AppError> {
    u64::try_from(value).map_err(|_| BackendError::database(format!("{field} is negative")))
}

pub(crate) fn not_found_or(error: rusqlite::Error, message: &str) -> AppError {
    match error {
        rusqlite::Error::QueryReturnedNoRows => BackendError::not_found(message),
        other => other.into(),
    }
}

/// Нарушение ограничения SQLite → `conflict` (или `immutable` для защитных триггеров).
pub(crate) fn map_write_error(error: rusqlite::Error, message: &str) -> AppError {
    if let rusqlite::Error::SqliteFailure(details, text) = &error {
        if details.code == rusqlite::ErrorCode::ConstraintViolation {
            let reason = text.clone().unwrap_or_default();
            if reason.contains("immutable") || reason.contains("submitted homework") {
                return BackendError::immutable(message);
            }
            return BackendError::conflict(message);
        }
    }
    error.into()
}

// ───────────────────────── Homework ─────────────────────────

/// Строка Homework, нужная для проверок state machine и ревизии.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeworkCore {
    pub id: String,
    pub stage: HomeworkStage,
    pub workflow_stage: HomeworkWorkflowStage,
    pub final_version_id: Option<String>,
    pub revision: i64,
}

pub fn homework_core(connection: &Connection, homework_id: &str) -> Result<HomeworkCore, AppError> {
    let (id, stage, workflow_stage, final_version_id, revision): (
        String,
        String,
        String,
        Option<String>,
        i64,
    ) = connection
        .query_row(
            "SELECT id, stage, workflow_stage, final_version_id, revision FROM homeworks WHERE id = ?1",
            [homework_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|error| not_found_or(error, "ДЗ не найдено"))?;
    Ok(HomeworkCore {
        id,
        stage: db_enum(&stage, "homeworks.stage", HomeworkStage::try_from_db)?,
        workflow_stage: db_enum(
            &workflow_stage,
            "homeworks.workflow_stage",
            HomeworkWorkflowStage::try_from_db,
        )?,
        final_version_id,
        revision,
    })
}

pub fn homework_dto(connection: &Connection, homework_id: &str) -> Result<HomeworkDto, AppError> {
    load_homework(connection, homework_id)
}

/// Производная «папочная» стадия по этапу рабочего цикла.
pub fn folder_stage_for(stage: HomeworkWorkflowStage) -> HomeworkStage {
    match stage {
        HomeworkWorkflowStage::MaterialsCollected | HomeworkWorkflowStage::CodexAnalysis => {
            HomeworkStage::Task
        }
        HomeworkWorkflowStage::PromptReady
        | HomeworkWorkflowStage::ClaudeSolving
        | HomeworkWorkflowStage::CodexReview
        | HomeworkWorkflowStage::NeedsFixes
        | HomeworkWorkflowStage::TeacherReview => HomeworkStage::InProgress,
        HomeworkWorkflowStage::ReadyToSubmit => HomeworkStage::Ready,
        HomeworkWorkflowStage::Submitted => HomeworkStage::Submitted,
    }
}

/// Увеличивает ревизию агрегата. `expected` — ревизия, которую видел клиент:
/// расхождение означает устаревшее состояние → `conflict`.
pub fn bump_revision(
    connection: &Connection,
    homework_id: &str,
    expected: i64,
) -> Result<i64, AppError> {
    let changed = connection
        .execute(
            "UPDATE homeworks SET revision = revision + 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND revision = ?2",
            params![homework_id, expected],
        )
        .map_err(|error| map_write_error(error, "Отправленная работа неизменяема"))?;
    if changed != 1 {
        return Err(stale_revision());
    }
    Ok(expected + 1)
}

pub fn stale_revision() -> AppError {
    BackendError::conflict(
        "Данные ДЗ изменились в другом окне или команде — перечитайте состояние и повторите действие",
    )
}

pub fn set_workflow_stage(
    connection: &Connection,
    homework_id: &str,
    to: HomeworkWorkflowStage,
) -> Result<(), AppError> {
    if to == HomeworkWorkflowStage::Submitted {
        return Err(BackendError::blocked(
            "Отметка «Отправлено» создаётся только подтверждением отправки (этап 10)",
        ));
    }
    connection
        .execute(
            "UPDATE homeworks SET workflow_stage = ?2, stage = ?3,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            params![
                homework_id,
                to.as_db_str(),
                folder_stage_for(to).as_db_str()
            ],
        )
        .map_err(|error| map_write_error(error, "Отправленная работа неизменяема"))?;
    Ok(())
}

pub fn set_current_version(
    connection: &Connection,
    homework_id: &str,
    version_id: &str,
) -> Result<(), AppError> {
    connection
        .execute(
            "UPDATE homeworks SET final_version_id = ?2,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            params![homework_id, version_id],
        )
        .map_err(|error| map_write_error(error, "Отправленная работа неизменяема"))?;
    Ok(())
}

// ───────────────────────── Версии ─────────────────────────

pub fn versions(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<HomeworkVersionDto>, AppError> {
    Ok(
        crate::repositories::load_versions(connection, Some(homework_id))?
            .remove(homework_id)
            .unwrap_or_default(),
    )
}

pub struct NewVersion<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub main: &'a FileRefDto,
    pub files: &'a [FileRefDto],
    pub note: Option<&'a str>,
    pub author: VersionAuthor,
    pub prompt_id: Option<&'a str>,
    pub source_bundle_id: Option<&'a str>,
}

/// Append-only вставка: номер назначает backend внутри транзакции (`MAX + 1`),
/// а `UNIQUE(homework_id, number)` исключает одинаковый номер при гонке.
pub fn insert_version(connection: &Connection, version: &NewVersion<'_>) -> Result<i64, AppError> {
    require_relative_path(&version.main.relative_path, "homework_versions.file_path")?;
    for file in version.files {
        require_relative_path(&file.relative_path, "homework_versions.files_json")?;
    }
    let number: i64 = connection.query_row(
        "SELECT coalesce(max(number), 0) + 1 FROM homework_versions WHERE homework_id = ?1",
        [version.homework_id],
        |row| row.get(0),
    )?;
    let label = format!("v{number:02}");
    let files_json = json_text(&version.files, "homework_versions.files_json")?;
    connection
        .execute(
            "INSERT INTO homework_versions(
                id, homework_id, number, label, file_path, size_bytes, author, metadata_json,
                frozen, created_at, file_name, source_file_id, note, sections_json, prompt_id,
                source_bundle_id, files_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                       ?8, ?9, ?10, '[]', ?11, ?12, ?13)",
            params![
                version.id,
                version.homework_id,
                number,
                label,
                version.main.relative_path,
                version
                    .files
                    .iter()
                    .map(|file| file.size_bytes)
                    .sum::<i64>(),
                version.author.as_str(),
                version.main.name,
                version.main.id,
                version.note,
                version.prompt_id,
                version.source_bundle_id,
                files_json,
            ],
        )
        .map_err(|error| {
            map_write_error(error, "Версия с этим файлом или номером уже существует")
        })?;
    Ok(number)
}

/// Файлы ДЗ, которые можно оформить новой версией (черновики и итоговые файлы,
/// ещё не ставшие основным файлом версии).
pub fn version_candidates(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<FileRefDto>, AppError> {
    let mut statement = connection.prepare(
        "SELECT f.id, f.name, f.kind, f.size_bytes, coalesce(f.modified_at, f.updated_at, f.created_at),
                f.relative_path
         FROM files f
         WHERE f.homework_id = ?1 AND f.category IN ('homework_draft', 'homework_final')
           AND f.status = 'available'
           AND NOT EXISTS (SELECT 1 FROM homework_versions v WHERE v.file_path = f.relative_path)
         ORDER BY coalesce(f.modified_at, f.updated_at, f.created_at) DESC, f.id",
    )?;
    let mut rows = statement.query([homework_id])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        output.push(hydrate_file(
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        )?);
    }
    Ok(output)
}

/// Файл ДЗ по id; файл другого ДЗ или недели не принимается.
pub fn homework_file(
    connection: &Connection,
    homework_id: &str,
    file_id: &str,
) -> Result<FileRefDto, AppError> {
    let row: Option<(String, String, String, i64, String, String)> = connection
        .query_row(
            "SELECT id, name, kind, size_bytes, coalesce(modified_at, updated_at, created_at), relative_path
             FROM files WHERE id = ?1 AND homework_id = ?2 AND status = 'available'",
            params![file_id, homework_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .optional()?;
    let (id, name, kind, size, modified, path) =
        row.ok_or_else(|| BackendError::not_found("Файл ДЗ не найден"))?;
    hydrate_file(id, name, kind, size, modified, path)
}

// ───────────────────────── SourceBundle ─────────────────────────

pub struct NewBundle<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub purpose: RunKind,
    pub version_id: Option<&'a str>,
    pub complete: bool,
    pub missing: &'a [MissingSourceDto],
    pub selection_id: &'a str,
    pub extractor_version: u32,
    pub warnings: &'a [BundleWarningDto],
    pub total_chars: u64,
    pub limit_chars: u64,
}

pub struct NewBundleItem<'a> {
    pub id: String,
    pub material: &'a MaterialRef,
    pub kind: SourceKind,
    pub required: bool,
    pub position: i64,
    pub name: &'a str,
    pub file_kind: FileKind,
    pub size_bytes: i64,
    pub modified_at: &'a str,
    pub relative_path: &'a str,
    pub sha256: &'a str,
    pub extraction_id: &'a str,
    pub extractor_version: u32,
    pub extraction_status: &'a str,
    pub text_chars: u64,
    pub warnings: &'a [String],
}

pub fn insert_bundle(
    connection: &Connection,
    bundle: &NewBundle<'_>,
    items: &[NewBundleItem<'_>],
) -> Result<(), AppError> {
    let missing = json_text(&bundle.missing, "source_bundles.missing_json")?;
    let warnings = json_text(&bundle.warnings, "source_bundles.warnings_json")?;
    connection
        .execute(
            "INSERT INTO source_bundles(
                id, homework_id, purpose, version_id, complete, missing_json, created_at, updated_at,
                origin, selection_id, extractor_version, warnings_json, total_chars, limit_chars
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                       strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'corpus_selection', ?7, ?8, ?9, ?10, ?11)",
            params![
                bundle.id,
                bundle.homework_id,
                bundle.purpose.as_db_str(),
                bundle.version_id,
                i64::from(bundle.complete),
                missing,
                bundle.selection_id,
                i64::from(bundle.extractor_version),
                warnings,
                bundle.total_chars as i64,
                bundle.limit_chars as i64,
            ],
        )
        .map_err(|error| map_write_error(error, "Набор источников не сохранён"))?;
    for item in items {
        require_relative_path(item.relative_path, "source_bundle_items.relative_path")?;
        let warnings = json_text(&item.warnings, "source_bundle_items.warnings_json")?;
        connection
            .execute(
                "INSERT INTO source_bundle_items(
                    id, source_bundle_id, material_kind, material_id, source_kind, required, included,
                    position, name, file_kind, size_bytes, modified_at, relative_path, sha256,
                    extraction_id, extractor_version, extraction_status, text_chars, warnings_json,
                    created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                           ?17, ?18, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![
                    item.id,
                    bundle.id,
                    item.material.kind.as_db_str(),
                    item.material.id,
                    item.kind.as_db_str(),
                    i64::from(item.required),
                    item.position,
                    item.name,
                    file_kind_str(item.file_kind),
                    item.size_bytes,
                    item.modified_at,
                    item.relative_path,
                    item.sha256,
                    item.extraction_id,
                    i64::from(item.extractor_version),
                    item.extraction_status,
                    item.text_chars as i64,
                    warnings,
                ],
            )
            .map_err(|error| map_write_error(error, "Элемент набора источников не сохранён"))?;
    }
    Ok(())
}

pub(crate) fn file_kind_str(kind: FileKind) -> &'static str {
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

struct BundleRow {
    id: String,
    homework_id: String,
    purpose: String,
    version_id: Option<String>,
    complete: bool,
    missing_json: String,
    created_at: String,
    origin: String,
    selection_id: Option<String>,
    extractor_version: Option<i64>,
    warnings_json: String,
    total_chars: i64,
    limit_chars: i64,
}

const BUNDLE_COLUMNS: &str =
    "b.id, b.homework_id, b.purpose, b.version_id, b.complete, b.missing_json,
    b.created_at, b.origin, b.selection_id, b.extractor_version, b.warnings_json, b.total_chars,
    b.limit_chars";

fn bundle_row(row: &Row<'_>) -> rusqlite::Result<BundleRow> {
    Ok(BundleRow {
        id: row.get(0)?,
        homework_id: row.get(1)?,
        purpose: row.get(2)?,
        version_id: row.get(3)?,
        complete: row.get::<_, i64>(4)? == 1,
        missing_json: row.get(5)?,
        created_at: row.get(6)?,
        origin: row.get(7)?,
        selection_id: row.get(8)?,
        extractor_version: row.get(9)?,
        warnings_json: row.get(10)?,
        total_chars: row.get(11)?,
        limit_chars: row.get(12)?,
    })
}

const ITEM_SELECT: &str =
    "SELECT i.source_bundle_id, i.id, i.material_kind, i.material_id, i.source_kind,
        i.required, i.included, i.name, i.file_kind, i.size_bytes, i.modified_at, i.relative_path,
        i.sha256, i.extraction_id, i.extractor_version, i.text_chars, i.warnings_json,
        CASE i.material_kind
            WHEN 'inbox' THEN EXISTS(SELECT 1 FROM inbox_files x WHERE x.id = i.material_id)
            ELSE EXISTS(SELECT 1 FROM files f WHERE f.id = i.material_id)
        END,
        CASE i.material_kind
            WHEN 'inbox' THEN (SELECT x.sha256 FROM inbox_files x WHERE x.id = i.material_id)
            ELSE NULL
        END,
        (SELECT m.sha256 FROM material_extractions m
          WHERE m.material_kind = i.material_kind AND m.material_id = i.material_id)
    FROM source_bundle_items i JOIN source_bundles b ON b.id = i.source_bundle_id";

/// Элемент снимка вместе с признаком «материал изменился после снимка».
#[derive(Debug, Clone, PartialEq)]
pub struct BundleItemState {
    pub item: SourceItemDto,
    pub changed: bool,
}

fn item_row(row: &Row<'_>) -> Result<(String, BundleItemState), AppError> {
    let bundle_id: String = row.get(0)?;
    let material_kind: String = row.get(2)?;
    let file_kind: String = row.get(8)?;
    let relative_path: String = row.get(11)?;
    require_relative_path(&relative_path, "source_bundle_items.relative_path")?;
    let sha256: Option<String> = row.get(12)?;
    let extractor_version: Option<i64> = row.get(14)?;
    let warnings_json: String = row.get(16)?;
    let present: bool = row.get(17)?;
    let inbox_sha: Option<String> = row.get(18)?;
    let link_sha: Option<String> = row.get(19)?;
    let changed = match sha256.as_deref() {
        // Legacy-строка без закреплённого содержимого: сравнить не с чем.
        None => !present,
        Some(pinned) => {
            !present
                || inbox_sha
                    .as_deref()
                    .is_some_and(|current| current != pinned)
                || link_sha.as_deref().is_some_and(|current| current != pinned)
        }
    };
    let item = SourceItemDto {
        id: row.get(1)?,
        material: MaterialRef {
            kind: db_enum(
                &material_kind,
                "source_bundle_items.material_kind",
                material_kind_from_db,
            )?,
            id: row.get(3)?,
        },
        kind: db_enum(
            &row.get::<_, String>(4)?,
            "source_bundle_items.source_kind",
            SourceKind::try_from_db,
        )?,
        required: row.get::<_, i64>(5)? == 1,
        included: row.get::<_, i64>(6)? == 1,
        file: FileRefDto {
            id: row.get(3)?,
            name: row.get(7)?,
            kind: db_enum(
                &file_kind,
                "source_bundle_items.file_kind",
                FileKind::try_from_db,
            )?,
            size_bytes: row.get(9)?,
            modified_at: row.get(10)?,
            relative_path,
        },
        sha256,
        extraction_id: row.get(13)?,
        extractor_version: extractor_version
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    BackendError::database("source_bundle_items.extractor_version is invalid")
                })
            })
            .transpose()?,
        text_chars: non_negative(row.get(15)?, "source_bundle_items.text_chars")?,
        warnings: parse_column(&warnings_json, "source_bundle_items.warnings_json")?,
    };
    Ok((bundle_id, BundleItemState { item, changed }))
}

/// Все снимки ДЗ: два запроса (наборы + элементы всех наборов).
pub fn bundles(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<(SourceBundleDto, Vec<bool>)>, AppError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {BUNDLE_COLUMNS} FROM source_bundles b WHERE b.homework_id = ?1 ORDER BY b.created_at, b.id"
    ))?;
    let rows = statement
        .query_map([homework_id], bundle_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut items = bundle_items(connection, "b.homework_id = ?1", homework_id)?;
    rows.into_iter()
        .map(|row| {
            let states = items.remove(&row.id).unwrap_or_default();
            hydrate_bundle(row, states)
        })
        .collect()
}

pub fn bundle(
    connection: &Connection,
    bundle_id: &str,
) -> Result<(SourceBundleDto, Vec<bool>), AppError> {
    let row = connection
        .query_row(
            &format!("SELECT {BUNDLE_COLUMNS} FROM source_bundles b WHERE b.id = ?1"),
            [bundle_id],
            bundle_row,
        )
        .map_err(|error| not_found_or(error, "Набор источников не найден"))?;
    let mut items = bundle_items(connection, "b.id = ?1", bundle_id)?;
    let states = items.remove(&row.id).unwrap_or_default();
    hydrate_bundle(row, states)
}

fn bundle_items(
    connection: &Connection,
    filter: &str,
    parameter: &str,
) -> Result<HashMap<String, Vec<BundleItemState>>, AppError> {
    let mut statement = connection.prepare(&format!(
        "{ITEM_SELECT} WHERE {filter} ORDER BY i.source_bundle_id, i.position, i.id"
    ))?;
    let mut rows = statement.query([parameter])?;
    let mut output: HashMap<String, Vec<BundleItemState>> = HashMap::new();
    while let Some(row) = rows.next()? {
        let (bundle_id, state) = item_row(row)?;
        output.entry(bundle_id).or_default().push(state);
    }
    Ok(output)
}

fn hydrate_bundle(
    row: BundleRow,
    states: Vec<BundleItemState>,
) -> Result<(SourceBundleDto, Vec<bool>), AppError> {
    let changed = states.iter().map(|state| state.changed).collect();
    let dto = SourceBundleDto {
        purpose: db_enum(&row.purpose, "source_bundles.purpose", RunKind::try_from_db)?,
        origin: db_enum(
            &row.origin,
            "source_bundles.origin",
            BundleOrigin::try_from_db,
        )?,
        missing: parse_column(&row.missing_json, "source_bundles.missing_json")?,
        warnings: parse_column(&row.warnings_json, "source_bundles.warnings_json")?,
        extractor_version: row
            .extractor_version
            .map(|value| {
                u32::try_from(value).map_err(|_| {
                    BackendError::database("source_bundles.extractor_version is invalid")
                })
            })
            .transpose()?,
        total_chars: non_negative(row.total_chars, "source_bundles.total_chars")?,
        limit_chars: non_negative(row.limit_chars, "source_bundles.limit_chars")?,
        items: states.into_iter().map(|state| state.item).collect(),
        id: row.id,
        homework_id: row.homework_id,
        version_id: row.version_id,
        complete: row.complete,
        created_at: row.created_at,
        selection_id: row.selection_id,
    };
    Ok((dto, changed))
}

// ───────────────────────── ReviewRun ─────────────────────────

pub struct NewRun<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub kind: RunKind,
    pub role: WorkflowRole,
    pub provider: ProviderId,
    pub version_id: Option<&'a str>,
    pub source_bundle_id: &'a str,
    pub sources_used: &'a [EvidenceReferenceDto],
    pub status: ReviewRunStatus,
    pub step: &'a str,
    pub style_threshold: u8,
}

pub fn insert_run(connection: &Connection, run: &NewRun<'_>) -> Result<(), AppError> {
    let sources = json_text(&run.sources_used, "review_runs.sources_used_json")?;
    let inputs = super::fingerprint::current(connection, run.homework_id)?;
    connection
        .execute(
            "INSERT INTO review_runs(
                id, homework_id, kind, role, provider, version_id, source_bundle_id, sources_used_json,
                status, progress, step, started_at, saved_as_iteration, provenance, style_threshold,
                created_at, inputs_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                       0, 'manual', ?11, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?12)",
            params![
                run.id,
                run.homework_id,
                run.kind.as_db_str(),
                run.role.as_db_str(),
                provider_db_str(run.provider),
                run.version_id,
                run.source_bundle_id,
                sources,
                run.status.as_db_str(),
                run.step,
                i64::from(run.style_threshold),
                inputs.combined,
            ],
        )
        .map_err(|error| map_write_error(error, "Запуск не сохранён"))?;
    Ok(())
}

/// Результат, сохранённый пользователем вручную.
pub struct ManualCompletion<'a> {
    pub run_id: &'a str,
    pub result: &'a StoredReviewResult,
    pub correctness: CriterionStatus,
    pub task_fit: CriterionStatus,
    pub course_grounding: CriterionStatus,
    pub teacher: CriterionStatus,
    pub style_score: Option<u8>,
}

pub fn complete_run_manually(
    connection: &Connection,
    completion: &ManualCompletion<'_>,
) -> Result<(), AppError> {
    complete_run(
        connection,
        completion,
        Provenance::Manual,
        "Результат сохранён вручную",
    )
}

/// Завершение запуска результатом, который подтвердил пользователь.
/// `provenance = provider` — результат получен от провайдера (CLI или ручная
/// передача ответа) и подтверждён пользователем; `manual` — введён вручную.
pub fn complete_run(
    connection: &Connection,
    completion: &ManualCompletion<'_>,
    provenance: Provenance,
    step: &str,
) -> Result<(), AppError> {
    let result = json_text(completion.result, "review_runs.result_json")?;
    let missing = json_text(&completion.result.missing, "review_runs.missing_json")?;
    let critical = json_text(&completion.result.critical, "review_runs.critical_json")?;
    let unsupported = json_text(
        &completion.result.unsupported_claims,
        "review_runs.unsupported_json",
    )?;
    let changed = connection
        .execute(
            "UPDATE review_runs SET status = 'completed', progress = 100, step = ?12,
                    finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), provenance = ?13,
                    verdict = ?2, correctness = ?3, task_fit = ?4, course_grounding = ?5,
                    teacher_status = ?6, style_score = ?7, missing_json = ?8, critical_json = ?9,
                    unsupported_json = ?10, result_json = ?11, error_message = NULL
             WHERE id = ?1 AND status IN ('queued', 'waiting_for_provider', 'manual_handoff', 'running')",
            params![
                completion.run_id,
                completion.result.verdict.as_db_str(),
                completion.correctness.as_db_str(),
                completion.task_fit.as_db_str(),
                completion.course_grounding.as_db_str(),
                completion.teacher.as_db_str(),
                completion.style_score.map(i64::from),
                missing,
                critical,
                unsupported,
                result,
                step,
                provenance.as_db_str(),
            ],
        )
        .map_err(|error| map_write_error(error, "Завершённый запуск неизменяем"))?;
    if changed != 1 {
        return Err(BackendError::conflict("Запуск уже завершён или отменён"));
    }
    Ok(())
}

pub fn cancel_run(connection: &Connection, run_id: &str) -> Result<(), AppError> {
    let changed = connection
        .execute(
            "UPDATE review_runs SET status = 'cancelled', step = 'Отменено пользователем',
                    finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND status IN ('queued', 'waiting_for_provider', 'manual_handoff', 'running')",
            [run_id],
        )
        .map_err(|error| map_write_error(error, "Завершённый запуск неизменяем"))?;
    if changed != 1 {
        return Err(BackendError::conflict("Запуск уже завершён или отменён"));
    }
    Ok(())
}

pub fn pin_run(connection: &Connection, run_id: &str) -> Result<(), AppError> {
    let changed = connection
        .execute(
            "UPDATE review_runs SET saved_as_iteration = 1 WHERE id = ?1 AND status = 'completed'",
            [run_id],
        )
        .map_err(|error| map_write_error(error, "Запуск не закреплён"))?;
    if changed != 1 {
        return Err(BackendError::blocked(
            "Закрепить можно только завершённый запуск",
        ));
    }
    Ok(())
}

struct RunRow {
    id: String,
    homework_id: String,
    kind: String,
    role: String,
    provider: String,
    version_id: Option<String>,
    source_bundle_id: Option<String>,
    sources_used_json: String,
    status: String,
    progress: i64,
    step: String,
    started_at: String,
    finished_at: Option<String>,
    saved_as_iteration: bool,
    provenance: String,
    verdict: Option<String>,
    correctness: Option<String>,
    task_fit: Option<String>,
    course_grounding: Option<String>,
    teacher_status: Option<String>,
    critical_json: String,
    unsupported_json: String,
    missing_json: String,
    result_json: Option<String>,
    error_message: Option<String>,
    style_score: Option<i64>,
    style_threshold: i64,
    inputs_fingerprint: Option<String>,
}

const RUN_SELECT: &str =
    "SELECT id, homework_id, kind, role, provider, version_id, source_bundle_id,
        sources_used_json, status, progress, step, started_at, finished_at, saved_as_iteration,
        provenance, verdict, correctness, task_fit, course_grounding, teacher_status, critical_json,
        unsupported_json, missing_json, result_json, error_message, style_score, style_threshold,
        inputs_fingerprint
    FROM review_runs";

fn run_row(row: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        homework_id: row.get(1)?,
        kind: row.get(2)?,
        role: row.get(3)?,
        provider: row.get(4)?,
        version_id: row.get(5)?,
        source_bundle_id: row.get(6)?,
        sources_used_json: row.get(7)?,
        status: row.get(8)?,
        progress: row.get(9)?,
        step: row.get(10)?,
        started_at: row.get(11)?,
        finished_at: row.get(12)?,
        saved_as_iteration: row.get::<_, i64>(13)? == 1,
        provenance: row.get(14)?,
        verdict: row.get(15)?,
        correctness: row.get(16)?,
        task_fit: row.get(17)?,
        course_grounding: row.get(18)?,
        teacher_status: row.get(19)?,
        critical_json: row.get(20)?,
        unsupported_json: row.get(21)?,
        missing_json: row.get(22)?,
        result_json: row.get(23)?,
        error_message: row.get(24)?,
        style_score: row.get(25)?,
        style_threshold: row.get(26)?,
        inputs_fingerprint: row.get(27)?,
    })
}

/// Все запуски ДЗ (новые сверху). `prompts` — уже прочитанные промты того же ДЗ:
/// промт результата подставляется без дополнительного запроса.
pub fn runs(
    connection: &Connection,
    homework_id: &str,
    prompts: &[RevisionPromptDto],
) -> Result<Vec<ReviewRunDto>, AppError> {
    let mut statement = connection.prepare(&format!(
        "{RUN_SELECT} WHERE homework_id = ?1 ORDER BY created_at DESC, started_at DESC, id DESC"
    ))?;
    let rows = statement
        .query_map([homework_id], run_row)?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|row| hydrate_run(row, prompts))
        .collect()
}

pub fn run(connection: &Connection, run_id: &str) -> Result<ReviewRunDto, AppError> {
    let row = connection
        .query_row(&format!("{RUN_SELECT} WHERE id = ?1"), [run_id], run_row)
        .map_err(|error| not_found_or(error, "Запуск не найден"))?;
    let prompts = prompts_for_run(connection, &row.id)?;
    let mut run = hydrate_run(row, &prompts)?;
    run.stale = !super::fingerprint::current(connection, &run.homework_id)?
        .is_current(run.inputs_fingerprint.as_deref());
    Ok(run)
}

fn hydrate_run(row: RunRow, prompts: &[RevisionPromptDto]) -> Result<ReviewRunDto, AppError> {
    let status = db_enum(
        &row.status,
        "review_runs.status",
        ReviewRunStatus::try_from_db,
    )?;
    let provenance = db_enum(
        &row.provenance,
        "review_runs.provenance",
        Provenance::try_from_db,
    )?;
    let prompt = prompts
        .iter()
        .filter(|prompt| prompt.review_run_id.as_deref() == Some(row.id.as_str()))
        .max_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        })
        .cloned();
    let result = row
        .result_json
        .as_deref()
        .map(|raw| parse_column::<StoredReviewResult>(raw, "review_runs.result_json"))
        .transpose()?;
    // Колонки критериев валидируются всегда, даже если результат построен из result_json.
    let critical: Vec<super::dto::ReviewFindingDto> =
        parse_column(&row.critical_json, "review_runs.critical_json")?;
    let unsupported: Vec<UnsupportedClaimDto> =
        parse_column(&row.unsupported_json, "review_runs.unsupported_json")?;
    let missing: Vec<ReviewMissingItemDto> =
        parse_column(&row.missing_json, "review_runs.missing_json")?;
    let verdict = optional_enum(
        row.verdict,
        "review_runs.verdict",
        ReviewVerdict::try_from_db,
    )?;
    let assessment = match (status, verdict) {
        (ReviewRunStatus::Completed, Some(verdict)) => Some(ReviewAssessmentDto {
            verdict,
            correctness: required_criterion(row.correctness, "review_runs.correctness")?,
            task_fit: required_criterion(row.task_fit, "review_runs.task_fit")?,
            course_grounding: required_criterion(
                row.course_grounding,
                "review_runs.course_grounding",
            )?,
            teacher: required_criterion(row.teacher_status, "review_runs.teacher_status")?,
            style_score: row
                .style_score
                .map(|score| small_u8(score, "review_runs.style_score"))
                .transpose()?,
            critical_count: critical.len() as u32,
            unsupported_count: unsupported.len() as u32,
            missing_required: missing.iter().filter(|item| item.required).count() as u32,
        }),
        _ => None,
    };
    if status == ReviewRunStatus::Completed
        && provenance != Provenance::Legacy
        && (assessment.is_none() || result.is_none())
    {
        return Err(BackendError::database(format!(
            "completed review run {} has no stored assessment",
            row.id
        )));
    }
    Ok(ReviewRunDto {
        kind: db_enum(&row.kind, "review_runs.kind", RunKind::try_from_db)?,
        role: db_enum(&row.role, "review_runs.role", WorkflowRole::try_from_db)?,
        provider: db_enum(&row.provider, "review_runs.provider", provider_from_db)?,
        sources_used: parse_column(&row.sources_used_json, "review_runs.sources_used_json")?,
        progress: small_u8(row.progress, "review_runs.progress")?,
        style_threshold: small_u8(row.style_threshold, "review_runs.style_threshold")?,
        result: result.map(|stored| ReviewResultDto::from_stored(stored, prompt)),
        status,
        provenance,
        assessment,
        id: row.id,
        homework_id: row.homework_id,
        version_id: row.version_id,
        source_bundle_id: row.source_bundle_id,
        step: row.step,
        started_at: row.started_at,
        finished_at: row.finished_at,
        saved_as_iteration: row.saved_as_iteration,
        error_message: row.error_message,
        inputs_fingerprint: row.inputs_fingerprint,
        stale: false,
    })
}

fn required_criterion(value: Option<String>, field: &str) -> Result<CriterionStatus, AppError> {
    let raw = value.ok_or_else(|| BackendError::database(format!("{field} is missing")))?;
    db_enum(&raw, field, CriterionStatus::try_from_db)
}

// ───────────────────────── RevisionPrompt ─────────────────────────

pub struct NewPrompt<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub kind: PromptKind,
    pub text: &'a str,
    pub created_by: VersionAuthor,
    pub based_on_version_id: Option<&'a str>,
    pub review_run_id: Option<&'a str>,
    pub addresses: &'a [String],
    pub sources: &'a [EvidenceReferenceDto],
}

pub fn insert_draft_prompt(
    connection: &Connection,
    prompt: &NewPrompt<'_>,
) -> Result<(), AppError> {
    insert_draft_prompt_with(connection, prompt, Provenance::Manual)
}

/// Черновик промта с указанным происхождением текста.
pub fn insert_draft_prompt_with(
    connection: &Connection,
    prompt: &NewPrompt<'_>,
    provenance: Provenance,
) -> Result<(), AppError> {
    let addresses = json_text(&prompt.addresses, "revision_prompts.addresses_json")?;
    let sources = json_text(&prompt.sources, "revision_prompts.sources_json")?;
    let inputs = super::fingerprint::current(connection, prompt.homework_id)?;
    connection
        .execute(
            "INSERT INTO revision_prompts(
                id, homework_id, kind, text, created_by, provenance, status, based_on_version_id,
                review_run_id, addresses_json, sources_json, sent_at, committed_at, created_at, updated_at,
                inputs_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?10, 'draft', ?6, ?7, ?8, ?9, NULL, NULL,
                       strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?11)",
            params![
                prompt.id,
                prompt.homework_id,
                prompt.kind.as_db_str(),
                prompt.text,
                prompt.created_by.as_str(),
                prompt.based_on_version_id,
                prompt.review_run_id,
                addresses,
                sources,
                provenance.as_db_str(),
                inputs.combined,
            ],
        )
        .map_err(|error| {
            map_write_error(
                error,
                "Черновик промта уже есть — зафиксируйте или измените его",
            )
        })?;
    Ok(())
}

pub fn update_draft_text(
    connection: &Connection,
    prompt_id: &str,
    text: &str,
) -> Result<(), AppError> {
    // Правка черновика — решение пользователя по текущим входным данным.
    let homework_id: String = connection
        .query_row(
            "SELECT homework_id FROM revision_prompts WHERE id = ?1",
            [prompt_id],
            |row| row.get(0),
        )
        .map_err(|error| not_found_or(error, "Промт не найден"))?;
    let inputs = super::fingerprint::current(connection, &homework_id)?;
    let changed = connection
        .execute(
            "UPDATE revision_prompts SET text = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    inputs_fingerprint = ?3
             WHERE id = ?1 AND status = 'draft'",
            params![prompt_id, text, inputs.combined],
        )
        .map_err(|error| map_write_error(error, "Зафиксированный промт неизменяем"))?;
    if changed != 1 {
        return Err(BackendError::immutable("Зафиксированный промт неизменяем"));
    }
    Ok(())
}

pub fn commit_prompt(connection: &Connection, prompt_id: &str) -> Result<(), AppError> {
    let changed = connection
        .execute(
            "UPDATE revision_prompts SET status = 'committed',
                    committed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND status = 'draft'",
            [prompt_id],
        )
        .map_err(|error| map_write_error(error, "Зафиксированный промт неизменяем"))?;
    if changed != 1 {
        return Err(BackendError::immutable("Промт уже зафиксирован"));
    }
    Ok(())
}

pub fn mark_prompt_handed_off(connection: &Connection, prompt_id: &str) -> Result<(), AppError> {
    let changed = connection
        .execute(
            "UPDATE revision_prompts SET sent_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND status = 'committed' AND sent_at IS NULL",
            [prompt_id],
        )
        .map_err(|error| map_write_error(error, "Передача промта уже отмечена"))?;
    if changed != 1 {
        return Err(BackendError::blocked(
            "Передать можно только зафиксированный и ещё не переданный промт",
        ));
    }
    Ok(())
}

const PROMPT_SELECT: &str = "SELECT id, homework_id, kind, text, created_at, created_by, based_on_version_id,
        review_run_id, addresses_json, sources_json, sent_at, status, committed_at, provenance, updated_at,
        inputs_fingerprint
    FROM revision_prompts";

fn prompt_row(row: &Row<'_>) -> Result<RevisionPromptDto, AppError> {
    let addresses_json: String = row.get(8)?;
    let sources_json: String = row.get(9)?;
    Ok(RevisionPromptDto {
        id: row.get(0)?,
        homework_id: row.get(1)?,
        kind: db_enum(
            &row.get::<_, String>(2)?,
            "revision_prompts.kind",
            PromptKind::try_from_db,
        )?,
        text: row.get(3)?,
        created_at: row.get(4)?,
        created_by: db_enum(
            &row.get::<_, String>(5)?,
            "revision_prompts.created_by",
            VersionAuthor::try_from_db,
        )?,
        based_on_version_id: row.get(6)?,
        review_run_id: row.get(7)?,
        addresses: parse_column(&addresses_json, "revision_prompts.addresses_json")?,
        sources: parse_column(&sources_json, "revision_prompts.sources_json")?,
        sent_at: row.get(10)?,
        status: db_enum(
            &row.get::<_, String>(11)?,
            "revision_prompts.status",
            PromptStatus::try_from_db,
        )?,
        committed_at: row.get(12)?,
        provenance: db_enum(
            &row.get::<_, String>(13)?,
            "revision_prompts.provenance",
            Provenance::try_from_db,
        )?,
        updated_at: row.get(14)?,
        inputs_fingerprint: row.get(15)?,
        stale: false,
    })
}

fn collect_prompts(
    connection: &Connection,
    filter: &str,
    parameter: &str,
) -> Result<Vec<RevisionPromptDto>, AppError> {
    let mut statement = connection.prepare(&format!(
        "{PROMPT_SELECT} WHERE {filter} ORDER BY created_at DESC, id DESC"
    ))?;
    let mut rows = statement.query([parameter])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        output.push(prompt_row(row)?);
    }
    Ok(output)
}

pub fn prompts(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<RevisionPromptDto>, AppError> {
    collect_prompts(connection, "homework_id = ?1", homework_id)
}

fn prompts_for_run(
    connection: &Connection,
    run_id: &str,
) -> Result<Vec<RevisionPromptDto>, AppError> {
    collect_prompts(connection, "review_run_id = ?1", run_id)
}

pub fn prompt(connection: &Connection, prompt_id: &str) -> Result<RevisionPromptDto, AppError> {
    let mut prompt = collect_prompts(connection, "id = ?1", prompt_id)?
        .into_iter()
        .next()
        .ok_or_else(|| BackendError::not_found("Промт не найден"))?;
    prompt.stale = !super::fingerprint::current(connection, &prompt.homework_id)?
        .is_current(prompt.inputs_fingerprint.as_deref());
    Ok(prompt)
}

// ───────────────────────── Преподаватель ─────────────────────────

pub struct NewTeacherComment<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub source: TeacherCommentSource,
    pub date: &'a str,
    pub question: &'a str,
    pub answer: &'a str,
    pub authority: CommentAuthority,
    pub author: &'a str,
    pub context: &'a str,
    pub official: bool,
}

pub fn insert_teacher_comment(
    connection: &Connection,
    comment: &NewTeacherComment<'_>,
) -> Result<(), AppError> {
    connection
        .execute(
            "INSERT INTO teacher_comments(
                id, homework_id, question, answer, source, source_date, created_at, date, added_at,
                authority, author, context, official_confirmed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?6,
                       strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?7, ?8, ?9,
                       CASE WHEN ?10 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END)",
            params![
                comment.id,
                comment.homework_id,
                comment.question,
                comment.answer,
                comment.source.as_db_str(),
                comment.date,
                comment.authority.as_db_str(),
                comment.author,
                comment.context,
                comment.official,
            ],
        )
        .map_err(|error| map_write_error(error, "Комментарий преподавателя не сохранён"))?;
    insert_authority_event(
        connection,
        comment.id,
        comment.homework_id,
        comment.authority,
        comment.author,
        comment.official,
    )
}

fn insert_authority_event(
    connection: &Connection,
    comment_id: &str,
    homework_id: &str,
    authority: CommentAuthority,
    author: &str,
    official: bool,
) -> Result<(), AppError> {
    connection.execute(
        "INSERT INTO comment_authority_events(id, comment_id, homework_id, authority, author, official)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            uuid::Uuid::new_v4().to_string(),
            comment_id,
            homework_id,
            authority.as_db_str(),
            author,
            official
        ],
    )?;
    Ok(())
}

/// Новые автор, роль, контекст и официальность ответа. При отзыве
/// официальности требования этого ответа отзываются (остаются в истории).
pub fn set_comment_authority(
    connection: &Connection,
    comment_id: &str,
    authority: CommentAuthority,
    author: &str,
    context: &str,
    official: bool,
) -> Result<(), AppError> {
    let homework_id: String = connection
        .query_row(
            "SELECT homework_id FROM teacher_comments WHERE id = ?1",
            [comment_id],
            |row| row.get(0),
        )
        .map_err(|error| not_found_or(error, "Комментарий не найден"))?;
    connection
        .execute(
            "UPDATE teacher_comments SET authority = ?2, author = ?3, context = ?4,
                    official_confirmed_at = CASE
                        WHEN NOT ?5 THEN NULL
                        ELSE coalesce(official_confirmed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                    END
             WHERE id = ?1",
            params![comment_id, authority.as_db_str(), author, context, official],
        )
        .map_err(|error| {
            map_write_error(
                error,
                "Официальным может быть только ответ преподавателя или ассистента",
            )
        })?;
    if !official {
        withdraw_comment_requirements(connection, comment_id)?;
    }
    insert_authority_event(
        connection,
        comment_id,
        &homework_id,
        authority,
        author,
        official,
    )
}

pub fn withdraw_comment_requirements(
    connection: &Connection,
    comment_id: &str,
) -> Result<(), AppError> {
    connection.execute(
        "UPDATE teacher_requirements
            SET withdrawn_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE comment_id = ?1 AND withdrawn_at IS NULL",
        [comment_id],
    )?;
    Ok(())
}

/// Требование удалено пользователем: хранится для истории, но не действует.
pub fn withdraw_requirement(connection: &Connection, requirement_id: &str) -> Result<(), AppError> {
    let changed = connection.execute(
        "UPDATE teacher_requirements
            SET withdrawn_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
          WHERE id = ?1 AND withdrawn_at IS NULL",
        [requirement_id],
    )?;
    if changed != 1 {
        return Err(BackendError::blocked("Требование уже отозвано"));
    }
    Ok(())
}

pub fn comment_homework(connection: &Connection, comment_id: &str) -> Result<String, AppError> {
    connection
        .query_row(
            "SELECT homework_id FROM teacher_comments WHERE id = ?1",
            [comment_id],
            |row| row.get(0),
        )
        .map_err(|error| not_found_or(error, "Комментарий не найден"))
}

/// Отпечаток входных данных, на котором отмечено «Готово к сдаче».
pub fn set_ready_fingerprint(
    connection: &Connection,
    homework_id: &str,
    fingerprint: Option<&str>,
) -> Result<(), AppError> {
    connection
        .execute(
            "UPDATE homeworks SET ready_fingerprint = ?2 WHERE id = ?1",
            params![homework_id, fingerprint],
        )
        .map_err(|error| map_write_error(error, "Отправленная работа неизменяема"))?;
    Ok(())
}

pub fn teacher_comments(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<TeacherCommentDto>, AppError> {
    let mut statement = connection.prepare(
        "SELECT c.id, c.homework_id, c.source, coalesce(c.date, c.source_date, c.created_at), c.question,
                c.answer, coalesce(c.added_at, c.created_at),
                f.id, f.name, f.kind, f.size_bytes, coalesce(f.modified_at, f.updated_at, f.created_at),
                f.relative_path, c.authority, c.author, c.context, c.official_confirmed_at
         FROM teacher_comments c LEFT JOIN files f ON f.id = c.attachment_file_id
         WHERE c.homework_id = ?1
         ORDER BY coalesce(c.date, c.source_date, c.created_at), c.id",
    )?;
    let mut rows = statement.query([homework_id])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        let source: Option<String> = row.get(2)?;
        let source =
            source.ok_or_else(|| BackendError::database("teacher_comments.source is missing"))?;
        let attachment_id: Option<String> = row.get(7)?;
        let attachment = match attachment_id {
            Some(id) => Some(hydrate_file(
                id,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
                row.get(11)?,
                row.get(12)?,
            )?),
            None => None,
        };
        output.push(TeacherCommentDto {
            id: row.get(0)?,
            homework_id: row.get(1)?,
            source: db_enum(
                &source,
                "teacher_comments.source",
                TeacherCommentSource::try_from_db,
            )?,
            date: row.get(3)?,
            question: row.get(4)?,
            answer: row.get(5)?,
            added_at: row.get(6)?,
            attachment,
            authority: db_enum(
                &row.get::<_, String>(13)?,
                "teacher_comments.authority",
                CommentAuthority::try_from_db,
            )?,
            author: row.get(14)?,
            context: row.get(15)?,
            official: row.get::<_, Option<String>>(16)?.is_some(),
            official_confirmed_at: row.get(16)?,
        });
    }
    Ok(output)
}

pub struct NewRequirement<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub comment_id: &'a str,
    pub text: &'a str,
    pub origin: &'a str,
    pub status: RequirementStatus,
    pub note: &'a str,
    pub checked_version_id: &'a str,
}

pub fn insert_requirement(
    connection: &Connection,
    requirement: &NewRequirement<'_>,
) -> Result<(), AppError> {
    let official: Option<bool> = connection
        .query_row(
            "SELECT official_confirmed_at IS NOT NULL FROM teacher_comments
              WHERE id = ?1 AND homework_id = ?2",
            params![requirement.comment_id, requirement.homework_id],
            |row| row.get(0),
        )
        .optional()?;
    match official {
        None => {
            return Err(BackendError::not_found(
                "Комментарий преподавателя не найден",
            ))
        }
        Some(false) => {
            return Err(BackendError::blocked(
                "Требование выводится только из официального ответа преподавателя или ассистента — сначала подтвердите источник",
            ))
        }
        Some(true) => {}
    }
    connection
        .execute(
            "INSERT INTO teacher_requirements(
                id, homework_id, comment_id, text, origin, status, note, checked_version_id,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                       strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                requirement.id,
                requirement.homework_id,
                requirement.comment_id,
                requirement.text,
                requirement.origin,
                requirement.status.as_str(),
                requirement.note,
                requirement.checked_version_id,
            ],
        )
        .map_err(|error| map_write_error(error, "Требование преподавателя не сохранено"))?;
    Ok(())
}

pub fn resolve_requirement(
    connection: &Connection,
    requirement_id: &str,
    status: RequirementStatus,
    note: &str,
    checked_version_id: &str,
) -> Result<(), AppError> {
    let changed = connection
        .execute(
            "UPDATE teacher_requirements SET status = ?2, note = ?3, checked_version_id = ?4,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND withdrawn_at IS NULL",
            params![requirement_id, status.as_str(), note, checked_version_id],
        )
        .map_err(|error| map_write_error(error, "Требование не обновлено"))?;
    if changed != 1 {
        return Err(BackendError::not_found("Требование не найдено"));
    }
    Ok(())
}

pub fn requirement_homework(
    connection: &Connection,
    requirement_id: &str,
) -> Result<String, AppError> {
    connection
        .query_row(
            "SELECT homework_id FROM teacher_requirements WHERE id = ?1",
            [requirement_id],
            |row| row.get(0),
        )
        .map_err(|error| not_found_or(error, "Требование не найдено"))
}

pub fn teacher_requirements(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<TeacherRequirementDto>, AppError> {
    let mut statement = connection.prepare(
        "SELECT id, text, comment_id, origin, status, note, checked_version_id, withdrawn_at
         FROM teacher_requirements WHERE homework_id = ?1 ORDER BY created_at, id",
    )?;
    let mut rows = statement.query([homework_id])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        output.push(TeacherRequirementDto {
            id: row.get(0)?,
            text: row.get(1)?,
            comment_id: row.get(2)?,
            origin: row.get(3)?,
            status: db_enum(
                &row.get::<_, String>(4)?,
                "teacher_requirements.status",
                RequirementStatus::try_from_db,
            )?,
            note: row.get(5)?,
            checked_version_id: row.get(6)?,
            withdrawn_at: row.get(7)?,
        });
    }
    Ok(output)
}

// ───────────────────────── Подтверждение стиля ─────────────────────────

pub fn insert_style_confirmation(
    connection: &Connection,
    id: &str,
    run: &ReviewRunDto,
    score: u8,
) -> Result<(), AppError> {
    let version_id = run
        .version_id
        .as_deref()
        .ok_or_else(|| BackendError::blocked("Подтверждение относится только к проверке версии"))?;
    connection
        .execute(
            "INSERT INTO style_confirmations(
                id, homework_id, version_id, review_run_id, score, threshold, confirmed_by, confirmed_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'user', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                id,
                run.homework_id,
                version_id,
                run.id,
                i64::from(score),
                i64::from(run.style_threshold),
            ],
        )
        .map_err(|error| map_write_error(error, "Расхождение для этой проверки уже подтверждено"))?;
    Ok(())
}

pub fn style_confirmations(
    connection: &Connection,
    homework_id: &str,
) -> Result<Vec<StyleConfirmationDto>, AppError> {
    let mut statement = connection.prepare(
        "SELECT id, homework_id, version_id, review_run_id, score, threshold, confirmed_by, confirmed_at
         FROM style_confirmations WHERE homework_id = ?1 ORDER BY confirmed_at, id",
    )?;
    let mut rows = statement.query([homework_id])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        let confirmed_by: String = row.get(6)?;
        if confirmed_by != "user" {
            return Err(BackendError::database(format!(
                "invalid enum in style_confirmations.confirmed_by: {confirmed_by}"
            )));
        }
        output.push(StyleConfirmationDto {
            id: row.get(0)?,
            homework_id: row.get(1)?,
            version_id: row.get(2)?,
            review_run_id: row.get(3)?,
            score: small_u8(row.get(4)?, "style_confirmations.score")?,
            threshold: small_u8(row.get(5)?, "style_confirmations.threshold")?,
            confirmed_by: "user",
            confirmed_at: row.get(7)?,
        });
    }
    Ok(output)
}

// ───────────────────────── Комплект и снимок ─────────────────────────

struct PackageRow {
    id: String,
    homework_id: String,
    version_id: String,
    folder_path: String,
    items_json: String,
    links_json: String,
    warnings_json: String,
    checklist_json: String,
    upload_list_json: String,
    built_at: String,
    inputs_fingerprint: Option<String>,
}

pub fn submission_package(
    connection: &Connection,
    homework_id: &str,
) -> Result<Option<SubmissionPackageDto>, AppError> {
    let row = connection
        .query_row(
            "SELECT id, homework_id, final_version_id, package_path, items_json, links_json,
                    warnings_json, checklist_json, upload_list_json, coalesce(built_at, created_at),
                    inputs_fingerprint
             FROM submission_packages WHERE homework_id = ?1",
            [homework_id],
            |row| {
                Ok(PackageRow {
                    id: row.get(0)?,
                    homework_id: row.get(1)?,
                    version_id: row.get(2)?,
                    folder_path: row.get(3)?,
                    items_json: row.get(4)?,
                    links_json: row.get(5)?,
                    warnings_json: row.get(6)?,
                    checklist_json: row.get(7)?,
                    upload_list_json: row.get(8)?,
                    built_at: row.get(9)?,
                    inputs_fingerprint: row.get(10)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    require_relative_path(&row.folder_path, "submission_packages.package_path")?;
    let items: Vec<PackageItemDto> =
        parse_column(&row.items_json, "submission_packages.items_json")?;
    for item in &items {
        if let Some(file) = &item.file {
            require_relative_path(&file.relative_path, "submission_packages.items_json")?;
        }
    }
    let stale = !super::fingerprint::current(connection, &row.homework_id)?
        .is_current(row.inputs_fingerprint.as_deref());
    Ok(Some(SubmissionPackageDto {
        items,
        links: parse_column::<Vec<PackageLinkDto>>(
            &row.links_json,
            "submission_packages.links_json",
        )?,
        warnings: parse_column(&row.warnings_json, "submission_packages.warnings_json")?,
        checklist: parse_column::<Vec<PackageCheckDto>>(
            &row.checklist_json,
            "submission_packages.checklist_json",
        )?,
        upload_list: parse_column(
            &row.upload_list_json,
            "submission_packages.upload_list_json",
        )?,
        id: row.id,
        homework_id: row.homework_id,
        version_id: row.version_id,
        folder_path: row.folder_path,
        built_at: row.built_at,
        stale,
        inputs_fingerprint: row.inputs_fingerprint,
    }))
}

struct ReceiptRow {
    id: String,
    homework_id: String,
    version_id: String,
    package_id: String,
    submitted_at: String,
    confirmed_at: String,
    assignment_url: String,
    proof_json: Option<String>,
    snapshot_path: String,
    files_json: String,
    snapshot_hash: String,
}

pub fn submission_receipt(
    connection: &Connection,
    homework_id: &str,
) -> Result<Option<SubmissionReceiptDto>, AppError> {
    let row = connection
        .query_row(
            "SELECT id, homework_id, submitted_version_id, package_id,
                    coalesce(submitted_at, confirmed_at), confirmed_at, assignment_url, proof_json,
                    snapshot_path, files_json, snapshot_hash
             FROM submission_receipts WHERE homework_id = ?1",
            [homework_id],
            |row| {
                Ok(ReceiptRow {
                    id: row.get(0)?,
                    homework_id: row.get(1)?,
                    version_id: row.get(2)?,
                    package_id: row.get(3)?,
                    submitted_at: row.get(4)?,
                    confirmed_at: row.get(5)?,
                    assignment_url: row.get(6)?,
                    proof_json: row.get(7)?,
                    snapshot_path: row.get(8)?,
                    files_json: row.get(9)?,
                    snapshot_hash: row.get(10)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    require_relative_path(&row.snapshot_path, "submission_receipts.snapshot_path")?;
    let proof: Option<FileRefDto> = row
        .proof_json
        .as_deref()
        .map(|raw| parse_column(raw, "submission_receipts.proof_json"))
        .transpose()?;
    let files: Vec<FileRefDto> = parse_column(&row.files_json, "submission_receipts.files_json")?;
    for file in files.iter().chain(proof.iter()) {
        require_relative_path(&file.relative_path, "submission_receipts.files_json")?;
    }
    Ok(Some(SubmissionReceiptDto {
        id: row.id,
        homework_id: row.homework_id,
        version_id: row.version_id,
        package_id: row.package_id,
        submitted_at: row.submitted_at,
        confirmed_at: row.confirmed_at,
        assignment_url: row.assignment_url,
        proof,
        snapshot_path: row.snapshot_path,
        files,
        snapshot_hash: row.snapshot_hash,
    }))
}

// ───────────────────────── Снимок всего агрегата ─────────────────────────

/// Всё состояние рабочего цикла одного ДЗ, прочитанное в одной точке времени.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSnapshot {
    pub homework: HomeworkDto,
    pub bundles: Vec<SourceBundleDto>,
    /// Для каждого набора — признаки «материал изменился после снимка» по элементам.
    pub bundle_changes: Vec<Vec<bool>>,
    pub runs: Vec<ReviewRunDto>,
    pub prompts: Vec<RevisionPromptDto>,
    pub teacher_comments: Vec<TeacherCommentDto>,
    pub teacher_requirements: Vec<TeacherRequirementDto>,
    pub style_confirmations: Vec<StyleConfirmationDto>,
    pub submission_package: Option<SubmissionPackageDto>,
    pub submission_receipt: Option<SubmissionReceiptDto>,
    /// Задания передачи провайдерам (этап 9), без текстов.
    pub ai_jobs: Vec<crate::ai::model::AiJobDto>,
    /// Этап 7: текущий отпечаток входных данных и отпечаток отметки «Готово к сдаче».
    pub inputs: super::fingerprint::InputsFingerprintDto,
    pub ready_fingerprint: Option<String>,
}

/// Фиксированная серия запросов (14), не зависящая от числа версий и запусков.
pub fn load_snapshot(
    connection: &Connection,
    homework_id: &str,
) -> Result<WorkflowSnapshot, AppError> {
    let homework = homework_dto(connection, homework_id)?;
    let (bundles, bundle_changes) = bundles(connection, homework_id)?.into_iter().unzip();
    let inputs = super::fingerprint::current(connection, homework_id)?;
    let mut prompts = prompts(connection, homework_id)?;
    for prompt in &mut prompts {
        prompt.stale = !inputs.is_current(prompt.inputs_fingerprint.as_deref());
    }
    let mut runs = runs(connection, homework_id, &prompts)?;
    for run in &mut runs {
        run.stale = !inputs.is_current(run.inputs_fingerprint.as_deref());
    }
    let ready_fingerprint: Option<String> = connection.query_row(
        "SELECT ready_fingerprint FROM homeworks WHERE id = ?1",
        [homework_id],
        |row| row.get(0),
    )?;
    Ok(WorkflowSnapshot {
        inputs,
        ready_fingerprint,
        teacher_comments: teacher_comments(connection, homework_id)?,
        teacher_requirements: teacher_requirements(connection, homework_id)?,
        style_confirmations: style_confirmations(connection, homework_id)?,
        submission_package: submission_package(connection, homework_id)?,
        submission_receipt: submission_receipt(connection, homework_id)?,
        ai_jobs: crate::ai::store::jobs_for_homework(connection, homework_id)?,
        homework,
        bundles,
        bundle_changes,
        runs,
        prompts,
    })
}

impl WorkflowSnapshot {
    pub fn bundle(&self, bundle_id: &str) -> Option<(&SourceBundleDto, &[bool])> {
        self.bundles
            .iter()
            .zip(self.bundle_changes.iter())
            .find(|(bundle, _)| bundle.id == bundle_id)
            .map(|(bundle, changes)| (bundle, changes.as_slice()))
    }

    pub fn current_version(&self) -> Option<&HomeworkVersionDto> {
        let id = self.homework.final_version_id.as_deref()?;
        self.homework
            .versions
            .iter()
            .find(|version| version.id == id)
    }

    pub fn latest_version(&self) -> Option<&HomeworkVersionDto> {
        self.homework
            .versions
            .iter()
            .max_by_key(|version| version.number)
    }

    /// Последний завершённый запуск указанного вида для версии.
    pub fn latest_completed(
        &self,
        kind: RunKind,
        version_id: Option<&str>,
    ) -> Option<&ReviewRunDto> {
        self.runs.iter().find(|run| {
            run.kind == kind
                && run.status == ReviewRunStatus::Completed
                && (version_id.is_none() || run.version_id.as_deref() == version_id)
        })
    }
}
