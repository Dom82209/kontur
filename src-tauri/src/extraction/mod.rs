//! Этап 7: локальное извлечение текста и структуры из учебных материалов.
//!
//! - Всё выполняется на этом компьютере; Codex/Claude и сеть не используются.
//! - Оригиналы неизменяемы: файл открывается только на чтение fd-relative,
//!   содержимое и SHA-256 берутся из одного inode.
//! - Результат привязан к SHA-256 источника и версии правил; повтор для того же
//!   содержимого берётся из cache, cache можно перестроить (`force`) или очистить.
//! - Лимиты размера, времени, числа страниц/листов/ячеек/слайдов и символов;
//!   разбор идёт в отдельном потоке со сторожевым таймером и `catch_unwind`.
//! - Office-файлы: строгий ZIP-парсер в памяти (zip-бомбы, обход пути),
//!   XML без DTD и пользовательских сущностей, relationships только внутрь
//!   пакета; макросы, формулы и встроенные объекты не выполняются.
//! - В будущий анализ передаётся только подборка, явно выбранная пользователем.

pub mod docx;
pub mod dto;
pub mod engine;
pub mod image;
pub mod jobs;
pub mod model;
pub mod ocr;
pub mod ooxml;
pub mod pdf;
pub mod pptx;
pub mod service;
pub mod store;
pub mod text;
pub mod xlsx;

#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
mod tests;

pub use dto::{
    BatchItemOutcome, CacheClearedDto, CorpusPreviewDto, CorpusSelectionDto, ExtractionBatchReport,
    ExtractionSettingsDto, MaterialExtractionDto, MaterialExtractionItemDto, MaterialKind,
    MaterialRef,
};
pub use jobs::ExtractionJobs;
pub use model::{ExtractionLimits, ExtractionStatus, SourceFormat, EXTRACTOR_VERSION};
pub use ocr::{OcrEngine, TesseractOcr};
pub use service::{BatchOutcome, BatchProgress, ExtractionService};
