//! DOCX: абзацы, заголовки (по стилям и уровню структуры), списки и таблицы.
//!
//! Не извлекаются (с предупреждением): колонтитулы, сноски, комментарии,
//! изображения, встроенные объекты, скрытый текст, удалённый текст правок.

use std::collections::HashMap;

use super::model::Budget;
use super::{
    model::{
        clean_text, Block, Collector, ExtractResult, Extracted, ExtractionLimits, SourceFormat,
        WarningCode,
    },
    ooxml::{attr, flag, name_of, Node, Package, Xml},
};

#[derive(Debug, Clone, Default)]
struct Style {
    name: String,
    based_on: Option<String>,
    outline: Option<u8>,
    /// Нумерация, заданная самим стилем (так делают «Маркированный список» и т. п.).
    num_id: Option<String>,
    num_level: Option<u8>,
}

pub fn extract(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let mut package = Package::open(bytes, limits, budget)?;
    let mut collector = Collector::new(limits);
    let metadata = package.properties()?;
    package.package_warnings(&mut collector);

    let main = package.main_part("word/document.xml")?;
    let relationships = package.relationships(&main)?;
    let target = |kind: &str| {
        relationships
            .iter()
            .find(|relationship| relationship.kind == kind)
            .and_then(|relationship| relationship.target.clone())
    };
    let styles = match target("styles") {
        Some(path) => match package.optional_xml(&path)? {
            Some(xml) => parse_styles(&xml, budget, limits)?,
            None => HashMap::new(),
        },
        None => HashMap::new(),
    };
    let numbering = match target("numbering") {
        Some(path) => match package.optional_xml(&path)? {
            Some(xml) => parse_numbering(&xml, budget, limits)?,
            None => HashMap::new(),
        },
        None => HashMap::new(),
    };
    let mut skipped_parts = Vec::new();
    for (kind, label) in [
        ("header", "колонтитулы"),
        ("footer", "колонтитулы"),
        ("footnotes", "сноски"),
        ("endnotes", "концевые сноски"),
        ("comments", "комментарии"),
    ] {
        if relationships
            .iter()
            .any(|relationship| relationship.kind == kind)
            && !skipped_parts.contains(&label)
        {
            skipped_parts.push(label);
        }
    }
    if relationships
        .iter()
        .any(|relationship| relationship.external && relationship.kind != "hyperlink")
    {
        collector.warn(
            WarningCode::ExternalLinksIgnored,
            "Документ ссылается на внешние файлы или шаблоны — они не открываются",
            None,
        );
    }

    let xml = package
        .read_xml(&main)?
        .ok_or_else(|| super::model::ExtractError::Corrupt("В DOCX нет основного текста".into()))?;
    let mut state = DocState::new(&styles, &numbering);
    parse_body(&xml, budget, limits, &mut state, &mut collector)?;

    if !skipped_parts.is_empty() {
        collector.warn(
            WarningCode::PartsSkipped,
            format!("Не извлекаются: {}", skipped_parts.join(", ")),
            None,
        );
    }
    if state.images > 0 {
        collector.warn(
            WarningCode::ImagesSkipped,
            format!(
                "Изображения и фигуры без текста не извлекаются ({})",
                state.images
            ),
            None,
        );
    }
    if state.objects > 0 {
        collector.warn(
            WarningCode::EmbeddedObjectsSkipped,
            format!("Встроенные объекты не извлекаются ({})", state.objects),
            None,
        );
    }
    if state.hidden_runs > 0 {
        collector.warn(
            WarningCode::HiddenContent,
            "Скрытый текст документа пропущен",
            None,
        );
    }
    if state.deleted_runs > 0 {
        collector.warn(
            WarningCode::TrackedChangesSkipped,
            "Удалённый в режиме правок текст не включён",
            None,
        );
    }
    if state.alt_chunks > 0 {
        collector.warn(
            WarningCode::PartsSkipped,
            "Вставленные внешние фрагменты (altChunk) не извлекаются",
            Some("altChunk".into()),
        );
    }
    let mut extracted = collector.finish(SourceFormat::Docx, metadata);
    if extracted.text.is_empty() {
        extracted.warnings.push(super::model::ExtractionWarning {
            code: WarningCode::EmptyDocument,
            message: "В документе нет текста".into(),
            location: None,
        });
    }
    Ok(extracted)
}

