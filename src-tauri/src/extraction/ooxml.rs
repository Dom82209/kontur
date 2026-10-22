//! Общий слой Office Open XML (DOCX, XLSX, PPTX).
//!
//! Безопасность:
//!
//! - ZIP разбирается строгим парсером этапа 6 (`lms::zip_safe`) прямо из буфера
//!   в памяти: лимиты числа записей, размера части, суммы и коэффициента сжатия
//!   (zip-бомбы), запрет обхода пути, symlink, шифрования и перекрытия данных.
//!   На диск ничего не распаковывается.
//! - XML читается потоково `quick-xml`; любой `<!DOCTYPE>` отклоняет документ,
//!   поэтому пользовательские и внешние сущности (XXE, «billion laughs») не
//!   существуют. Ссылки `&...;` раскрываются только для пяти стандартных
//!   сущностей и числовых ссылок на символ.
//! - Relationships разрешаются только внутрь пакета; внешние цели
//!   (`TargetMode="External"`) не открываются, `..` за корень пакета отклоняется.
//! - Макросы VBA, встроенные объекты и ActiveX никогда не читаются и не
//!   запускаются — только отмечаются в предупреждениях.

use std::collections::HashMap;

use quick_xml::{
    events::{BytesStart, Event},
    Reader, XmlVersion,
};

use crate::lms::zip_safe::{self, ZipEntry, ZipIndex, ZipLimits, ZipRejection};

use super::model::{
    clean_text, Budget, Collector, ExtractError, ExtractResult, ExtractionLimits,
    ExtractionMetadata, WarningCode,
};

/// Узел потокового XML.
pub enum Node<'a> {
    Start(BytesStart<'a>),
    Empty(BytesStart<'a>),
    End(String),
    Text(String),
    Eof,
}

/// Потоковый XML-читатель без DTD и пользовательских сущностей.
pub struct Xml<'a> {
    reader: Reader<&'a [u8]>,
    budget: &'a Budget,
    events: u64,
    max_events: u64,
    pub unknown_entities: usize,
}

impl<'a> Xml<'a> {
    pub fn new(data: &'a str, budget: &'a Budget, limits: &ExtractionLimits) -> Self {
        let mut reader = Reader::from_str(data);
        let config = reader.config_mut();
        config.check_end_names = false;
        config.allow_unmatched_ends = true;
        config.allow_dangling_amp = true;
        Self {
            reader,
            budget,
            events: 0,
            max_events: limits.max_xml_events,
            unknown_entities: 0,
        }
    }

    pub fn next_node(&mut self) -> ExtractResult<Node<'a>> {
        loop {
            self.events += 1;
            if self.events.is_multiple_of(4096) {
                self.budget.check()?;
            }
            if self.events > self.max_events {
                return Err(ExtractError::Rejected(
                    "XML-часть документа слишком велика".into(),
                ));
            }
            let event = self
                .reader
                .read_event()
                .map_err(|_| ExtractError::Corrupt("XML-часть документа повреждена".into()))?;
            return Ok(match event {
                Event::Start(start) => Node::Start(start),
                Event::Empty(start) => Node::Empty(start),
                Event::End(end) => Node::End(local(end.name().as_ref()).to_owned()),
                Event::Text(text) => Node::Text(text.xml_content(XmlVersion::Implicit1_0).into_owned()),
                Event::CData(data) => Node::Text(data.into_inner().into_owned()),
                Event::GeneralRef(reference) => {
                    let resolved = match reference.resolve_char_ref() {
                        Ok(Some(ch)) => Some(ch.to_string()),
                        Ok(None) => predefined_entity(&reference).map(str::to_owned),
                        Err(_) => None,
                    };
                    match resolved {
                        Some(text) => Node::Text(text),
                        None => {
                            self.unknown_entities += 1;
                            continue;
                        }
                    }
                }
                Event::DocType(_) => {
                    return Err(ExtractError::Rejected(
                        "Документ содержит DTD (<!DOCTYPE>) — такие XML не разбираются ради защиты от XXE и раздувания сущностей".into(),
                    ))
                }
                Event::Decl(_) | Event::PI(_) | Event::Comment(_) => continue,
                Event::Eof => Node::Eof,
            });
        }
    }

    /// Пропускает поддерево открытого элемента.
    pub fn skip_subtree(&mut self) -> ExtractResult<()> {
        let mut depth = 1_usize;
        loop {
            match self.next_node()? {
                Node::Start(_) => depth += 1,
                Node::End(_) => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                Node::Eof => return Ok(()),
                _ => {}
            }
        }
    }
}

