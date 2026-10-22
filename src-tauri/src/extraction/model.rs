//! Нормализованная модель извлечённого содержимого.
//!
//! Структура и текст строятся одинаково для всех форматов, чтобы будущий анализ
//! получал один корпус. Сериализация — точный camelCase-контракт frontend.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// Версия правил извлечения. Повышение версии делает старый cache недействительным:
/// результаты ищутся по паре (SHA-256 источника, версия).
pub const EXTRACTOR_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Text,
    Markdown,
    Csv,
    Tsv,
    Notebook,
    Image,
    Unknown,
}

impl SourceFormat {
    pub const ALL: [Self; 11] = [
        Self::Pdf,
        Self::Docx,
        Self::Xlsx,
        Self::Pptx,
        Self::Text,
        Self::Markdown,
        Self::Csv,
        Self::Tsv,
        Self::Notebook,
        Self::Image,
        Self::Unknown,
    ];

    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Text => "text",
            Self::Markdown => "markdown",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Notebook => "notebook",
            Self::Image => "image",
            Self::Unknown => "unknown",
        }
    }

    pub fn try_from_db(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.as_db_str() == value)
    }
}

/// Итог извлечения. `Timeout` и `Failed` не считаются переиспользуемым cache:
/// следующий анализ того же содержимого выполняется заново.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStatus {
    /// Всё поддерживаемое содержимое извлечено.
    Complete,
    /// Текст есть, но часть содержимого пропущена или обрезана (см. предупреждения).
    Partial,
    /// Текстового слоя нет (скан, изображение без OCR); метаданные сохранены.
    NeedsOcr,
    Unsupported,
    Encrypted,
    Corrupt,
    /// Отклонён по соображениям безопасности (zip-бомба, DTD/сущности, обход пути).
    Rejected,
    TooLarge,
    Timeout,
    Failed,
}

impl ExtractionStatus {
    pub const ALL: [Self; 10] = [
        Self::Complete,
        Self::Partial,
        Self::NeedsOcr,
        Self::Unsupported,
        Self::Encrypted,
        Self::Corrupt,
        Self::Rejected,
        Self::TooLarge,
        Self::Timeout,
        Self::Failed,
    ];

    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::NeedsOcr => "needs_ocr",
            Self::Unsupported => "unsupported",
            Self::Encrypted => "encrypted",
            Self::Corrupt => "corrupt",
            Self::Rejected => "rejected",
            Self::TooLarge => "too_large",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }

    pub fn try_from_db(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.as_db_str() == value)
    }

    /// Можно ли вернуть сохранённый результат без повторного разбора.
    pub fn reusable(self) -> bool {
        !matches!(self, Self::Timeout | Self::Failed)
    }

    /// Пригоден ли результат для подборки материалов (есть нормализованный текст).
    pub fn usable_for_corpus(self) -> bool {
        matches!(self, Self::Complete | Self::Partial)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    TextTruncated,
    BlocksTruncated,
    PagesTruncated,
    PageWithoutText,
    PageUnreadable,
    SheetsTruncated,
    CellsTruncated,
    RowsTruncated,
    SlidesTruncated,
    MacrosIgnored,
    EmbeddedObjectsSkipped,
    ImagesSkipped,
    ChartsSkipped,
    ExternalLinksIgnored,
    FormulasNotEvaluated,
    PartsSkipped,
    PartUnreadable,
    EncodingFallback,
    ExtensionMismatch,
    HiddenContent,
    UnknownEntity,
    TrackedChangesSkipped,
    OutputsSkipped,
    OcrDisabled,
    OcrUnavailable,
    OcrFailed,
    EmptyDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionWarning {
    pub code: WarningCode,
    pub message: String,
    /// Где пропущено: «стр. 3», «лист «Данные»», «слайд 4». Без путей файловой системы.
    pub location: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    /// Программа, создавшая документ (по данным файла).
    pub application: Option<String>,
    pub producer: Option<String>,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub page_count: Option<u32>,
    pub sheet_count: Option<u32>,
    pub slide_count: Option<u32>,
    pub word_count: Option<u64>,
    pub pdf_version: Option<String>,
    pub image_format: Option<String>,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    pub encoding: Option<String>,
    pub delimiter: Option<String>,
    pub row_count: Option<u64>,
    pub column_count: Option<u32>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetCell {
    #[serde(rename = "ref")]
    pub reference: String,
    pub value: String,
    /// Текст формулы без вычисления. `null` — обычное значение.
    pub formula: Option<String>,
}

/// Блок структуры документа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Block {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph {
        text: String,
    },
    ListItem {
        level: u8,
        ordered: bool,
        text: String,
    },
    Table {
        rows: Vec<Vec<String>>,
        truncated: bool,
    },
    Page {
        number: u32,
        text: String,
    },
    Slide {
        number: u32,
        title: Option<String>,
        body: Vec<String>,
        notes: Option<String>,
        hidden: bool,
    },
    Sheet {
        name: String,
        hidden: bool,
        declared_range: Option<String>,
        used_range: Option<String>,
        row_count: u32,
        column_count: u32,
        cell_count: u64,
        formula_count: u64,
        /// Первые ячейки для просмотра; полный текст листа — в нормализованном тексте.
        cells: Vec<SheetCell>,
        truncated: bool,
    },
    Code {
        language: Option<String>,
        text: String,
    },
}