fn parse_styles(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
) -> ExtractResult<HashMap<String, Style>> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut styles = HashMap::new();
    let mut current: Option<(String, Style)> = None;
    loop {
        match reader.next_node()? {
            Node::Start(start) | Node::Empty(start) => match name_of(&start) {
                "style" => {
                    let paragraph = attr(&start, "type").as_deref() != Some("character");
                    current = attr(&start, "styleId")
                        .filter(|_| paragraph)
                        .map(|id| (id, Style::default()));
                }
                "name" => {
                    if let Some((_, style)) = current.as_mut() {
                        style.name = attr(&start, "val").unwrap_or_default().to_lowercase();
                    }
                }
                "basedOn" => {
                    if let Some((_, style)) = current.as_mut() {
                        style.based_on = attr(&start, "val");
                    }
                }
                "outlineLvl" => {
                    if let Some((_, style)) = current.as_mut() {
                        style.outline = attr(&start, "val").and_then(|value| value.parse().ok());
                    }
                }
                "numId" => {
                    if let Some((_, style)) = current.as_mut() {
                        style.num_id = attr(&start, "val");
                    }
                }
                "ilvl" => {
                    if let Some((_, style)) = current.as_mut() {
                        style.num_level = attr(&start, "val").and_then(|value| value.parse().ok());
                    }
                }
                _ => {}
            },
            Node::End(name) if name == "style" => {
                if let Some((id, style)) = current.take() {
                    styles.insert(id, style);
                }
            }
            Node::Eof => break,
            _ => {}
        }
    }
    Ok(styles)
}

/// numId → (ilvl → нумерованный ли список).
fn parse_numbering(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
) -> ExtractResult<HashMap<String, HashMap<u8, bool>>> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut abstract_levels: HashMap<String, HashMap<u8, bool>> = HashMap::new();
    let mut nums: HashMap<String, String> = HashMap::new();
    let mut abstract_id: Option<String> = None;
    let mut level: Option<u8> = None;
    let mut num_id: Option<String> = None;
    loop {
        match reader.next_node()? {
            Node::Start(start) | Node::Empty(start) => match name_of(&start) {
                "abstractNum" => abstract_id = attr(&start, "abstractNumId"),
                "lvl" => level = attr(&start, "ilvl").and_then(|value| value.parse().ok()),
                "numFmt" => {
                    if let (Some(id), Some(level)) = (&abstract_id, level) {
                        let format = attr(&start, "val").unwrap_or_default();
                        abstract_levels
                            .entry(id.clone())
                            .or_default()
                            .insert(level, format != "bullet" && format != "none");
                    }
                }
                "num" => num_id = attr(&start, "numId"),
                "abstractNumId" => {
                    if let (Some(id), Some(target)) = (&num_id, attr(&start, "val")) {
                        nums.insert(id.clone(), target);
                    }
                }
                _ => {}
            },
            Node::End(name) => match name.as_str() {
                "abstractNum" => abstract_id = None,
                "lvl" => level = None,
                "num" => num_id = None,
                _ => {}
            },
            Node::Eof => break,
            _ => {}
        }
    }
    Ok(nums
        .into_iter()
        .filter_map(|(num, abstract_id)| {
            abstract_levels
                .get(&abstract_id)
                .map(|levels| (num, levels.clone()))
        })
        .collect())
}

struct Paragraph {
    text: String,
    style: Option<String>,
    outline: Option<u8>,
    num_id: Option<String>,
    level: u8,
}

struct Table {
    rows: Vec<Vec<String>>,
    row: Option<Vec<String>>,
    cell: Option<String>,
}

struct DocState<'s> {
    styles: &'s HashMap<String, Style>,
    numbering: &'s HashMap<String, HashMap<u8, bool>>,
    paragraph: Option<Paragraph>,
    tables: Vec<Table>,
    in_text: bool,
    in_run: bool,
    run_hidden: bool,
    images: usize,
    objects: usize,
    hidden_runs: usize,
    deleted_runs: usize,
    alt_chunks: usize,
}