fn predefined_entity(name: &str) -> Option<&'static str> {
    match name {
        "amp" => Some("&"),
        "lt" => Some("<"),
        "gt" => Some(">"),
        "quot" => Some("\""),
        "apos" => Some("'"),
        _ => None,
    }
}

/// Локальное имя без префикса пространства имён.
pub fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub fn name_of<'b>(start: &'b BytesStart<'_>) -> &'b str {
    local(start.name().0)
}

/// Значение атрибута по локальному имени (без префикса).
pub fn attr(start: &BytesStart<'_>, wanted: &str) -> Option<String> {
    start
        .attributes()
        .flatten()
        .find(|attribute| {
            local(attribute.key.as_ref()) == wanted && !is_prefixed(attribute.key.as_ref())
        })
        .or_else(|| {
            start
                .attributes()
                .flatten()
                .find(|attribute| local(attribute.key.as_ref()) == wanted)
        })
        .and_then(|attribute| {
            attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .ok()
                .map(|value| value.into_owned())
        })
}

/// Идентификатор relationship (`r:id`, `r:embed`): атрибут с префиксом.
pub fn rel_attr(start: &BytesStart<'_>, wanted: &str) -> Option<String> {
    start
        .attributes()
        .flatten()
        .find(|attribute| {
            is_prefixed(attribute.key.as_ref()) && local(attribute.key.as_ref()) == wanted
        })
        .and_then(|attribute| {
            attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .ok()
                .map(|value| value.into_owned())
        })
}

fn is_prefixed(key: &str) -> bool {
    key.contains(':')
}

/// Булев атрибут OOXML: отсутствие = `default`, «0»/«false»/«off» = false.
pub fn flag(start: &BytesStart<'_>, wanted: &str, default: bool) -> bool {
    match attr(start, wanted) {
        None => default,
        Some(value) => !matches!(value.as_str(), "0" | "false" | "off"),
    }
}