/// Результат разбора одного источника (до сохранения в SQLite).
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    pub format: SourceFormat,
    pub status: ExtractionStatus,
    pub status_message: Option<String>,
    pub metadata: ExtractionMetadata,
    pub blocks: Vec<Block>,
    pub blocks_truncated: bool,
    pub text: String,
    pub text_chars: usize,
    pub truncated: bool,
    pub warnings: Vec<ExtractionWarning>,
    pub ocr_used: bool,
}

impl Extracted {
    /// Результат без содержимого: неподдерживаемый, повреждённый, зашифрованный файл и т. п.
    pub fn empty(
        format: SourceFormat,
        status: ExtractionStatus,
        message: impl Into<String>,
    ) -> Self {
        Self {
            format,
            status,
            status_message: Some(message.into()),
            metadata: ExtractionMetadata::default(),
            blocks: Vec::new(),
            blocks_truncated: false,
            text: String::new(),
            text_chars: 0,
            truncated: false,
            warnings: Vec::new(),
            ocr_used: false,
        }
    }
}

/// Лимиты извлечения. Значения по умолчанию рассчитаны на учебные материалы
/// и не дают одному файлу занять память или время приложения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionLimits {
    /// Максимальный размер исходного файла, который читается в память.
    pub max_input_bytes: u64,
    pub max_text_input_bytes: u64,
    pub max_image_bytes: u64,
    /// Время на один файл (кооперативные проверки + сторожевой таймер потока).
    pub time_limit: Duration,
    /// Сколько символов нормализованного текста сохраняется на файл.
    pub max_text_chars: usize,
    pub max_blocks: usize,
    /// Суммарный объём текста в блоках структуры.
    pub max_structure_chars: usize,
    pub max_pdf_pages: u32,
    /// Предел распаковки одного потока PDF (защита от decompression bomb).
    pub max_pdf_stream_bytes: usize,
    pub max_office_entries: usize,
    pub max_office_part_bytes: u64,
    pub max_office_total_bytes: u64,
    pub max_sheets: usize,
    pub max_cells_per_sheet: u64,
    pub max_total_cells: u64,
    pub max_structure_cells_per_sheet: usize,
    pub max_slides: usize,
    pub max_table_rows: usize,
    pub max_table_columns: usize,
    pub max_shared_strings: usize,
    pub max_xml_events: u64,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 200 * 1024 * 1024,
            max_text_input_bytes: 32 * 1024 * 1024,
            max_image_bytes: 50 * 1024 * 1024,
            time_limit: Duration::from_secs(60),
            max_text_chars: 2_000_000,
            max_blocks: 20_000,
            max_structure_chars: 4_000_000,
            max_pdf_pages: 2_000,
            max_pdf_stream_bytes: 64 * 1024 * 1024,
            max_office_entries: 10_000,
            max_office_part_bytes: 128 * 1024 * 1024,
            max_office_total_bytes: 512 * 1024 * 1024,
            max_sheets: 100,
            max_cells_per_sheet: 200_000,
            max_total_cells: 500_000,
            max_structure_cells_per_sheet: 2_000,
            max_slides: 1_000,
            max_table_rows: 1_000,
            max_table_columns: 64,
            max_shared_strings: 1_000_000,
            max_xml_events: 50_000_000,
        }
    }
}

/// Почему разбор остановлен.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    Timeout,
    Cancelled,
}

/// Кооперативный бюджет времени и флаг отмены. Разборщики проверяют его
/// между страницами, листами, слайдами и каждые несколько тысяч XML-событий.
#[derive(Debug, Clone)]
pub struct Budget {
    deadline: Instant,
    cancel: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
}

impl Budget {
    pub fn new(limit: Duration, cancel: Arc<AtomicBool>, abort: Arc<AtomicBool>) -> Self {
        Self {
            deadline: Instant::now() + limit,
            cancel,
            abort,
        }
    }

