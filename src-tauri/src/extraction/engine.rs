//! Определение формата и изолированный запуск разбора.
//!
//! Разбор выполняется в отдельном потоке с увеличенным стеком, под
//! `catch_unwind` и сторожевым таймером: зависание или panic разборщика не
//! блокирует приложение, а превращается в статус `timeout`/`corrupt`.
//! Содержимое уже прочитано в память — поток не обращается к файловой системе.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    docx,
    image::{self, OcrMode},
    model::{
        Budget, ExtractError, Extracted, ExtractionLimits, ExtractionStatus, ExtractionWarning,
        Interrupt, SourceFormat, WarningCode,
    },
    ocr::OcrEngine,
    ooxml, pdf, pptx, text, xlsx,
};

const WORKER_STACK_BYTES: usize = 32 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(50);
/// Сколько ждать поток после истечения бюджета, прежде чем вернуть `timeout`.
const GRACE: Duration = Duration::from_secs(2);

/// Подсказка формата по расширению имени.
pub fn hint_from_name(name: &str) -> Option<SourceFormat> {
    let extension = name.rsplit_once('.')?.1.to_lowercase();
    Some(match extension.as_str() {
        "pdf" => SourceFormat::Pdf,
        "docx" | "docm" | "dotx" | "dotm" => SourceFormat::Docx,
        "xlsx" | "xlsm" | "xltx" | "xltm" => SourceFormat::Xlsx,
        "pptx" | "pptm" | "potx" | "ppsx" => SourceFormat::Pptx,
        "md" | "markdown" => SourceFormat::Markdown,
        "csv" => SourceFormat::Csv,
        "tsv" | "tab" => SourceFormat::Tsv,
        "ipynb" => SourceFormat::Notebook,
        "txt" | "text" | "log" | "py" | "r" | "sql" | "tex" | "json" | "yaml" | "yml" | "toml"
        | "ini" | "cfg" | "rst" => SourceFormat::Text,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "tif" | "tiff" | "bmp" | "heic" | "heif" => {
            SourceFormat::Image
        }
        _ => return None,
    })
}

/// Состояние OCR, влияющее на результат для изображений.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrKey {
    Disabled,
    Enabled,
    /// Включён пользователем, но локальный движок не найден.
    Unavailable,
}

/// Ключ cache: результат зависит от содержимого, версии правил и того, как
/// интерпретировать текстовый формат/в каком состоянии OCR для изображений.
pub fn options_key(name: &str, ocr: OcrKey) -> String {
    match hint_from_name(name) {
        Some(SourceFormat::Image) => match ocr {
            OcrKey::Disabled => "image".into(),
            OcrKey::Enabled => "image+ocr".into(),
            OcrKey::Unavailable => "image+no-ocr".into(),
        },
        Some(format) => format.as_db_str().into(),
        None => "auto".into(),
    }
}

enum Detected {
    Format(SourceFormat),
    Unsupported(String),
    Encrypted(String),
}

const OLE_SIGNATURE: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";

/// Формат определяется по содержимому; расширение лишь уточняет текстовые форматы.
fn detect(
    name: &str,
    bytes: &[u8],
    limits: &ExtractionLimits,
) -> (Detected, Option<ExtractionWarning>) {
    let hint = hint_from_name(name);
    let mismatch = |actual: SourceFormat| {
        hint.filter(|hinted| *hinted != actual && !is_text_family(*hinted, actual))
            .map(|_| ExtractionWarning {
                code: WarningCode::ExtensionMismatch,
                message: format!(
                    "Расширение файла не совпадает с содержимым: разобран как {}",
                    actual.as_db_str()
                ),
                location: None,
            })
    };
    let head = &bytes[..bytes.len().min(1024)];
    if head.windows(5).any(|window| window == b"%PDF-") {
        return (
            Detected::Format(SourceFormat::Pdf),
            mismatch(SourceFormat::Pdf),
        );
    }
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return detect_package(bytes, limits, hint, mismatch);
    }
    if bytes.starts_with(OLE_SIGNATURE) {
        let encrypted = contains_utf16(bytes, "EncryptedPackage");
        return (
            if encrypted {
                Detected::Encrypted("Документ Office защищён паролем — текст не извлекается".into())
            } else {
                Detected::Unsupported(
                    "Старый формат Office (.doc, .xls, .ppt) не поддерживается — сохраните файл как .docx, .xlsx или .pptx".into(),
                )
            },
            None,
        );
    }
    if image::image_format(bytes).is_some() {
        return (
            Detected::Format(SourceFormat::Image),
            mismatch(SourceFormat::Image),
        );
    }
    match hint {
        Some(
            format @ (SourceFormat::Text
            | SourceFormat::Markdown
            | SourceFormat::Csv
            | SourceFormat::Tsv
            | SourceFormat::Notebook),
        ) => (Detected::Format(format), None),
        Some(SourceFormat::Pdf) => (
            Detected::Unsupported("Файл с расширением .pdf не является PDF".into()),
            None,
        ),
        Some(SourceFormat::Docx | SourceFormat::Xlsx | SourceFormat::Pptx) => (
            Detected::Unsupported("Файл не является документом Office Open XML".into()),
            None,
        ),
        Some(SourceFormat::Image) => (
            Detected::Unsupported("Формат изображения не распознан".into()),
            None,
        ),
        _ => (
            Detected::Unsupported(format!(
                "Формат «{}» не поддерживается: текст извлекается из PDF, DOCX, XLSX, PPTX, TXT, MD, CSV, TSV, IPYNB и изображений",
                name.rsplit_once('.').map(|(_, extension)| extension).unwrap_or("без расширения")
            )),
            None,
        ),
    }
}

