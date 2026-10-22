//! DTO этапа 7 для frontend (camelCase). Абсолютных путей нет: материал
//! адресуется непрозрачной ссылкой `{ kind, id }`.

use serde::{Deserialize, Serialize};

use crate::{dto::FileKind, error::AppError};

use super::{
    model::{
        Block, ExtractionLimits, ExtractionMetadata, ExtractionStatus, ExtractionWarning,
        SourceFormat,
    },
    ocr::OcrStatusDto,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    /// Файл «Входящих».
    Inbox,
    /// Файл недели или ДЗ (таблица `files`).
    File,
}

impl MaterialKind {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::File => "file",
        }
    }

    pub fn try_from_db(value: &str) -> Option<Self> {
        match value {
            "inbox" => Some(Self::Inbox),
            "file" => Some(Self::File),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MaterialRef {
    pub kind: MaterialKind,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionSummaryDto {
    pub sha256: String,
    pub format: Option<SourceFormat>,
    pub status: Option<ExtractionStatus>,
    pub status_message: Option<String>,
    pub text_chars: u64,
    pub truncated: bool,
    pub warning_count: u64,
    pub analyzed_at: String,
    /// Файл изменился после анализа — результат относится к старому содержимому.
    pub stale: bool,
    pub ocr_used: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialExtractionItemDto {
    pub material: MaterialRef,
    pub name: String,
    pub file_kind: FileKind,
    pub size_bytes: u64,
    /// «Входящие» или «ОБА · Неделя 7 · Лекции». Только для показа.
    pub context: String,
    pub extraction: Option<ExtractionSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialExtractionDto {
    pub material: MaterialRef,
    pub name: String,
    pub context: String,
    pub sha256: String,
    pub extractor_version: u32,
    pub format: SourceFormat,
    pub status: ExtractionStatus,
    pub status_message: Option<String>,
    pub metadata: ExtractionMetadata,
    pub blocks: Vec<Block>,
    pub blocks_truncated: bool,
    pub text: String,
    pub text_chars: u64,
    pub truncated: bool,
    pub warnings: Vec<ExtractionWarning>,
    pub ocr_used: bool,
    pub source_size_bytes: u64,
    pub duration_ms: u64,
    pub extracted_at: String,
    pub stale: bool,
    /// Результат взят из cache по SHA-256 без повторного разбора.
    pub cached: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchItemOutcome {
    Extracted,
    Cached,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchItemReportDto {
    pub material: MaterialRef,
    pub name: String,
    pub outcome: BatchItemOutcome,
    pub status: Option<ExtractionStatus>,
    pub text_chars: u64,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionBatchReport {
    pub operation_id: String,
    pub items: Vec<BatchItemReportDto>,
    pub extracted: usize,
    pub cached: usize,
    pub failed: usize,
    pub cancelled: usize,
    /// Пользователь остановил пакет; уже обработанные файлы сохранены в cache.
    pub stopped_by_user: bool,
    #[serde(default)]
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionLimitsDto {
    pub max_input_bytes: u64,
    pub max_text_chars: u64,
    pub time_limit_seconds: u64,
    pub max_pdf_pages: u32,
    pub max_sheets: u64,
    pub max_cells_per_sheet: u64,
    pub max_slides: u64,
    pub max_corpus_chars: u64,
}

impl ExtractionLimitsDto {
    pub fn from_limits(limits: &ExtractionLimits, max_corpus_chars: u64) -> Self {
        Self {
            max_input_bytes: limits.max_input_bytes,
            max_text_chars: limits.max_text_chars as u64,
            time_limit_seconds: limits.time_limit.as_secs(),
            max_pdf_pages: limits.max_pdf_pages,
            max_sheets: limits.max_sheets as u64,
            max_cells_per_sheet: limits.max_cells_per_sheet,
            max_slides: limits.max_slides as u64,
            max_corpus_chars,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionSettingsDto {
    pub ocr_enabled: bool,
    pub ocr: OcrStatusDto,
    pub limits: ExtractionLimitsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusItemDto {
    pub material: MaterialRef,
    pub name: String,
    pub context: String,
    pub sha256: String,
    pub status: Option<ExtractionStatus>,
    pub text_chars: u64,
    pub warning_count: u64,
    /// Готов к передаче: материал на месте, содержимое не менялось, текст извлечён.
    pub ready: bool,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusSelectionDto {
    pub homework_id: Option<String>,
    pub updated_at: Option<String>,
    pub items: Vec<CorpusItemDto>,
    pub total_chars: u64,
    pub limit_chars: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusDocumentDto {
    pub material: MaterialRef,
    pub name: String,
    pub sha256: String,
    pub format: SourceFormat,
    pub status: ExtractionStatus,
    pub text: String,
    pub text_chars: u64,
    /// Текст документа обрезан лимитом корпуса.
    pub truncated: bool,
    pub warnings: Vec<ExtractionWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusExcludedDto {
    pub material: MaterialRef,
    pub name: String,
    pub reason: String,
}

/// Ровно то, что будущий анализ получит из подборки. Сам анализ не запускается.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusPreviewDto {
    pub homework_id: Option<String>,
    pub documents: Vec<CorpusDocumentDto>,
    pub excluded: Vec<CorpusExcludedDto>,
    pub total_chars: u64,
    pub limit_chars: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheClearedDto {
    pub removed: u64,
}
