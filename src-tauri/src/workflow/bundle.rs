//! SourceBundle из подтверждённой подборки этапа 7.
//!
//! Подборка (`corpus_selections`) — пользовательский выбор материалов.
//! SourceBundle — неизменяемый снимок этой подборки для конкретного запуска:
//! материалы, SHA-256, результат извлечения и версия правил, порядок, время и
//! предупреждения корпуса. Второго механизма выбора материалов нет.
//!
//! Создание идёт в три шага: план читается из БД, содержимое сверяется с диском
//! (без удержания соединения), затем в транзакции план перечитывается и обязан
//! совпасть — иначе `conflict` (подборка изменилась во время проверки).

use std::path::PathBuf;

use rusqlite::Connection;

use crate::{
    db::Database,
    dto::FileKind,
    error::{AppError, BackendError, ErrorCode},
    extraction::{
        dto::MaterialKind, service::MAX_CORPUS_CHARS, ExtractionStatus, MaterialRef,
        EXTRACTOR_VERSION,
    },
    file_service::{read_vault_file, PersistentPath},
    intake::RootsService,
    repositories::require_relative_path,
};

use super::dto::{BundleWarningDto, MissingSourceDto, RunKind, SourceKind};

/// Результат сверки содержимого материала с закреплённым SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentCheck {
    Same,
    Changed,
    Missing,
    Unavailable(String),
}

/// Сверка текущего содержимого материала на диске. Production-реализация читает
/// файл хранилища fd-relative; тесты подставляют свою.
pub trait ContentVerifier {
    fn check(&self, item: &PlannedItem) -> ContentCheck;
}

/// Проверка по файлам хранилища (только чтение, потоковый SHA-256).
pub struct VaultContentVerifier<'a> {
    database: &'a Database,
    home: Option<PathBuf>,
}

impl<'a> VaultContentVerifier<'a> {
    pub fn new(database: &'a Database, home: Option<PathBuf>) -> Self {
        Self { database, home }
    }
}

