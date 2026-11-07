//! Этап 9: профиль личного стиля.
//!
//! - Источники выбирает только пользователь (`add_source`); папки не сканируются.
//! - Профиль — неизменяемые ревизии: новая ревизия появляется, когда меняется
//!   набор источников или их содержимое; прежние остаются в истории.
//! - Проверка версии — эвристическая оценка соответствия личному стилю с
//!   объяснимыми признаками и нехарактерными фрагментами. Это не «детектор ИИ»:
//!   низкое соответствие не доказывает использование нейросети.
//! - Состояние профиля входит в отпечаток входных данных (этап 7), поэтому его
//!   изменение делает устаревшими проверки, готовность, промты и комплекты.

pub mod features;
#[cfg(test)]
pub(crate) mod tests;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::Database,
    error::{AppError, BackendError, ErrorCode},
    extraction::{
        engine::extract_bytes, image::OcrMode, model::Budget, ExtractionLimits, ExtractionStatus,
    },
    file_service::{read_vault_file, PersistentPath, VaultRoot},
    intake::RootsService,
    workflow::{fingerprint, repository},
};

use features::{FeatureDelta, StyleFeatures, StyleFragment, MIN_CHECKED_WORDS, MIN_PROFILE_WORDS};

pub const PROFILE_ID: &str = "personal";
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleSourceKind {
    StyleMarkdown,
    PreviousHomework,
    ConfirmedSample,
}