impl<'s> DocState<'s> {
    fn new(
        styles: &'s HashMap<String, Style>,
        numbering: &'s HashMap<String, HashMap<u8, bool>>,
    ) -> Self {
        Self {
            styles,
            numbering,
            paragraph: None,
            tables: Vec::new(),
            in_text: false,
            in_run: false,
            run_hidden: false,
            images: 0,
            objects: 0,
            hidden_runs: 0,
            deleted_runs: 0,
            alt_chunks: 0,
        }
    }

    fn append(&mut self, text: &str) {
        if self.run_hidden {
            return;
        }
        if let Some(paragraph) = self.paragraph.as_mut() {
            paragraph.text.push_str(text);
        }
    }

    /// Уровень заголовка 1–6 по стилю (с наследованием) или outlineLvl абзаца.
    fn heading_level(&self, paragraph: &Paragraph) -> Option<u8> {
        if let Some(level) = paragraph.outline.filter(|level| *level < 9) {
            return Some((level + 1).min(6));
        }
        let mut current = paragraph.style.clone();
        for _ in 0..10 {
            let style = self.styles.get(current.as_deref()?)?;
            let name = style.name.as_str();
            if name == "title" {
                return Some(1);
            }
            for prefix in ["heading ", "заголовок "] {
                if let Some(number) = name.strip_prefix(prefix) {
                    if let Ok(level) = number.trim().parse::<u8>() {
                        return Some(level.clamp(1, 6));
                    }
                }
            }
            if let Some(level) = style.outline.filter(|level| *level < 9) {
                return Some((level + 1).min(6));
            }
            current = style.based_on.clone();
        }
        None
    }

    /// Нумерация абзаца: своя или унаследованная от стиля.
    fn numbering_of(&self, paragraph: &Paragraph) -> Option<(String, u8)> {
        if let Some(num_id) = &paragraph.num_id {
            return Some((num_id.clone(), paragraph.level));
        }
        let mut current = paragraph.style.clone();
        for _ in 0..10 {
            let style = self.styles.get(current.as_deref()?)?;
            if let Some(num_id) = &style.num_id {
                return Some((num_id.clone(), style.num_level.unwrap_or(paragraph.level)));
            }
            current = style.based_on.clone();
        }
        None
    }

    fn finish_paragraph(&mut self, collector: &mut Collector) {
        let Some(paragraph) = self.paragraph.take() else {
            return;
        };
        let text = clean_text(paragraph.text.trim());
        if text.trim().is_empty() {
            return;
        }
        if let Some(table) = self.tables.last_mut() {
            if let Some(cell) = table.cell.as_mut() {
                if !cell.is_empty() {
                    cell.push('\n');
                }
                cell.push_str(&text);
            }
            return;
        }
        if let Some(level) = self.heading_level(&paragraph) {
            collector.push(Block::Heading { level, text });
            return;
        }
        if let Some((num_id, level)) = self.numbering_of(&paragraph).filter(|(id, _)| id != "0") {
            let ordered = self
                .numbering
                .get(&num_id)
                .and_then(|levels| levels.get(&level))
                .copied()
                .unwrap_or(false);
            collector.push(Block::ListItem {
                level: level.min(8),
                ordered,
                text,
            });
            return;
        }
        collector.push(Block::Paragraph { text });
    }
}