fn is_text_family(hinted: SourceFormat, actual: SourceFormat) -> bool {
    matches!(
        (hinted, actual),
        (SourceFormat::Text, SourceFormat::Markdown) | (SourceFormat::Markdown, SourceFormat::Text)
    )
}

fn detect_package(
    bytes: &[u8],
    limits: &ExtractionLimits,
    hint: Option<SourceFormat>,
    mismatch: impl Fn(SourceFormat) -> Option<ExtractionWarning>,
) -> (Detected, Option<ExtractionWarning>) {
    let index = match crate::lms::zip_safe::read_index(bytes, &ooxml::zip_limits(limits)) {
        Ok(index) => index,
        // Ошибку разбора покажет сам разборщик подходящего формата.
        Err(_) => {
            return match hint {
                Some(format @ (SourceFormat::Docx | SourceFormat::Xlsx | SourceFormat::Pptx)) => {
                    (Detected::Format(format), None)
                }
                _ => (
                    Detected::Unsupported(
                        "ZIP-архив: при извлечении текста архивы не распаковываются".into(),
                    ),
                    None,
                ),
            }
        }
    };
    let has = |path: &str| {
        index
            .files()
            .any(|entry| entry.path.eq_ignore_ascii_case(path))
    };
    let prefix = |path: &str| {
        index
            .files()
            .any(|entry| entry.path.to_lowercase().starts_with(path))
    };
    let format = if has("word/document.xml") || (prefix("word/") && has("[Content_Types].xml")) {
        Some(SourceFormat::Docx)
    } else if has("xl/workbook.xml") || (prefix("xl/") && has("[Content_Types].xml")) {
        Some(SourceFormat::Xlsx)
    } else if has("ppt/presentation.xml") || (prefix("ppt/") && has("[Content_Types].xml")) {
        Some(SourceFormat::Pptx)
    } else {
        None
    };
    match format {
        Some(format) => (Detected::Format(format), mismatch(format)),
        None if has("mimetype") && prefix("content.xml") => (
            Detected::Unsupported(
                "Формат OpenDocument (.odt, .ods, .odp) пока не поддерживается — сохраните как .docx, .xlsx или .pptx".into(),
            ),
            None,
        ),
        None => (
            Detected::Unsupported("ZIP-архив: при извлечении текста архивы не распаковываются".into()),
            None,
        ),
    }
}

fn contains_utf16(bytes: &[u8], needle: &str) -> bool {
    let pattern: Vec<u8> = needle
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let scan = &bytes[..bytes.len().min(8 * 1024 * 1024)];
    scan.windows(pattern.len())
        .any(|window| window == pattern.as_slice())
}