impl StyleSourceKind {
    fn as_db(self) -> &'static str {
        match self {
            Self::StyleMarkdown => "style_markdown",
            Self::PreviousHomework => "previous_homework",
            Self::ConfirmedSample => "confirmed_sample",
        }
    }
    fn from_db(value: &str) -> Self {
        match value {
            "style_markdown" => Self::StyleMarkdown,
            "previous_homework" => Self::PreviousHomework,
            _ => Self::ConfirmedSample,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleSourceDto {
    pub id: String,
    pub relative_path: String,
    pub file_id: Option<String>,
    pub kind: StyleSourceKind,
    pub language: String,
    pub document_type: String,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
    pub confirmation: String,
    pub read_status: String,
    pub added_at: String,
    pub last_read_at: Option<String>,
    pub removed_at: Option<String>,
}

/// Источник в составе ревизии (снимок на момент сборки).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionSourceDto {
    pub id: String,
    pub relative_path: String,
    pub kind: StyleSourceKind,
    pub sha256: Option<String>,
    pub size_bytes: Option<i64>,
    pub words: usize,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileContent {
    pub features: StyleFeatures,
    /// Правила из Markdown-описания стиля (пункты списков, коротко).
    pub declared_rules: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Ready,
    InsufficientSources,
    Unsupported,
}

impl ProfileStatus {
    fn as_db(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::InsufficientSources => "insufficient_sources",
            Self::Unsupported => "unsupported",
        }
    }
    fn from_db(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "unsupported" => Self::Unsupported,
            _ => Self::InsufficientSources,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleProfileRevisionDto {
    pub id: String,
    pub profile_id: String,
    pub revision: i64,
    pub created_at: String,
    pub language: String,
    pub source_fingerprint: String,
    pub sources: Vec<RevisionSourceDto>,
    pub content: Option<ProfileContent>,
    pub limitations: Vec<String>,
    pub status: ProfileStatus,
    pub unusable_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRevisionSummaryDto {
    pub id: String,
    pub revision: i64,
    pub created_at: String,
    pub status: ProfileStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleProfileStateDto {
    pub profile_id: String,
    pub sources: Vec<StyleSourceDto>,
    pub current: Option<StyleProfileRevisionDto>,
    pub history: Vec<ProfileRevisionSummaryDto>,
    /// Источники или их содержимое изменились после последней ревизии.
    pub stale: bool,
    pub current_source_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleCheckStatus {
    Checked,
    NotChecked,
    Stale,
    InsufficientSources,
    Unsupported,
    ManualRequired,
}

impl StyleCheckStatus {
    fn as_db(self) -> &'static str {
        match self {
            Self::Checked => "checked",
            Self::NotChecked | Self::Stale => "not_checked",
            Self::InsufficientSources => "insufficient_sources",
            Self::Unsupported => "unsupported",
            Self::ManualRequired => "manual_required",
        }
    }
    fn from_db(value: &str) -> Self {
        match value {
            "checked" => Self::Checked,
            "insufficient_sources" => Self::InsufficientSources,
            "unsupported" => Self::Unsupported,
            "manual_required" => Self::ManualRequired,
            _ => Self::NotChecked,
        }
    }
}

/// Сохраняемая часть результата проверки (без копии текста работы).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleCheckBody {
    pub explanation: String,
    pub fragments: Vec<StyleFragment>,
    pub deltas: Vec<FeatureDelta>,
    /// Подтверждённые образцы из профиля, с которыми сравнивали.
    pub sample_refs: Vec<String>,
    pub sample_examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleCheckDto {
    pub id: String,
    pub homework_id: String,
    pub version_id: String,
    pub profile_revision_id: Option<String>,
    pub profile_revision: Option<i64>,
    /// `stale` — после проверки изменились профиль, версия или другие входные данные.
    pub status: StyleCheckStatus,
    /// Сохранённый статус (до учёта устаревания).
    pub recorded_status: StyleCheckStatus,
    /// Эвристическое расхождение 0..10; только при `checked`.
    pub score: Option<u8>,
    pub threshold: u8,
    pub body: StyleCheckBody,
    pub version_fingerprint: String,
    pub inputs_fingerprint: String,
    pub created_at: String,
    pub confirmed_at: Option<String>,
    /// Проверка закрывает требование стиля для готовности.
    pub passes: bool,
}

/// Текст документа для анализа: Markdown/текст — как есть, DOCX/PDF — через
/// локальное извлечение. `Err` — формат не поддерживается или текста нет.
pub fn document_text(name: &str, bytes: &[u8]) -> Result<String, String> {
    let lower = name.to_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".txt") || lower.ends_with(".markdown") {
        return String::from_utf8(bytes.to_vec()).map_err(|_| "Файл не в UTF-8".to_owned());
    }
    if !(lower.ends_with(".docx") || lower.ends_with(".pdf")) {
        return Err(
            "Формат не поддерживается для анализа стиля (нужны .md, .txt, .docx или .pdf)"
                .to_owned(),
        );
    }
    let extracted = extract_bytes(
        name,
        bytes,
        &ExtractionLimits::default(),
        &Budget::unlimited(),
        OcrMode::Disabled,
    )
    .map_err(|_| "Извлечение текста прервано".to_owned())?;
    match extracted.status {
        ExtractionStatus::Complete | ExtractionStatus::Partial
            if !extracted.text.trim().is_empty() =>
        {
            Ok(extracted.text)
        }
        ExtractionStatus::NeedsOcr => Err("В документе нет текстового слоя".to_owned()),
        _ => Err(extracted
            .status_message
            .unwrap_or_else(|| "Текст извлечь не удалось".to_owned())),
    }
}

fn document_type(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        "markdown"
    } else if lower.ends_with(".txt") {
        "text"
    } else if lower.ends_with(".docx") {
        "docx"
    } else if lower.ends_with(".pdf") {
        "pdf"
    } else {
        "other"
    }
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn json<T: Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value).map_err(|error| BackendError::database(error.to_string()))
}

fn parse<T: for<'de> Deserialize<'de>>(raw: &str, column: &str) -> Result<T, AppError> {
    serde_json::from_str(raw)
        .map_err(|error| BackendError::database(format!("invalid {column}: {error}")))
}

fn validate_language(language: &str) -> Result<(), AppError> {
    if language == "ru" || language == "en" {
        Ok(())
    } else {
        Err(BackendError::conflict("Язык профиля: ru или en"))
    }
}

const SOURCE_SELECT: &str = "SELECT id, relative_path, file_id, source_kind, language, document_type,
    sha256, size_bytes, confirmation, read_status, added_at, last_read_at, removed_at FROM style_sources";

fn source_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StyleSourceDto> {
    Ok(StyleSourceDto {
        id: row.get(0)?,
        relative_path: row.get(1)?,
        file_id: row.get(2)?,
        kind: StyleSourceKind::from_db(&row.get::<_, String>(3)?),
        language: row.get(4)?,
        document_type: row.get(5)?,
        sha256: row.get(6)?,
        size_bytes: row.get(7)?,
        confirmation: row.get(8)?,
        read_status: row.get(9)?,
        added_at: row.get(10)?,
        last_read_at: row.get(11)?,
        removed_at: row.get(12)?,
    })
}

pub fn sources(
    connection: &Connection,
    active_only: bool,
) -> Result<Vec<StyleSourceDto>, AppError> {
    let filter = if active_only {
        " WHERE removed_at IS NULL"
    } else {
        ""
    };
    let mut statement =
        connection.prepare(&format!("{SOURCE_SELECT}{filter} ORDER BY added_at, id"))?;
    let rows = statement
        .query_map([], source_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Отпечаток текущего набора подтверждённых источников и их содержимого.
pub fn source_fingerprint(connection: &Connection) -> Result<String, AppError> {
    let mut hasher = Sha256::new();
    for source in sources(connection, true)? {
        if source.confirmation != "confirmed" {
            continue;
        }
        let line = format!(
            "{}|{}|{}|{}|{}|{}|{}\n",
            source.id,
            source.relative_path,
            source.kind.as_db(),
            source.language,
            source.sha256.as_deref().unwrap_or(""),
            source.size_bytes.unwrap_or(-1),
            source.read_status
        );
        hasher.update(line.as_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

const REVISION_SELECT: &str = "SELECT id, profile_id, revision, created_at, language, source_fingerprint,
    sources_json, features_json, limitations_json, status, unusable_reasons_json FROM style_profile_revisions";

fn revision_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<[Option<String>; 11]> {
    Ok([
        row.get(0)?,
        row.get(1)?,
        Some(row.get::<_, i64>(2)?.to_string()),
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
    ])
}

fn hydrate_revision(raw: [Option<String>; 11]) -> Result<StyleProfileRevisionDto, AppError> {
    let [id, profile_id, revision, created_at, language, source_fingerprint, sources, content, limitations, status, reasons] =
        raw;
    let text = |value: Option<String>| value.unwrap_or_default();
    Ok(StyleProfileRevisionDto {
        id: text(id),
        profile_id: text(profile_id),
        revision: text(revision).parse().unwrap_or(0),
        created_at: text(created_at),
        language: text(language),
        source_fingerprint: text(source_fingerprint),
        sources: parse(&text(sources), "style_profile_revisions.sources_json")?,
        content: content
            .map(|raw| parse(&raw, "style_profile_revisions.features_json"))
            .transpose()?,
        limitations: parse(
            &text(limitations),
            "style_profile_revisions.limitations_json",
        )?,
        status: ProfileStatus::from_db(&text(status)),
        unusable_reasons: parse(
            &text(reasons),
            "style_profile_revisions.unusable_reasons_json",
        )?,
    })
}

pub fn latest_revision(
    connection: &Connection,
) -> Result<Option<StyleProfileRevisionDto>, AppError> {
    connection
        .query_row(
            &format!("{REVISION_SELECT} WHERE profile_id = ?1 ORDER BY revision DESC LIMIT 1"),
            [PROFILE_ID],
            revision_row,
        )
        .optional()?
        .map(hydrate_revision)
        .transpose()
}

fn revision_by_id(
    connection: &Connection,
    id: &str,
) -> Result<Option<StyleProfileRevisionDto>, AppError> {
    connection
        .query_row(
            &format!("{REVISION_SELECT} WHERE id = ?1"),
            [id],
            revision_row,
        )
        .optional()?
        .map(hydrate_revision)
        .transpose()
}

pub fn state(connection: &Connection) -> Result<StyleProfileStateDto, AppError> {
    let current = latest_revision(connection)?;
    let fingerprint = source_fingerprint(connection)?;
    let mut statement = connection.prepare(
        "SELECT id, revision, created_at, status FROM style_profile_revisions
          WHERE profile_id = ?1 ORDER BY revision DESC",
    )?;
    let history = statement
        .query_map([PROFILE_ID], |row| {
            Ok(ProfileRevisionSummaryDto {
                id: row.get(0)?,
                revision: row.get(1)?,
                created_at: row.get(2)?,
                status: ProfileStatus::from_db(&row.get::<_, String>(3)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StyleProfileStateDto {
        profile_id: PROFILE_ID.to_owned(),
        sources: sources(connection, false)?,
        stale: current
            .as_ref()
            .is_some_and(|revision| revision.source_fingerprint != fingerprint),
        current,
        history,
        current_source_fingerprint: fingerprint,
    })
}

const CHECK_SELECT: &str =
    "SELECT c.id, c.homework_id, c.version_id, c.profile_revision_id, r.revision,
    c.status, c.score, c.threshold, c.result_json, c.version_fingerprint, c.inputs_fingerprint,
    c.created_at, k.confirmed_at
    FROM style_checks c
    LEFT JOIN style_profile_revisions r ON r.id = c.profile_revision_id
    LEFT JOIN style_check_confirmations k ON k.style_check_id = c.id";

fn hydrate_check(
    connection: &Connection,
    row: &rusqlite::Row<'_>,
) -> Result<StyleCheckDto, AppError> {
    let homework_id: String = row.get(1)?;
    let version_id: String = row.get(2)?;
    let recorded = StyleCheckStatus::from_db(&row.get::<_, String>(5)?);
    let inputs_fingerprint: String = row.get(10)?;
    let final_version = repository::homework_core(connection, &homework_id)?.final_version_id;
    let stale = final_version.as_deref() != Some(version_id.as_str())
        || !fingerprint::current(connection, &homework_id)?.is_current(Some(&inputs_fingerprint));
    let score: Option<i64> = row.get(6)?;
    let threshold: i64 = row.get(7)?;
    let confirmed_at: Option<String> = row.get(12)?;
    let score = score.map(|value| value.clamp(0, 10) as u8);
    let threshold = threshold.clamp(0, 10) as u8;
    let status = if stale {
        StyleCheckStatus::Stale
    } else {
        recorded
    };
    let passes = match status {
        StyleCheckStatus::Checked => {
            score.is_some_and(|s| s <= threshold) || confirmed_at.is_some()
        }
        StyleCheckStatus::ManualRequired
        | StyleCheckStatus::InsufficientSources
        | StyleCheckStatus::Unsupported => confirmed_at.is_some(),
        StyleCheckStatus::NotChecked | StyleCheckStatus::Stale => false,
    };
    Ok(StyleCheckDto {
        id: row.get(0)?,
        profile_revision_id: row.get(3)?,
        profile_revision: row.get(4)?,
        status,
        recorded_status: recorded,
        score,
        threshold,
        body: parse(&row.get::<_, String>(8)?, "style_checks.result_json")?,
        version_fingerprint: row.get(9)?,
        inputs_fingerprint,
        created_at: row.get(11)?,
        confirmed_at,
        passes,
        homework_id,
        version_id,
    })
}

/// Последняя проверка стиля ДЗ (с учётом устаревания).
pub fn latest_check(
    connection: &Connection,
    homework_id: &str,
) -> Result<Option<StyleCheckDto>, AppError> {
    let mut statement = connection.prepare(&format!(
        "{CHECK_SELECT} WHERE c.homework_id = ?1 ORDER BY c.created_at DESC, c.rowid DESC LIMIT 1"
    ))?;
    let mut rows = statement.query([homework_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(hydrate_check(connection, row)?)),
        None => Ok(None),
    }
}

pub fn check_by_id(connection: &Connection, id: &str) -> Result<StyleCheckDto, AppError> {
    let mut statement = connection.prepare(&format!("{CHECK_SELECT} WHERE c.id = ?1"))?;
    let mut rows = statement.query([id])?;
    match rows.next()? {
        Some(row) => hydrate_check(connection, row),
        None => Err(BackendError::not_found("Проверка стиля не найдена")),
    }
}

pub struct StyleService<'a> {
    database: &'a Database,
    roots: RootsService<'a>,
}

struct ReadSource {
    status: &'static str,
    text: Option<String>,
}

impl<'a> StyleService<'a> {
    pub fn new(database: &'a Database, home: Option<std::path::PathBuf>) -> Self {
        Self {
            database,
            roots: RootsService::new(database, home),
        }
    }

    pub fn state(&self) -> Result<StyleProfileStateDto, AppError> {
        self.database.with_connection(state)
    }

    /// Явный выбор источника пользователем. Файл должен быть внутри vault.
    pub fn add_source(
        &self,
        relative_path: &str,
        kind: StyleSourceKind,
        language: &str,
    ) -> Result<StyleProfileStateDto, AppError> {
        validate_language(language)?;
        let path = PersistentPath::new(relative_path.trim())?;
        let vault = self.roots.open_vault()?;
        let snapshot =
            read_vault_file(&vault, path.clone(), MAX_SOURCE_BYTES).map_err(|error| {
                if error.code == ErrorCode::NotFound {
                    BackendError::not_found("Файл не найден в хранилище")
                } else {
                    error
                }
            })?;
        let doc_type = document_type(path.as_str());
        if kind == StyleSourceKind::StyleMarkdown && doc_type != "markdown" && doc_type != "text" {
            return Err(BackendError::conflict(
                "Описание стиля — Markdown или текстовый файл",
            ));
        }
        self.database.with_transaction(|tx| {
            let file_id: Option<String> = tx
                .query_row(
                    "SELECT id FROM files WHERE relative_path = ?1 LIMIT 1",
                    [path.as_str()],
                    |r| r.get(0),
                )
                .optional()?;
            tx.execute(
                "INSERT INTO style_sources(id, relative_path, file_id, source_kind, language, document_type,
                    sha256, size_bytes, confirmation, read_status, last_read_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'confirmed', 'unread', ?9)",
                params![
                    Uuid::new_v4().to_string(),
                    path.as_str(),
                    file_id,
                    kind.as_db(),
                    language,
                    doc_type,
                    snapshot.sha256.as_str(),
                    i64::try_from(snapshot.size_bytes).unwrap_or(i64::MAX),
                    now(),
                ],
            )
            .map_err(|error| match error {
                rusqlite::Error::SqliteFailure(_, Some(ref message)) if message.contains("UNIQUE") => {
                    BackendError::conflict("Этот файл уже выбран источником")
                }
                other => other.into(),
            })?;
            state(tx)
        })
    }

    /// Источник исключается из следующей ревизии; прежние ревизии не меняются.
    pub fn remove_source(&self, id: &str) -> Result<StyleProfileStateDto, AppError> {
        self.database.with_transaction(|tx| {
            let changed = tx.execute(
                "UPDATE style_sources SET removed_at = ?2 WHERE id = ?1 AND removed_at IS NULL",
                params![id, now()],
            )?;
            if changed != 1 {
                return Err(BackendError::not_found(
                    "Источник не найден или уже исключён",
                ));
            }
            state(tx)
        })
    }

    fn read_source(
        vault: &VaultRoot,
        source: &StyleSourceDto,
    ) -> (Option<(String, i64)>, ReadSource) {
        let Ok(path) = PersistentPath::new(&source.relative_path) else {
            return (
                None,
                ReadSource {
                    status: "missing",
                    text: None,
                },
            );
        };
        match read_vault_file(vault, path, MAX_SOURCE_BYTES) {
            Ok(snapshot) => {
                let meta = Some((
                    snapshot.sha256.as_str().to_owned(),
                    i64::try_from(snapshot.size_bytes).unwrap_or(i64::MAX),
                ));
                let Some(bytes) = snapshot.bytes else {
                    return (
                        meta,
                        ReadSource {
                            status: "unsupported",
                            text: None,
                        },
                    );
                };
                match document_text(&source.relative_path, &bytes) {
                    Ok(text) if !text.trim().is_empty() => (
                        meta,
                        ReadSource {
                            status: "ok",
                            text: Some(text),
                        },
                    ),
                    Ok(_) => (
                        meta,
                        ReadSource {
                            status: "empty",
                            text: None,
                        },
                    ),
                    Err(_) => (
                        meta,
                        ReadSource {
                            status: "unsupported",
                            text: None,
                        },
                    ),
                }
            }
            Err(_) => (
                None,
                ReadSource {
                    status: "missing",
                    text: None,
                },
            ),
        }
    }

    /// Перечитывает активные источники: SHA-256, размер, статус и время чтения.
    /// Изменённое содержимое меняет отпечаток — профиль становится устаревшим.
    pub fn refresh_sources(&self) -> Result<StyleProfileStateDto, AppError> {
        self.refresh_inner().map(|_| ())?;
        self.state()
    }

    fn refresh_inner(&self) -> Result<Vec<(StyleSourceDto, Option<String>)>, AppError> {
        let active = self.database.with_connection(|c| sources(c, true))?;
        if active.is_empty() {
            return Ok(Vec::new());
        }
        let vault = self.roots.open_vault()?;
        let mut out = Vec::new();
        let stamp = now();
        self.database.with_transaction(|tx| {
            for source in &active {
                let (meta, read) = Self::read_source(&vault, source);
                tx.execute(
                    "UPDATE style_sources SET sha256 = ?2, size_bytes = ?3, read_status = ?4,
                            last_read_at = CASE WHEN ?4 = 'missing' THEN last_read_at ELSE ?5 END
                      WHERE id = ?1",
                    params![
                        source.id,
                        meta.as_ref().map(|m| m.0.clone()),
                        meta.as_ref().map(|m| m.1),
                        read.status,
                        stamp,
                    ],
                )?;
                out.push((source.id.clone(), read.text));
            }
            Ok(())
        })?;
        let refreshed = self.database.with_connection(|c| sources(c, true))?;
        Ok(refreshed
            .into_iter()
            .map(|source| {
                let text = out
                    .iter()
                    .find(|(id, _)| *id == source.id)
                    .and_then(|(_, text)| text.clone());
                (source, text)
            })
            .collect())
    }

    /// Строит новую ревизию, если изменились источники, их содержимое или язык.
    pub fn rebuild(&self, language: &str) -> Result<StyleProfileStateDto, AppError> {
        validate_language(language)?;
        let read = self.refresh_inner()?;
        let fingerprint = self.database.with_connection(source_fingerprint)?;
        if let Some(latest) = self.database.with_connection(latest_revision)? {
            if latest.source_fingerprint == fingerprint && latest.language == language {
                return self.state();
            }
        }
        let mut parts = Vec::new();
        let mut declared = Vec::new();
        let mut revision_sources = Vec::new();
        let mut limitations = vec![
            "Эвристическая оценка по объяснимым признакам текста; не доказывает использование ИИ и не определяет авторство".to_owned(),
            "Формулы, таблицы и код в признаках почти не учитываются".to_owned(),
        ];
        for (source, text) in read.iter().filter(|(s, _)| s.confirmation == "confirmed") {
            let mut words = 0;
            if let Some(text) = text {
                if source.kind == StyleSourceKind::StyleMarkdown {
                    declared.extend(features::declared_rules(text));
                } else if source.language == language {
                    let features = features::analyze(text, language, true);
                    words = features.words;
                    parts.push(features);
                } else {
                    limitations.push(format!(
                        "«{}» на другом языке ({}) — не использован",
                        source.relative_path, source.language
                    ));
                }
            } else {
                limitations.push(format!(
                    "«{}» не прочитан: {}",
                    source.relative_path,
                    match source.read_status.as_str() {
                        "missing" => "файл не найден",
                        "empty" => "в файле нет текста",
                        _ => "формат не поддерживается",
                    }
                ));
            }
            revision_sources.push(RevisionSourceDto {
                id: source.id.clone(),
                relative_path: source.relative_path.clone(),
                kind: source.kind,
                sha256: source.sha256.clone(),
                size_bytes: source.size_bytes,
                words,
                status: source.read_status.clone(),
            });
        }
        let merged = features::merge(&parts);
        let total_words = merged.as_ref().map_or(0, |f| f.words);
        let any_read = read.iter().any(|(_, text)| text.is_some());
        let mut reasons = Vec::new();
        let status = if !read.is_empty() && !any_read {
            reasons.push("Ни один выбранный источник не удалось прочитать".to_owned());
            ProfileStatus::Unsupported
        } else if total_words < MIN_PROFILE_WORDS {
            reasons.push(format!(
                "В подтверждённых работах {total_words} слов на выбранном языке — нужно не меньше {MIN_PROFILE_WORDS}"
            ));
            if parts.is_empty() {
                reasons.push("Добавьте предыдущие домашние работы или подтверждённые образцы: описание стиля само по себе не даёт признаков".to_owned());
            }
            ProfileStatus::InsufficientSources
        } else {
            ProfileStatus::Ready
        };
        let content = match (status, merged) {
            (ProfileStatus::Ready, Some(features)) => Some(ProfileContent {
                features,
                declared_rules: declared,
            }),
            _ => None,
        };
        self.database.with_transaction(|tx| {
            let revision: i64 = tx.query_row(
                "SELECT coalesce(max(revision), 0) + 1 FROM style_profile_revisions WHERE profile_id = ?1",
                [PROFILE_ID],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO style_profile_revisions(id, profile_id, revision, language, source_fingerprint,
                    sources_json, features_json, limitations_json, status, unusable_reasons_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    Uuid::new_v4().to_string(),
                    PROFILE_ID,
                    revision,
                    language,
                    fingerprint,
                    json(&revision_sources)?,
                    content.as_ref().map(json).transpose()?,
                    json(&limitations)?,
                    status.as_db(),
                    json(&reasons)?,
                ],
            )?;
            state(tx)
        })
    }

    /// Проверка итоговой версии ДЗ по текущей ревизии профиля.
    pub fn check(&self, homework_id: &str) -> Result<StyleCheckDto, AppError> {
        // Содержимое источников могло измениться — перечитываем до проверки.
        let has_sources = self
            .database
            .with_connection(|c| sources(c, true))?
            .into_iter()
            .any(|s| s.confirmation == "confirmed");
        if has_sources {
            self.refresh_inner()?;
        }
        let (core, threshold, profile, fresh_profile) = self.database.with_connection(|c| {
            let core = repository::homework_core(c, homework_id)?;
            if core.stage == crate::domain::HomeworkStage::Submitted {
                return Err(BackendError::immutable("Отправленная работа неизменяема"));
            }
            let rules = crate::repositories::load_automation_rules(c)?;
            let threshold = u8::try_from(rules.workflow.style_threshold.clamp(0, 10)).unwrap_or(4);
            let profile = latest_revision(c)?;
            let fresh = profile
                .as_ref()
                .is_some_and(|p| p.source_fingerprint == source_fingerprint(c).unwrap_or_default());
            Ok((core, threshold, profile, fresh))
        })?;
        let version_id = core
            .final_version_id
            .clone()
            .ok_or_else(|| BackendError::blocked("Сначала выберите итоговую версию"))?;
        let (version_path, version_name): (String, String) =
            self.database.with_connection(|c| {
                Ok(c.query_row(
                    "SELECT file_path, file_name FROM homework_versions WHERE id = ?1",
                    [&version_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })?;

        let mut body = StyleCheckBody {
            explanation: String::new(),
            fragments: Vec::new(),
            deltas: Vec::new(),
            sample_refs: Vec::new(),
            sample_examples: Vec::new(),
        };
        let mut score = None;
        let mut version_fingerprint = format!("{version_id}|unread");
        let status = match &profile {
            None => {
                body.explanation = "Профиль стиля не построен: выберите подтверждённые работы и постройте профиль. Требуется ручная проверка.".to_owned();
                StyleCheckStatus::InsufficientSources
            }
            Some(_) if !fresh_profile => {
                body.explanation = "Источники профиля изменились после последней ревизии — обновите профиль и проверьте снова".to_owned();
                StyleCheckStatus::NotChecked
            }
            Some(p) if p.status != ProfileStatus::Ready => {
                body.explanation = format!(
                    "Профиль ревизии {} непригоден: {}. Требуется ручная проверка.",
                    p.revision,
                    p.unusable_reasons.join("; ")
                );
                if p.status == ProfileStatus::Unsupported {
                    StyleCheckStatus::Unsupported
                } else {
                    StyleCheckStatus::InsufficientSources
                }
            }
            Some(p) => {
                let content = p.content.as_ref().expect("ready profile has content");
                body.sample_refs = p
                    .sources
                    .iter()
                    .filter(|s| s.words > 0)
                    .map(|s| s.relative_path.clone())
                    .collect();
                body.sample_examples = content.features.examples.clone();
                let vault = self.roots.open_vault()?;
                let read = PersistentPath::new(&version_path)
                    .and_then(|path| read_vault_file(&vault, path, MAX_SOURCE_BYTES));
                match read {
                    Err(error) => {
                        body.explanation = format!(
                            "Файл версии не прочитан: {}. Требуется ручная проверка.",
                            error.message
                        );
                        StyleCheckStatus::ManualRequired
                    }
                    Ok(snapshot) => {
                        version_fingerprint = snapshot.sha256.as_str().to_owned();
                        match snapshot
                            .bytes
                            .as_deref()
                            .map(|b| document_text(&version_name, b))
                        {
                            Some(Ok(text)) if features::words(&text).len() >= MIN_CHECKED_WORDS => {
                                let result =
                                    features::compare(&content.features, &text, &p.language);
                                score = Some(result.divergence);
                                body.deltas = result.deltas;
                                body.fragments = result.fragments;
                                body.explanation = format!(
                                    "Эвристическая оценка расхождения с личным стилем: {}/10 при пороге {threshold}. Низкое соответствие не доказывает использование ИИ.",
                                    result.divergence
                                );
                                StyleCheckStatus::Checked
                            }
                            Some(Ok(_)) => {
                                body.explanation = format!("В версии меньше {MIN_CHECKED_WORDS} слов текста — оценка ненадёжна. Требуется ручная проверка.");
                                StyleCheckStatus::ManualRequired
                            }
                            Some(Err(reason)) => {
                                body.explanation = format!("{reason}. Требуется ручная проверка.");
                                StyleCheckStatus::ManualRequired
                            }
                            None => {
                                body.explanation = "Файл версии слишком большой для анализа. Требуется ручная проверка.".to_owned();
                                StyleCheckStatus::ManualRequired
                            }
                        }
                    }
                }
            }
        };
        let id = Uuid::new_v4().to_string();
        self.database.with_transaction(|tx| {
            let inputs = fingerprint::current(tx, homework_id)?;
            tx.execute(
                "INSERT INTO style_checks(id, homework_id, version_id, profile_revision_id, status, score,
                    threshold, result_json, version_fingerprint, inputs_fingerprint)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    homework_id,
                    version_id,
                    profile.as_ref().map(|p| p.id.clone()),
                    status.as_db(),
                    score.map(i64::from),
                    i64::from(threshold),
                    json(&body)?,
                    version_fingerprint,
                    inputs.combined,
                ],
            )?;
            Ok(())
        })?;
        self.database.with_connection(|c| check_by_id(c, &id))
    }

    /// Ручное подтверждение: расхождение выше порога или проверка невозможна.
    pub fn confirm(&self, check_id: &str, note: &str) -> Result<StyleCheckDto, AppError> {
        self.database.with_transaction(|tx| {
            let check = check_by_id(tx, check_id)?;
            if check.confirmed_at.is_some() {
                return Err(BackendError::conflict("Проверка уже подтверждена"));
            }
            let allowed = match check.status {
                StyleCheckStatus::Checked => check.score.is_some_and(|s| s > check.threshold),
                StyleCheckStatus::ManualRequired
                | StyleCheckStatus::InsufficientSources
                | StyleCheckStatus::Unsupported => true,
                StyleCheckStatus::NotChecked | StyleCheckStatus::Stale => false,
            };
            if !allowed {
                return Err(BackendError::blocked(
                    "Подтверждать нечего: проверка устарела, не выполнена или расхождение в пределах порога",
                ));
            }
            tx.execute(
                "INSERT INTO style_check_confirmations(id, style_check_id, note) VALUES (?1, ?2, ?3)",
                params![Uuid::new_v4().to_string(), check_id, note.trim()],
            )?;
            check_by_id(tx, check_id)
        })
    }

    pub fn revision(&self, id: &str) -> Result<Option<StyleProfileRevisionDto>, AppError> {
        self.database.with_connection(|c| revision_by_id(c, id))
    }
}