    pub fn unlimited() -> Self {
        Self::new(
            Duration::from_secs(3600),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    pub fn check(&self) -> Result<(), Interrupt> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Interrupt::Cancelled);
        }
        if self.abort.load(Ordering::Relaxed) || Instant::now() >= self.deadline {
            return Err(Interrupt::Timeout);
        }
        Ok(())
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

/// Ошибка разбора: превращается в статус результата.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractError {
    Interrupted(Interrupt),
    Corrupt(String),
    Encrypted(String),
    Unsupported(String),
    Rejected(String),
    TooLarge(String),
}

impl From<Interrupt> for ExtractError {
    fn from(value: Interrupt) -> Self {
        Self::Interrupted(value)
    }
}

pub type ExtractResult<T> = Result<T, ExtractError>;

/// Собирает блоки, нормализованный текст и предупреждения с единым соблюдением лимитов.
pub struct Collector {
    max_text_chars: usize,
    max_blocks: usize,
    max_structure_chars: usize,
    blocks: Vec<Block>,
    structure_chars: usize,
    blocks_truncated: bool,
    text: String,
    text_chars: usize,
    truncated: bool,
    warnings: Vec<ExtractionWarning>,
    warned: HashMap<WarningCode, usize>,
    in_list: bool,
}

impl Collector {
    pub fn new(limits: &ExtractionLimits) -> Self {
        Self {
            max_text_chars: limits.max_text_chars,
            max_blocks: limits.max_blocks,
            max_structure_chars: limits.max_structure_chars,
            blocks: Vec::new(),
            structure_chars: 0,
            blocks_truncated: false,
            text: String::new(),
            text_chars: 0,
            truncated: false,
            warnings: Vec::new(),
            warned: HashMap::new(),
            in_list: false,
        }
    }

    /// Текст достиг лимита: дальнейший разбор только тратит время.
    pub fn is_full(&self) -> bool {
        self.truncated
    }

    pub fn text_chars(&self) -> usize {
        self.text_chars
    }

    /// Добавляет блок в структуру и его представление в текст.
    pub fn push(&mut self, block: Block) {
        let rendered = render_block(&block);
        let is_list = matches!(block, Block::ListItem { .. });
        if self.in_list && !is_list {
            self.push_text("\n");
        }
        self.in_list = is_list;
        self.push_text(&rendered);
        self.push_structure(block);
    }

    /// Только структура (например, лист с первыми ячейками; текст листа пишется отдельно).
    pub fn push_structure(&mut self, block: Block) {
        if self.blocks_truncated {
            return;
        }
        let size = block_chars(&block);
        if self.blocks.len() >= self.max_blocks
            || self.structure_chars.saturating_add(size) > self.max_structure_chars
        {
            self.blocks_truncated = true;
            self.warn(
                WarningCode::BlocksTruncated,
                "Структура документа слишком большая: показана первая часть, полный текст — во вкладке «Текст»",
                None,
            );
            return;
        }
        self.structure_chars += size;
        self.blocks.push(block);
    }

    /// Только нормализованный текст, с ограничением числа символов.
    pub fn push_text(&mut self, text: &str) {
        if self.truncated || text.is_empty() {
            return;
        }
        let remaining = self.max_text_chars.saturating_sub(self.text_chars);
        let count = text.chars().count();
        if count <= remaining {
            self.text.push_str(text);
            self.text_chars += count;
            return;
        }
        let cut: String = text.chars().take(remaining).collect();
        self.text.push_str(&cut);
        self.text_chars += remaining;
        self.truncated = true;
        let limit = self.max_text_chars;
        self.warn(
            WarningCode::TextTruncated,
            format!("Текст обрезан до {limit} символов"),
            None,
        );
    }

    /// Предупреждение с одним и тем же кодом и местом добавляется один раз;
    /// повторы считаются и не раздувают результат.
    pub fn warn(
        &mut self,
        code: WarningCode,
        message: impl Into<String>,
        location: Option<String>,
    ) {
        let counter = self.warned.entry(code).or_insert(0);
        *counter += 1;
        if *counter > 20 {
            return;
        }
        if self
            .warnings
            .iter()
            .any(|warning| warning.code == code && warning.location == location)
        {
            return;
        }
        self.warnings.push(ExtractionWarning {
            code,
            message: message.into(),
            location,
        });
    }

    pub fn has_warning(&self, code: WarningCode) -> bool {
        self.warned.contains_key(&code)
    }