fn parse_body(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
    state: &mut DocState<'_>,
    collector: &mut Collector,
) -> ExtractResult<()> {
    let mut reader = Xml::new(xml, budget, limits);
    loop {
        if collector.is_full() {
            break;
        }
        match reader.next_node()? {
            Node::Start(start) => match name_of(&start) {
                // Word дублирует содержимое надписей в mc:Fallback; перенесённый
                // текст правок (moveFrom) и удалённые фрагменты не включаются.
                "Fallback" | "moveFrom" | "instrText" => reader.skip_subtree()?,
                "delText" => {
                    state.deleted_runs += 1;
                    reader.skip_subtree()?;
                }
                "p" => {
                    state.finish_paragraph(collector);
                    state.paragraph = Some(Paragraph {
                        text: String::new(),
                        style: None,
                        outline: None,
                        num_id: None,
                        level: 0,
                    });
                }
                "t" => state.in_text = true,
                "r" => {
                    state.in_run = true;
                    state.run_hidden = false;
                }
                "tbl" => state.tables.push(Table {
                    rows: Vec::new(),
                    row: None,
                    cell: None,
                }),
                "tr" => {
                    if let Some(table) = state.tables.last_mut() {
                        table.row = Some(Vec::new());
                    }
                }
                "tc" => {
                    if let Some(table) = state.tables.last_mut() {
                        table.cell = Some(String::new());
                    }
                }
                "drawing" | "pict" => state.images += 1,
                "object" => {
                    state.objects += 1;
                    reader.skip_subtree()?;
                }
                _ => {}
            },
            Node::Empty(start) => match name_of(&start) {
                "pStyle" => {
                    if let Some(paragraph) = state.paragraph.as_mut() {
                        paragraph.style = attr(&start, "val");
                    }
                }
                "outlineLvl" => {
                    if let Some(paragraph) = state.paragraph.as_mut() {
                        paragraph.outline =
                            attr(&start, "val").and_then(|value| value.parse().ok());
                    }
                }
                "ilvl" => {
                    if let Some(paragraph) = state.paragraph.as_mut() {
                        paragraph.level = attr(&start, "val")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(0);
                    }
                }
                "numId" => {
                    if let Some(paragraph) = state.paragraph.as_mut() {
                        paragraph.num_id = attr(&start, "val");
                    }
                }
                "vanish" | "specVanish" => {
                    if state.in_run && flag(&start, "val", true) {
                        state.run_hidden = true;
                        state.hidden_runs += 1;
                    }
                }
                "tab" | "ptab" => state.append("\t"),
                "br" | "cr" => state.append("\n"),
                "noBreakHyphen" => state.append("-"),
                "altChunk" => state.alt_chunks += 1,
                "p" => state.finish_paragraph(collector),
                "tc" => {
                    if let Some(table) = state.tables.last_mut() {
                        if let Some(row) = table.row.as_mut() {
                            row.push(String::new());
                        }
                    }
                }
                _ => {}
            },
            Node::Text(text) => {
                if state.in_text {
                    state.append(&text);
                }
            }
            Node::End(name) => match name.as_str() {
                "t" => state.in_text = false,
                "r" => {
                    state.in_run = false;
                    state.run_hidden = false;
                }
                "p" => state.finish_paragraph(collector),
                "tc" => {
                    state.finish_paragraph(collector);
                    if let Some(table) = state.tables.last_mut() {
                        let cell = table.cell.take().unwrap_or_default();
                        if let Some(row) = table.row.as_mut() {
                            row.push(cell);
                        }
                    }
                }
                "tr" => {
                    if let Some(table) = state.tables.last_mut() {
                        if let Some(row) = table.row.take() {
                            table.rows.push(row);
                        }
                    }
                }
                "tbl" => {
                    state.finish_paragraph(collector);
                    if let Some(table) = state.tables.pop() {
                        finish_table(table, state, collector, limits);
                    }
                }
                _ => {}
            },
            Node::Eof => break,
        }
    }
    state.finish_paragraph(collector);
    while let Some(table) = state.tables.pop() {
        finish_table(table, state, collector, limits);
    }
    if reader.unknown_entities > 0 {
        collector.warn(
            WarningCode::UnknownEntity,
            "Неизвестные XML-сущности пропущены без раскрытия",
            None,
        );
    }
    Ok(())
}

/// Таблица верхнего уровня становится блоком; вложенная — текстом ячейки родителя.
fn finish_table(
    table: Table,
    state: &mut DocState<'_>,
    collector: &mut Collector,
    limits: &ExtractionLimits,
) {
    let rows: Vec<Vec<String>> = table
        .rows
        .into_iter()
        .filter(|row| row.iter().any(|cell| !cell.trim().is_empty()))
        .collect();
    if rows.is_empty() {
        return;
    }
    if let Some(parent) = state.tables.last_mut() {
        if let Some(cell) = parent.cell.as_mut() {
            for row in &rows {
                if !cell.is_empty() {
                    cell.push('\n');
                }
                cell.push_str(&row.join(" | "));
            }
        }
        return;
    }
    let truncated = rows.len() > limits.max_table_rows;
    let mut rows = rows;
    rows.truncate(limits.max_table_rows);
    for row in &mut rows {
        row.truncate(limits.max_table_columns);
    }
    if truncated {
        collector.warn(
            WarningCode::RowsTruncated,
            format!("Таблица обрезана до {} строк", limits.max_table_rows),
            None,
        );
    }
    collector.push(Block::Table { rows, truncated });
}
