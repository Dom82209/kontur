//! Этап 6: безопасный импорт материалов LMS Центрального университета через
//! скачанные пользователем файлы и экспорт CU LMS Enhancer.
//!
//! - Никакой авторизации в LMS, cookies и сессий: читаются только файлы,
//!   которые пользователь сам скачал браузером.
//! - Отправка домашней работы в LMS не автоматизируется.
//! - Содержимое документов не анализируется и AI не запускается: классификация
//!   строится по именам папок и файлов внутри экспорта.
//!
//! Модуль не зависит от GUI и тестируется на `TempDir`.

pub mod candidates;
pub mod classify;
pub mod export;
pub mod links;
pub mod store;
pub mod watcher;
pub mod zip_safe;

#[cfg(test)]
mod tests;

pub use candidates::{
    Candidate, CandidateKind, CandidateOrigin, CandidateRegistry, CandidateSource,
    DownloadCandidateDto,
};
pub use export::{
    open_candidate, preview, ExportImport, ExportImportEntry, ExportImportRequest, ExportPhase,
    ExportPreviewDto, ExportProgress, SourceContext,
};
pub use links::{normalize_lms_url, require_lms_url, ALLOWED_LMS_HOSTS};
pub use watcher::{DownloadsWatcherDto, ScannerConfig, WatcherHost, WatcherService, WatcherState};
pub use zip_safe::ZipLimits;