impl ContentVerifier for VaultContentVerifier<'_> {
    fn check(&self, item: &PlannedItem) -> ContentCheck {
        let vault = match RootsService::new(self.database, self.home.clone()).open_vault() {
            Ok(vault) => vault,
            Err(error) => return ContentCheck::Unavailable(error.message),
        };
        let relative = match PersistentPath::new(&item.relative_path) {
            Ok(path) => path,
            Err(error) => return ContentCheck::Unavailable(error.message),
        };
        match read_vault_file(&vault, relative, 0) {
            Ok(snapshot) if snapshot.sha256.as_str() == item.sha256 => ContentCheck::Same,
            Ok(_) => ContentCheck::Changed,
            Err(error) if error.code == ErrorCode::NotFound => ContentCheck::Missing,
            Err(error) => ContentCheck::Unavailable(error.message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedItem {
    pub material: MaterialRef,
    pub kind: SourceKind,
    pub required: bool,
    pub name: String,
    pub file_kind: FileKind,
    pub size_bytes: i64,
    pub modified_at: String,
    pub relative_path: String,
    pub sha256: String,
    pub extraction_id: String,
    pub extractor_version: u32,
    pub extraction_status: ExtractionStatus,
    pub text_chars: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundlePlan {
    pub selection_id: String,
    pub selection_updated_at: String,
    pub items: Vec<PlannedItem>,
    pub missing: Vec<MissingSourceDto>,
    pub warnings: Vec<BundleWarningDto>,
    pub total_chars: u64,
    pub limit_chars: u64,
    pub complete: bool,
}

struct SelectionItemRow {
    material: MaterialRef,
    pinned: String,
    present: bool,
    name: Option<String>,
    kind: Option<String>,
    size_bytes: Option<i64>,
    modified_at: Option<String>,
    relative_path: Option<String>,
    inbox_sha: Option<String>,
    category: Option<String>,
    link_sha: Option<String>,
    extraction_id: Option<String>,
    extractor_version: Option<i64>,
    status: Option<String>,
    text_chars: Option<i64>,
    truncated: Option<i64>,
    warnings_json: Option<String>,
}

fn blocked(name: &str, problem: &str) -> AppError {
    BackendError::blocked(format!("«{name}»: {problem}"))
}

/// План снимка подборки ДЗ. Любой невалидный материал блокирует создание.
pub fn plan_bundle(
    connection: &Connection,
    homework_id: &str,
    purpose: RunKind,
) -> Result<BundlePlan, AppError> {
    let selection: Option<(String, String)> =
        rusqlite::OptionalExtension::optional(connection.query_row(
            "SELECT id, updated_at FROM corpus_selections WHERE homework_id = ?1",
            [homework_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ))?;
    let Some((selection_id, selection_updated_at)) = selection else {
        return Err(BackendError::blocked(
            "Сначала выберите материалы ДЗ в подборке (вкладка «Материалы»)",
        ));
    };
    let mut statement = connection.prepare(
        "SELECT ci.material_kind, ci.material_id, ci.sha256,
                CASE ci.material_kind WHEN 'inbox' THEN x.id IS NOT NULL ELSE f.id IS NOT NULL END,
                coalesce(x.name, f.name), coalesce(x.kind, f.kind), coalesce(x.size_bytes, f.size_bytes),
                coalesce(x.added_at, f.modified_at, f.updated_at, f.created_at),
                coalesce(x.relative_path, f.relative_path), x.sha256,
                CASE ci.material_kind
                    WHEN 'inbox' THEN json_extract(x.classification_json, '$.category')
                    ELSE f.category
                END,
                m.sha256, e.id, e.extractor_version, e.status, e.text_chars, e.truncated, e.warnings_json
         FROM corpus_selection_items ci
         LEFT JOIN inbox_files x ON ci.material_kind = 'inbox' AND x.id = ci.material_id
         LEFT JOIN files f ON ci.material_kind = 'file' AND f.id = ci.material_id
         LEFT JOIN material_extractions m
                ON m.material_kind = ci.material_kind AND m.material_id = ci.material_id
         LEFT JOIN extractions e ON e.id = m.extraction_id
         WHERE ci.selection_id = ?1
         ORDER BY ci.position, ci.material_kind, ci.material_id",
    )?;
    let mut rows = statement.query([&selection_id])?;
    let mut raw = Vec::new();
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        raw.push(SelectionItemRow {
            material: MaterialRef {
                kind: MaterialKind::try_from_db(&kind).ok_or_else(|| {
                    BackendError::database(format!(
                        "invalid enum in corpus_selection_items.material_kind: {kind}"
                    ))
                })?,
                id: row.get(1)?,
            },
            pinned: row.get(2)?,
            present: row.get(3)?,
            name: row.get(4)?,
            kind: row.get(5)?,
            size_bytes: row.get(6)?,
            modified_at: row.get(7)?,
            relative_path: row.get(8)?,
            inbox_sha: row.get(9)?,
            category: row.get(10)?,
            link_sha: row.get(11)?,
            extraction_id: row.get(12)?,
            extractor_version: row.get(13)?,
            status: row.get(14)?,
            text_chars: row.get(15)?,
            truncated: row.get(16)?,
            warnings_json: row.get(17)?,
        });
    }
    if raw.is_empty() {
        return Err(BackendError::blocked(
            "Подборка материалов пуста — добавьте материалы перед созданием набора",
        ));
    }

    let mut items = Vec::with_capacity(raw.len());
    let mut warnings = Vec::new();
    let mut total_chars = 0_u64;
    for row in raw {
        let item = validate_row(row, &mut warnings)?;
        total_chars += item.text_chars;
        items.push(item);
    }
    if total_chars > MAX_CORPUS_CHARS {
        warnings.push(BundleWarningDto {
            code: "corpus_truncated".into(),
            message: format!(
                "Корпус {total_chars} символов больше лимита {MAX_CORPUS_CHARS}: анализ получит только начало"
            ),
            material_id: None,
        });
    }
    let mut missing = Vec::new();
    let needs_task = matches!(purpose, RunKind::AnalyzeMaterials | RunKind::ReviewVersion);
    if needs_task && !items.iter().any(|item| item.kind == SourceKind::Task) {
        missing.push(MissingSourceDto {
            kind: SourceKind::Task,
            label: "Условие задания".into(),
            required: true,
            hint: "Добавьте в подборку файл условия ДЗ (категория «Условие и данные»)".into(),
        });
    }
    if !items
        .iter()
        .any(|item| matches!(item.kind, SourceKind::Lecture | SourceKind::Seminar))
    {
        missing.push(MissingSourceDto {
            kind: SourceKind::Lecture,
            label: "Материалы курса".into(),
            required: false,
            hint: "Без лекций и семинаров нельзя проверить опору на материалы курса".into(),
        });
    }
    let complete = missing.iter().all(|entry| !entry.required);
    Ok(BundlePlan {
        selection_id,
        selection_updated_at,
        items,
        missing,
        warnings,
        total_chars,
        limit_chars: MAX_CORPUS_CHARS,
        complete,
    })
}

fn validate_row(
    row: SelectionItemRow,
    warnings: &mut Vec<BundleWarningDto>,
) -> Result<PlannedItem, AppError> {
    let label = row.name.clone().unwrap_or_else(|| row.material.id.clone());
    if !row.present {
        return Err(blocked(&label, "материал удалён — обновите подборку"));
    }
    let (Some(name), Some(raw_kind), Some(size_bytes), Some(modified_at), Some(relative_path)) = (
        row.name,
        row.kind,
        row.size_bytes,
        row.modified_at,
        row.relative_path,
    ) else {
        return Err(BackendError::database(
            "selection material row is incomplete",
        ));
    };
    require_relative_path(&relative_path, "corpus material relative_path")?;
    if row
        .inbox_sha
        .as_deref()
        .is_some_and(|current| current != row.pinned)
    {
        return Err(blocked(
            &name,
            "файл изменился после выбора — выберите его заново",
        ));
    }
    match row.link_sha.as_deref() {
        None => return Err(blocked(&name, "текст ещё не извлечён")),
        Some(link) if link != row.pinned => {
            return Err(blocked(
                &name,
                "текст извлечён из другой версии файла — проанализируйте и выберите заново",
            ))
        }
        Some(_) => {}
    }
    let (
        Some(extraction_id),
        Some(version),
        Some(raw_status),
        Some(text_chars),
        Some(truncated),
        Some(warnings_json),
    ) = (
        row.extraction_id,
        row.extractor_version,
        row.status,
        row.text_chars,
        row.truncated,
        row.warnings_json,
    )
    else {
        return Err(blocked(
            &name,
            "результат извлечения удалён из cache — проанализируйте заново",
        ));
    };
    let status = ExtractionStatus::try_from_db(&raw_status).ok_or_else(|| {
        BackendError::database(format!("invalid enum in extractions.status: {raw_status}"))
    })?;
    if !status.usable_for_corpus() {
        return Err(blocked(&name, "из файла не извлечён текст"));
    }
    let extractor_version = u32::try_from(version)
        .map_err(|_| BackendError::database("extractions.extractor_version is invalid"))?;
    if extractor_version != EXTRACTOR_VERSION {
        return Err(blocked(
            &name,
            "текст извлечён устаревшими правилами — проанализируйте заново",
        ));
    }
    let warning_values: Vec<serde_json::Value> =
        serde_json::from_str(&warnings_json).map_err(|error| {
            BackendError::database(format!(
                "invalid JSON in extractions.warnings_json: {error}"
            ))
        })?;
    let mut codes = Vec::with_capacity(warning_values.len());
    for value in &warning_values {
        let code = value
            .get("code")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| BackendError::database("extraction warning has no code"))?;
        codes.push(code.to_owned());
    }
    if status == ExtractionStatus::Partial {
        warnings.push(BundleWarningDto {
            code: "partial_extraction".into(),
            message: format!("«{name}»: извлечена только часть содержимого"),
            material_id: Some(row.material.id.clone()),
        });
    }
    if truncated != 0 {
        warnings.push(BundleWarningDto {
            code: "text_truncated".into(),
            message: format!("«{name}»: текст обрезан по лимиту извлечения"),
            material_id: Some(row.material.id.clone()),
        });
    }
    let file_kind = FileKind::try_from_db(&raw_kind).map_err(|message| {
        BackendError::database(format!("invalid enum in material kind: {message}"))
    })?;
    let kind = source_kind(row.material.kind, row.category.as_deref())?;
    Ok(PlannedItem {
        required: kind == SourceKind::Task,
        kind,
        material: row.material,
        name,
        file_kind,
        size_bytes,
        modified_at,
        relative_path,
        sha256: row.pinned,
        extraction_id,
        extractor_version,
        extraction_status: status,
        text_chars: u64::try_from(text_chars)
            .map_err(|_| BackendError::database("extractions.text_chars is negative"))?,
        warnings: codes,
    })
}

/// Категория материала → вид источника. Неизвестная категория — ошибка, не default.
fn source_kind(kind: MaterialKind, category: Option<&str>) -> Result<SourceKind, AppError> {
    let mapped = match (kind, category) {
        (_, Some("lecture")) => SourceKind::Lecture,
        (_, Some("seminar")) => SourceKind::Seminar,
        (_, Some("homework_task")) => SourceKind::Task,
        (_, Some("homework_data")) => SourceKind::Data,
        (_, Some("material" | "homework_draft" | "submission_attachment")) => SourceKind::Extra,
        (MaterialKind::File, Some("homework_final")) => SourceKind::PreviousWork,
        (MaterialKind::Inbox, Some("unknown") | None) => SourceKind::Extra,
        (_, Some(other)) => {
            return Err(BackendError::database(format!(
                "invalid enum in material category: {other}"
            )))
        }
        (MaterialKind::File, None) => {
            return Err(BackendError::database("file material has no category"))
        }
    };
    Ok(mapped)
}

/// Сверка содержимого всех элементов плана. Любое расхождение блокирует снимок.
pub fn verify_plan(plan: &BundlePlan, verifier: &dyn ContentVerifier) -> Result<(), AppError> {
    for item in &plan.items {
        match verifier.check(item) {
            ContentCheck::Same => {}
            ContentCheck::Changed => {
                return Err(blocked(
                    &item.name,
                    "файл на диске изменился после анализа — проанализируйте его заново",
                ))
            }
            ContentCheck::Missing => return Err(blocked(&item.name, "файл не найден в хранилище")),
            ContentCheck::Unavailable(reason) => {
                return Err(blocked(
                    &item.name,
                    &format!("не удалось проверить содержимое: {reason}"),
                ))
            }
        }
    }
    Ok(())
}

/// Шаги 1–2: план из БД и сверка содержимого с диском (соединение не удерживается).
pub fn prepare_snapshot(
    database: &Database,
    homework_id: &str,
    purpose: RunKind,
    verifier: &dyn ContentVerifier,
) -> Result<BundlePlan, AppError> {
    let plan =
        database.with_connection(|connection| plan_bundle(connection, homework_id, purpose))?;
    verify_plan(&plan, verifier)?;
    Ok(plan)
}

/// Шаг 3 (внутри транзакции): план перечитывается и обязан совпасть с проверенным,
/// затем снимок записывается. Возвращает id нового SourceBundle.
pub fn write_snapshot(
    connection: &Connection,
    homework_id: &str,
    purpose: RunKind,
    version_id: Option<&str>,
    verified: &BundlePlan,
) -> Result<String, AppError> {
    let current = plan_bundle(connection, homework_id, purpose)?;
    if &current != verified {
        return Err(BackendError::conflict(
            "Подборка материалов изменилась во время проверки — создайте набор заново",
        ));
    }
    if purpose == RunKind::ReviewVersion && version_id.is_none() {
        return Err(BackendError::blocked(
            "Для проверки версии нужен набор, привязанный к версии",
        ));
    }
    let bundle_id = uuid::Uuid::new_v4().to_string();
    let items: Vec<super::repository::NewBundleItem<'_>> = current
        .items
        .iter()
        .enumerate()
        .map(|(position, item)| super::repository::NewBundleItem {
            id: uuid::Uuid::new_v4().to_string(),
            material: &item.material,
            kind: item.kind,
            required: item.required,
            position: position as i64,
            name: &item.name,
            file_kind: item.file_kind,
            size_bytes: item.size_bytes,
            modified_at: &item.modified_at,
            relative_path: &item.relative_path,
            sha256: &item.sha256,
            extraction_id: &item.extraction_id,
            extractor_version: item.extractor_version,
            extraction_status: item.extraction_status.as_db_str(),
            text_chars: item.text_chars,
            warnings: &item.warnings,
        })
        .collect();
    super::repository::insert_bundle(
        connection,
        &super::repository::NewBundle {
            id: &bundle_id,
            homework_id,
            purpose,
            version_id,
            complete: current.complete,
            missing: &current.missing,
            selection_id: &current.selection_id,
            extractor_version: EXTRACTOR_VERSION,
            warnings: &current.warnings,
            total_chars: current.total_chars,
            limit_chars: current.limit_chars,
        },
        &items,
    )?;
    Ok(bundle_id)
}