/// XML-часть как строка: UTF-8 или UTF-16 по BOM.
pub fn xml_string(data: Vec<u8>) -> ExtractResult<String> {
    if let Some(rest) = data.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8(rest.to_vec())
            .map_err(|_| ExtractError::Corrupt("XML-часть не в UTF-8".into()));
    }
    for (bom, little) in [(&b"\xFF\xFE"[..], true), (&b"\xFE\xFF"[..], false)] {
        if let Some(rest) = data.strip_prefix(bom) {
            let (pairs, remainder) = rest.as_chunks::<2>();
            if !remainder.is_empty() {
                return Err(ExtractError::Corrupt(
                    "XML-часть содержит неполную UTF-16 кодовую единицу".into(),
                ));
            }
            let units: Vec<u16> = pairs
                .iter()
                .map(|pair| {
                    if little {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .collect();
            return String::from_utf16(&units)
                .map_err(|_| ExtractError::Corrupt("XML-часть в неверной UTF-16".into()));
        }
    }
    String::from_utf8(data).map_err(|_| ExtractError::Corrupt("XML-часть не в UTF-8".into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub id: String,
    /// Последний сегмент URI типа: `officeDocument`, `styles`, `worksheet`, `slide`…
    pub kind: String,
    /// Нормализованный путь части внутри пакета. `None` — внешняя или небезопасная цель.
    pub target: Option<String>,
    pub external: bool,
}

/// Открытый пакет OOXML в памяти.
pub struct Package<'a> {
    bytes: &'a [u8],
    index: ZipIndex,
    by_path: HashMap<String, usize>,
    zip_limits: ZipLimits,
    remaining: u64,
    limits: ExtractionLimits,
    budget: &'a Budget,
    pub unsafe_targets: usize,
}

pub fn zip_limits(limits: &ExtractionLimits) -> ZipLimits {
    ZipLimits {
        max_archive_bytes: limits.max_input_bytes,
        max_entries: limits.max_office_entries,
        max_entry_bytes: limits.max_office_part_bytes,
        max_total_bytes: limits.max_office_total_bytes,
        ..ZipLimits::default()
    }
}

pub fn map_zip_error(error: ZipRejection) -> ExtractError {
    match error {
        ZipRejection::NotZip | ZipRejection::Corrupt(_) | ZipRejection::Io => {
            ExtractError::Corrupt(format!("Документ повреждён: {}", error.message()))
        }
        ZipRejection::Encrypted => ExtractError::Encrypted("Части документа зашифрованы".into()),
        ZipRejection::UnsupportedMethod(_) | ZipRejection::Zip64 | ZipRejection::MultiDisk => {
            ExtractError::Unsupported(error.message())
        }
        ZipRejection::ArchiveTooLarge => ExtractError::TooLarge(error.message()),
        other => ExtractError::Rejected(format!("Документ отклонён: {}", other.message())),
    }
}

impl<'a> Package<'a> {
    pub fn open(
        bytes: &'a [u8],
        limits: &ExtractionLimits,
        budget: &'a Budget,
    ) -> ExtractResult<Self> {
        let zip_limits = zip_limits(limits);
        let index = zip_safe::read_index(bytes, &zip_limits).map_err(map_zip_error)?;
        let by_path = index
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.is_dir)
            .map(|(position, entry)| (entry.path.to_lowercase(), position))
            .collect();
        Ok(Self {
            bytes,
            index,
            by_path,
            zip_limits,
            remaining: limits.max_office_total_bytes,
            limits: *limits,
            budget,
            unsafe_targets: 0,
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = &ZipEntry> {
        self.index.files()
    }

    /// Пути частей сравниваются без учёта регистра (так требует OPC).
    pub fn has(&self, path: &str) -> bool {
        self.by_path.contains_key(&path.to_lowercase())
    }

    pub fn part_size(&self, path: &str) -> Option<u64> {
        self.by_path
            .get(&path.to_lowercase())
            .map(|&position| self.index.entries[position].uncompressed_size)
    }

    /// Читает часть в память с соблюдением лимитов. `Ok(None)` — части нет.
    pub fn read(&mut self, path: &str) -> ExtractResult<Option<Vec<u8>>> {
        self.budget.check()?;
        let Some(&position) = self.by_path.get(&path.to_lowercase()) else {
            return Ok(None);
        };
        let entry = &self.index.entries[position];
        if entry.uncompressed_size > self.limits.max_office_part_bytes {
            return Err(ExtractError::TooLarge(format!(
                "Часть документа «{}» слишком большая",
                entry.path
            )));
        }
        zip_safe::read_entry_to_vec(self.bytes, entry, &self.zip_limits, &mut self.remaining)
            .map(Some)
            .map_err(map_zip_error)
    }

    pub fn read_xml(&mut self, path: &str) -> ExtractResult<Option<String>> {
        match self.read(path)? {
            Some(data) => xml_string(data).map(Some),
            None => Ok(None),
        }
    }

    /// Relationships части (`word/document.xml` → `word/_rels/document.xml.rels`).
    pub fn relationships(&mut self, part: &str) -> ExtractResult<Vec<Relationship>> {
        let (directory, file) = match part.rsplit_once('/') {
            Some((directory, file)) => (directory, file),
            None => ("", part),
        };
        let rels_path = if part.is_empty() {
            "_rels/.rels".to_owned()
        } else if directory.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{directory}/_rels/{file}.rels")
        };
        let Some(xml) = self.read_xml(&rels_path)? else {
            return Ok(Vec::new());
        };
        let budget = self.budget;
        let mut reader = Xml::new(&xml, budget, &self.limits);
        let mut relationships = Vec::new();
        loop {
            match reader.next_node()? {
                Node::Start(start) | Node::Empty(start) if name_of(&start) == "Relationship" => {
                    let id = attr(&start, "Id").unwrap_or_default();
                    let kind = attr(&start, "Type")
                        .map(|value| value.rsplit('/').next().unwrap_or("").to_owned())
                        .unwrap_or_default();
                    let external = attr(&start, "TargetMode")
                        .is_some_and(|mode| mode.eq_ignore_ascii_case("external"));
                    let target = if external {
                        None
                    } else {
                        let resolved = attr(&start, "Target")
                            .and_then(|target| resolve_target(directory, &target));
                        if resolved.is_none() {
                            self.unsafe_targets += 1;
                        }
                        resolved
                    };
                    relationships.push(Relationship {
                        id,
                        kind,
                        target,
                        external,
                    });
                }
                Node::Eof => break,
                _ => {}
            }
        }
        Ok(relationships)
    }

    /// Главная часть документа из `_rels/.rels` (`officeDocument`), иначе `fallback`.
    pub fn main_part(&mut self, fallback: &str) -> ExtractResult<String> {
        let root = self.relationships("")?;
        Ok(root
            .iter()
            .find(|relationship| relationship.kind == "officeDocument")
            .and_then(|relationship| relationship.target.clone())
            .filter(|target| self.has(target))
            .unwrap_or_else(|| fallback.to_owned()))
    }

    /// Предупреждения уровня пакета: макросы, встроенные объекты, внешние ссылки.
    pub fn package_warnings(&mut self, collector: &mut Collector) {
        let mut macros = false;
        let mut embedded = 0_usize;
        for entry in self.index.files() {
            let lower = entry.path.to_lowercase();
            if lower.ends_with("vbaproject.bin") || lower.ends_with("vbadata.xml") {
                macros = true;
            }
            if lower.contains("/embeddings/") || lower.contains("/activex/") {
                embedded += 1;
            }
        }
        if macros {
            collector.warn(
                WarningCode::MacrosIgnored,
                "В документе есть макросы VBA: они не запускаются и не извлекаются",
                None,
            );
        }
        if embedded > 0 {
            collector.warn(
                WarningCode::EmbeddedObjectsSkipped,
                format!("Встроенные объекты и вложения не извлекаются ({embedded})"),
                None,
            );
        }
    }

    /// Свойства документа: `docProps/core.xml` и `docProps/app.xml`.
    pub fn properties(&mut self) -> ExtractResult<ExtractionMetadata> {
        let mut metadata = ExtractionMetadata::default();
        let root = self.relationships("")?;
        let core = root
            .iter()
            .find(|relationship| relationship.kind == "core-properties")
            .and_then(|relationship| relationship.target.clone())
            .unwrap_or_else(|| "docProps/core.xml".into());
        let app = root
            .iter()
            .find(|relationship| relationship.kind == "extended-properties")
            .and_then(|relationship| relationship.target.clone())
            .unwrap_or_else(|| "docProps/app.xml".into());
        if let Some(xml) = self.optional_xml(&core)? {
            for (field, value) in simple_fields(&xml, self.budget, &self.limits)? {
                match field.as_str() {
                    "title" => metadata.title = Some(value),
                    "creator" => metadata.author = Some(value),
                    "subject" => metadata.subject = Some(value),
                    "created" => metadata.created = Some(value),
                    "modified" => metadata.modified = Some(value),
                    "language" => metadata.language = Some(value),
                    _ => {}
                }
            }
        }
        if let Some(xml) = self.optional_xml(&app)? {
            for (field, value) in simple_fields(&xml, self.budget, &self.limits)? {
                match field.as_str() {
                    "Application" => metadata.application = Some(value),
                    "Pages" => metadata.page_count = value.parse().ok(),
                    "Words" => metadata.word_count = value.parse().ok(),
                    "Slides" => metadata.slide_count = value.parse().ok(),
                    _ => {}
                }
            }
        }
        Ok(metadata)
    }

    /// Необязательная часть: повреждение или превышение лимита не мешает разбору
    /// остального документа (кроме прерывания по времени/отмене и DTD).
    pub fn optional_xml(&mut self, path: &str) -> ExtractResult<Option<String>> {
        match self.read_xml(path) {
            Ok(value) => Ok(value),
            Err(error @ (ExtractError::Interrupted(_) | ExtractError::Rejected(_))) => Err(error),
            Err(_) => Ok(None),
        }
    }

    pub fn limits(&self) -> &ExtractionLimits {
        &self.limits
    }

    pub fn budget(&self) -> &'a Budget {
        self.budget
    }
}

/// Нормализует цель relationship относительно каталога исходной части.
/// Абсолютная цель (`/xl/worksheets/sheet1.xml`) отсчитывается от корня пакета.
/// `..` за корень, `\`, схемы URI и управляющие символы → `None`.
pub fn resolve_target(base_directory: &str, target: &str) -> Option<String> {
    if target.is_empty()
        || target.contains('\\')
        || target.contains(':')
        || target.contains('\0')
        || target.chars().any(char::is_control)
    {
        return None;
    }
    let target = target.split(['#', '?']).next().unwrap_or("");
    let (mut stack, relative) = match target.strip_prefix('/') {
        Some(rest) => (Vec::new(), rest),
        None => (
            base_directory
                .split('/')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>(),
            target,
        ),
    };
    for segment in relative.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        return None;
    }
    Some(stack.join("/"))
}

/// Простые поля вида `<dc:title>…</dc:title>` первого уровня.
fn simple_fields(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
) -> ExtractResult<Vec<(String, String)>> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut fields = Vec::new();
    let mut depth = 0_usize;
    let mut current: Option<(String, String)> = None;
    loop {
        match reader.next_node()? {
            Node::Start(start) => {
                depth += 1;
                if depth == 2 {
                    current = Some((name_of(&start).to_owned(), String::new()));
                }
            }
            Node::Text(text) => {
                if let Some((_, value)) = current.as_mut() {
                    value.push_str(&text);
                }
            }
            Node::End(_) => {
                if depth == 2 {
                    if let Some((field, value)) = current.take() {
                        let value = clean_text(value.trim())
                            .chars()
                            .take(500)
                            .collect::<String>();
                        if !value.is_empty() {
                            fields.push((field, value));
                        }
                    }
                }
                depth = depth.saturating_sub(1);
            }
            Node::Empty(_) => {}
            Node::Eof => break,
        }
    }
    Ok(fields)
}

/// Номер столбца (1-based) → буквы: 1 → A, 28 → AB.
pub fn column_letters(mut column: u32) -> String {
    let mut letters = Vec::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        letters.push(char::from(b'A' + remainder as u8));
        column = (column - 1) / 26;
    }
    letters.iter().rev().collect()
}