/// Разбор уже прочитанного содержимого (вызывается внутри изолированного потока).
pub fn extract_bytes(
    name: &str,
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
    ocr: OcrMode<'_>,
) -> Result<Extracted, Interrupt> {
    let (detected, warning) = detect(name, bytes, limits);
    let format = match detected {
        Detected::Format(format) => format,
        Detected::Unsupported(message) => {
            return Ok(Extracted::empty(
                hint_from_name(name).unwrap_or(SourceFormat::Unknown),
                ExtractionStatus::Unsupported,
                message,
            ))
        }
        Detected::Encrypted(message) => {
            return Ok(Extracted::empty(
                hint_from_name(name).unwrap_or(SourceFormat::Unknown),
                ExtractionStatus::Encrypted,
                message,
            ))
        }
    };
    let text_limit = limits.max_text_input_bytes;
    let result = match format {
        SourceFormat::Pdf => pdf::extract(bytes, limits, budget),
        SourceFormat::Docx => docx::extract(bytes, limits, budget),
        SourceFormat::Xlsx => xlsx::extract(bytes, limits, budget),
        SourceFormat::Pptx => pptx::extract(bytes, limits, budget),
        SourceFormat::Image if bytes.len() as u64 > limits.max_image_bytes => Err(
            ExtractError::TooLarge("Изображение больше допустимого размера".into()),
        ),
        SourceFormat::Image => image::extract(bytes, limits, budget, ocr),
        _ if bytes.len() as u64 > text_limit => Err(ExtractError::TooLarge(format!(
            "Текстовый файл больше {} МБ",
            text_limit / (1024 * 1024)
        ))),
        SourceFormat::Text => text::extract_plain(bytes, limits, budget),
        SourceFormat::Markdown => text::extract_markdown(bytes, limits, budget),
        SourceFormat::Csv | SourceFormat::Tsv => {
            text::extract_delimited(bytes, format, limits, budget)
        }
        SourceFormat::Notebook => text::extract_notebook(bytes, limits, budget),
        SourceFormat::Unknown => Err(ExtractError::Unsupported("Формат не определён".into())),
    };
    let mut extracted = match result {
        Ok(extracted) => extracted,
        Err(ExtractError::Interrupted(interrupt)) => return Err(interrupt),
        Err(error) => {
            let (status, message) = match error {
                ExtractError::Corrupt(message) => (ExtractionStatus::Corrupt, message),
                ExtractError::Encrypted(message) => (ExtractionStatus::Encrypted, message),
                ExtractError::Unsupported(message) => (ExtractionStatus::Unsupported, message),
                ExtractError::Rejected(message) => (ExtractionStatus::Rejected, message),
                ExtractError::TooLarge(message) => (ExtractionStatus::TooLarge, message),
                ExtractError::Interrupted(_) => unreachable!("handled above"),
            };
            Extracted::empty(format, status, message)
        }
    };
    if let Some(warning) = warning {
        extracted.warnings.insert(0, warning);
    }
    Ok(extracted)
}

/// Запускает разбор в изолированном потоке. `Err(Cancelled)` — пользователь
/// отменил; время истекло — результат со статусом `timeout`.
pub fn run_isolated(
    name: &str,
    bytes: Arc<Vec<u8>>,
    limits: ExtractionLimits,
    cancel: Arc<AtomicBool>,
    ocr: Option<Arc<dyn OcrEngine>>,
    ocr_enabled: bool,
) -> Result<Extracted, Interrupt> {
    let abort = Arc::new(AtomicBool::new(false));
    let budget = Budget::new(limits.time_limit, Arc::clone(&cancel), Arc::clone(&abort));
    let deadline = budget.deadline();
    let (sender, receiver) = mpsc::channel();
    let worker_name = name.to_owned();
    let spawned = thread::Builder::new()
        .name("kontur-extract".into())
        .stack_size(WORKER_STACK_BYTES)
        .spawn(move || {
            let mode = match (ocr_enabled, ocr.as_deref()) {
                (false, _) => OcrMode::Disabled,
                (true, None) => OcrMode::Unavailable,
                (true, Some(engine)) => OcrMode::Enabled(engine),
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                extract_bytes(&worker_name, &bytes, &limits, &budget, mode)
            }));
            let _ = sender.send(outcome);
        });
    if spawned.is_err() {
        return Ok(Extracted::empty(
            hint_from_name(name).unwrap_or(SourceFormat::Unknown),
            ExtractionStatus::Failed,
            "Не удалось запустить разбор",
        ));
    }
    loop {
        match receiver.recv_timeout(POLL) {
            Ok(Ok(Ok(extracted))) => return Ok(extracted),
            Ok(Ok(Err(Interrupt::Cancelled))) => return Err(Interrupt::Cancelled),
            Ok(Ok(Err(Interrupt::Timeout))) => return Ok(timeout_result(name, &limits)),
            Ok(Err(_panic)) => {
                return Ok(Extracted::empty(
                    hint_from_name(name).unwrap_or(SourceFormat::Unknown),
                    ExtractionStatus::Corrupt,
                    "Файл не удалось разобрать: структура документа некорректна",
                ))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::Relaxed) {
                    abort.store(true, Ordering::Relaxed);
                    return Err(Interrupt::Cancelled);
                }
                if Instant::now() >= deadline + GRACE {
                    // Поток остановится на ближайшей кооперативной проверке.
                    abort.store(true, Ordering::Relaxed);
                    return Ok(timeout_result(name, &limits));
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(Extracted::empty(
                    hint_from_name(name).unwrap_or(SourceFormat::Unknown),
                    ExtractionStatus::Failed,
                    "Разбор прервался",
                ))
            }
        }
    }
}

fn timeout_result(name: &str, limits: &ExtractionLimits) -> Extracted {
    Extracted::empty(
        hint_from_name(name).unwrap_or(SourceFormat::Unknown),
        ExtractionStatus::Timeout,
        format!(
            "Разбор не уложился в {} с и остановлен — попробуйте ещё раз или разделите файл",
            limits.time_limit.as_secs()
        ),
    )
}