    /// Завершает сбор. Статус `Partial`, если что-то пропущено или обрезано.
    pub fn finish(self, format: SourceFormat, metadata: ExtractionMetadata) -> Extracted {
        let lossy = self.truncated
            || self.blocks_truncated
            || self.warnings.iter().any(|warning| is_lossy(warning.code));
        let text = normalize_blank_lines(&self.text);
        let text_chars = text.chars().count();
        Extracted {
            format,
            status: if lossy {
                ExtractionStatus::Partial
            } else {
                ExtractionStatus::Complete
            },
            status_message: None,
            metadata,
            blocks: self.blocks,
            blocks_truncated: self.blocks_truncated,
            text,
            text_chars,
            truncated: self.truncated,
            warnings: self.warnings,
            ocr_used: false,
        }
    }
}

/// Предупреждения, означающие потерю содержимого (а не просто примечание).
fn is_lossy(code: WarningCode) -> bool {
    !matches!(
        code,
        WarningCode::FormulasNotEvaluated
            | WarningCode::EncodingFallback
            | WarningCode::ExtensionMismatch
            | WarningCode::MacrosIgnored
            | WarningCode::ExternalLinksIgnored
            | WarningCode::EmptyDocument
            | WarningCode::OcrDisabled
            | WarningCode::TrackedChangesSkipped
            | WarningCode::OutputsSkipped
    )
}

fn block_chars(block: &Block) -> usize {
    match block {
        Block::Heading { text, .. }
        | Block::Paragraph { text }
        | Block::ListItem { text, .. }
        | Block::Page { text, .. }
        | Block::Code { text, .. } => text.len(),
        Block::Table { rows, .. } => rows.iter().flatten().map(String::len).sum(),
        Block::Slide {
            title, body, notes, ..
        } => {
            title.as_ref().map_or(0, String::len)
                + body.iter().map(String::len).sum::<usize>()
                + notes.as_ref().map_or(0, String::len)
        }
        Block::Sheet { cells, name, .. } => {
            name.len()
                + cells
                    .iter()
                    .map(|cell| cell.value.len() + cell.formula.as_ref().map_or(0, String::len) + 8)
                    .sum::<usize>()
        }
    }
}

/// Текстовое представление блока (markdown-подобное, без HTML).
pub fn render_block(block: &Block) -> String {
    match block {
        Block::Heading { level, text } => {
            format!(
                "{} {}\n\n",
                "#".repeat(usize::from((*level).clamp(1, 6))),
                text
            )
        }
        Block::Paragraph { text } => format!("{text}\n\n"),
        Block::ListItem {
            level,
            ordered,
            text,
        } => format!(
            "{}{} {}\n",
            "  ".repeat(usize::from((*level).min(8))),
            if *ordered { "1." } else { "-" },
            text.replace('\n', " ")
        ),
        Block::Table { rows, truncated } => {
            let mut out = String::new();
            for row in rows {
                out.push_str("| ");
                out.push_str(
                    &row.iter()
                        .map(|cell| cell.replace(['\n', '|'], " "))
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                out.push_str(" |\n");
            }
            if *truncated {
                out.push_str("| … |\n");
            }
            out.push('\n');
            out
        }
        Block::Page { number, text } => format!("--- Страница {number} ---\n{text}\n\n"),
        Block::Slide {
            number,
            title,
            body,
            notes,
            hidden,
        } => {
            let mut out = format!(
                "--- Слайд {number}{} ---\n",
                if *hidden { " (скрытый)" } else { "" }
            );
            if let Some(title) = title {
                out.push_str(&format!("# {title}\n"));
            }
            for line in body {
                out.push_str(line);
                out.push('\n');
            }
            if let Some(notes) = notes {
                out.push_str(&format!("Заметки докладчика: {notes}\n"));
            }
            out.push('\n');
            out
        }
        Block::Sheet { .. } => String::new(),
        Block::Code { language, text } => format!(
            "```{}\n{}\n```\n\n",
            language.as_deref().unwrap_or(""),
            text.trim_end()
        ),
    }
}

/// Не больше одной пустой строки подряд, без хвостовых пробелов.
pub fn normalize_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0_usize;
    for line in text.split('\n') {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_owned()
}

/// Удаляет управляющие символы (кроме табуляции и перевода строки), BOM и
/// символы bidi-override; приводит переводы строк к `\n`.
pub fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            '\n' | '\t' => out.push(ch),
            '\u{FEFF}' | '\u{200B}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => {}
            '\u{00A0}' => out.push(' '),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    out
}

/// Компактная запись номеров: «2, 4–6, 9».
pub fn compact_ranges(numbers: &[u32]) -> String {
    let mut parts = Vec::new();
    let mut index = 0;
    while index < numbers.len() {
        let start = numbers[index];
        let mut end = start;
        while index + 1 < numbers.len() && numbers[index + 1] == end + 1 {
            index += 1;
            end = numbers[index];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}–{end}")
        });
        index += 1;
    }
    parts.join(", ")
}