/// `B12` → (столбец 2, строка 12).
pub fn parse_cell_ref(reference: &str) -> Option<(u32, u32)> {
    let reference = reference.trim().trim_start_matches('$');
    let letters: String = reference
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let digits = reference[letters.len()..].trim_start_matches('$');
    let row: u32 = digits.parse().ok()?;
    let column = letters.chars().try_fold(0_u32, |accumulator, ch| {
        accumulator
            .checked_mul(26)?
            .checked_add(u32::from(ch.to_ascii_uppercase() as u8 - b'A' + 1))
    })?;
    (row > 0 && column > 0).then_some((column, row))
}

#[cfg(test)]
mod tests {
    use super::{column_letters, parse_cell_ref, resolve_target};

    #[test]
    fn relationship_targets_stay_inside_the_package() {
        assert_eq!(
            resolve_target("word", "media/image1.png").as_deref(),
            Some("word/media/image1.png")
        );
        assert_eq!(
            resolve_target("ppt/slides", "../notesSlides/notesSlide1.xml").as_deref(),
            Some("ppt/notesSlides/notesSlide1.xml")
        );
        assert_eq!(
            resolve_target("xl", "/xl/worksheets/sheet1.xml").as_deref(),
            Some("xl/worksheets/sheet1.xml")
        );
        assert_eq!(resolve_target("word", "../../etc/passwd"), None);
        assert_eq!(resolve_target("", "../outside.xml"), None);
        assert_eq!(resolve_target("word", "..\\..\\x.xml"), None);
        assert_eq!(resolve_target("word", "file:///etc/passwd"), None);
        assert_eq!(resolve_target("word", "C:/Windows/x.xml"), None);
        assert_eq!(resolve_target("word", ""), None);
    }

    #[test]
    fn cell_references_round_trip() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(28), "AB");
        assert_eq!(column_letters(16_384), "XFD");
        assert_eq!(parse_cell_ref("B12"), Some((2, 12)));
        assert_eq!(parse_cell_ref("$AB$3"), Some((28, 3)));
        assert_eq!(parse_cell_ref("12"), None);
        assert_eq!(parse_cell_ref("A0"), None);
        assert_eq!(parse_cell_ref("ABCD1"), None);
    }
}
