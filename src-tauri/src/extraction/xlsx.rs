//! XLSX: названия листов, объявленный и фактический диапазоны, значения и
//! тексты формул.
//!
//! Формулы никогда не вычисляются: показывается значение, которое Excel
//! сохранил в файле, и текст формулы. Макросы, внешние связи, диаграммы и
//! сводные таблицы не открываются.

use std::collections::{HashMap, HashSet};

use super::{
    model::{
        clean_text, Block, Budget, Collector, ExtractError, ExtractResult, Extracted,
        ExtractionLimits, SheetCell, SourceFormat, WarningCode,
    },
    ooxml::{attr, column_letters, flag, name_of, parse_cell_ref, rel_attr, Node, Package, Xml},
};

struct SheetRef {
    name: String,
    rel_id: String,
    hidden: bool,
}

pub fn extract(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let mut package = Package::open(bytes, limits, budget)?;
    let mut collector = Collector::new(limits);
    let mut metadata = package.properties()?;
    package.package_warnings(&mut collector);

    let workbook_path = package.main_part("xl/workbook.xml")?;
    let relationships = package.relationships(&workbook_path)?;
    let workbook = package
        .read_xml(&workbook_path)?
        .ok_or_else(|| ExtractError::Corrupt("В XLSX нет описания книги".into()))?;
    let (sheets, date1904) = parse_workbook(&workbook, budget, limits)?;
    metadata.sheet_count = Some(sheets.len().min(u32::MAX as usize) as u32);

    let target = |kind: &str| {
        relationships
            .iter()
            .find(|relationship| relationship.kind == kind)
            .and_then(|relationship| relationship.target.clone())
    };
    let shared = match target("sharedStrings") {
        Some(path) => match package.read_xml(&path) {
            Ok(Some(xml)) => parse_shared_strings(&xml, budget, limits, &mut collector)?,
            Ok(None) => Vec::new(),
            Err(error @ (ExtractError::Interrupted(_) | ExtractError::Rejected(_))) => {
                return Err(error)
            }
            Err(_) => {
                collector.warn(
                    WarningCode::PartUnreadable,
                    "Таблица строк книги не прочитана: текстовые значения ячеек могут отсутствовать",
                    None,
                );
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let date_styles = match target("styles") {
        Some(path) => match package.optional_xml(&path)? {
            Some(xml) => parse_date_styles(&xml, budget, limits)?,
            None => HashSet::new(),
        },
        None => HashSet::new(),
    };
    if relationships
        .iter()
        .any(|relationship| relationship.kind == "externalLink" || relationship.external)
    {
        collector.warn(
            WarningCode::ExternalLinksIgnored,
            "Книга ссылается на внешние файлы — связи не открываются и не обновляются",
            None,
        );
    }

    let mut context = SheetContext {
        shared: &shared,
        date_styles: &date_styles,
        date1904,
        total_cells: 0,
        formulas: 0,
    };
    for (position, sheet) in sheets.iter().enumerate() {
        budget.check()?;
        if collector.is_full() {
            break;
        }
        if position >= limits.max_sheets {
            collector.warn(
                WarningCode::SheetsTruncated,
                format!(
                    "Извлечены первые {} листов из {}",
                    limits.max_sheets,
                    sheets.len()
                ),
                None,
            );
            break;
        }
        if context.total_cells >= limits.max_total_cells {
            collector.warn(
                WarningCode::CellsTruncated,
                format!(
                    "Достигнут общий лимит {} ячеек книги",
                    limits.max_total_cells
                ),
                None,
            );
            break;
        }
        let location = Some(format!("лист «{}»", sheet.name));
        let Some(relationship) = relationships
            .iter()
            .find(|relationship| relationship.id == sheet.rel_id)
        else {
            collector.warn(
                WarningCode::PartUnreadable,
                "Лист не найден в книге",
                location,
            );
            continue;
        };
        if relationship.kind != "worksheet" {
            let label = match relationship.kind.as_str() {
                "chartsheet" => "лист диаграммы",
                "dialogsheet" => "лист диалога",
                "macrosheet" | "xlMacrosheet" | "xlIntlMacrosheet" => "лист макросов",
                _ => "особый лист",
            };
            collector.warn(
                WarningCode::PartsSkipped,
                format!("Пропущен {label}"),
                location,
            );
            continue;
        }
        let Some(path) = relationship.target.clone() else {
            collector.warn(
                WarningCode::PartUnreadable,
                "Ссылка на лист указывает за пределы документа — отклонена",
                location,
            );
            continue;
        };
        let xml = match package.read_xml(&path) {
            Ok(Some(xml)) => xml,
            Ok(None) => {
                collector.warn(
                    WarningCode::PartUnreadable,
                    "Лист отсутствует в файле",
                    location,
                );
                continue;
            }
            Err(error @ (ExtractError::Interrupted(_) | ExtractError::Rejected(_))) => {
                return Err(error)
            }
            Err(_) => {
                collector.warn(
                    WarningCode::PartUnreadable,
                    "Лист слишком большой или повреждён — пропущен",
                    location,
                );
                continue;
            }
        };
        let sheet_relationships = package.relationships(&path)?;
        if sheet_relationships
            .iter()
            .any(|relationship| matches!(relationship.kind.as_str(), "drawing" | "chart"))
        {
            collector.warn(
                WarningCode::ChartsSkipped,
                "Диаграммы, рисунки и фигуры листа не извлекаются",
                location.clone(),
            );
        }
        if sheet_relationships.iter().any(|relationship| {
            matches!(relationship.kind.as_str(), "comments" | "threadedComment")
        }) {
            collector.warn(
                WarningCode::PartsSkipped,
                "Примечания к ячейкам не извлекаются",
                location.clone(),
            );
        }
        if sheet_relationships
            .iter()
            .any(|relationship| relationship.kind == "pivotTable")
        {
            collector.warn(
                WarningCode::PartsSkipped,
                "Сводные таблицы не пересчитываются; показаны только сохранённые значения",
                location.clone(),
            );
        }
        parse_sheet(&xml, sheet, &mut context, &mut collector, budget, limits)?;
    }
    if context.formulas > 0 {
        collector.warn(
            WarningCode::FormulasNotEvaluated,
            format!(
                "Формулы не вычисляются: для {} ячеек показан текст формулы и значение, сохранённое в файле",
                context.formulas
            ),
            None,
        );
    }
    let mut extracted = collector.finish(SourceFormat::Xlsx, metadata);
    if extracted.text.is_empty() {
        extracted.warnings.push(super::model::ExtractionWarning {
            code: WarningCode::EmptyDocument,
            message: "В книге нет заполненных ячеек".into(),
            location: None,
        });
    }
    Ok(extracted)
}

fn parse_workbook(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
) -> ExtractResult<(Vec<SheetRef>, bool)> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut sheets = Vec::new();
    let mut date1904 = false;
    loop {
        match reader.next_node()? {
            Node::Start(start) | Node::Empty(start) => match name_of(&start) {
                "workbookPr" => date1904 = flag(&start, "date1904", false),
                "sheet" => {
                    let name = clean_text(&attr(&start, "name").unwrap_or_default());
                    let rel_id = rel_attr(&start, "id").unwrap_or_default();
                    let hidden = attr(&start, "state").is_some_and(|state| state != "visible");
                    sheets.push(SheetRef {
                        name: if name.trim().is_empty() {
                            format!("Лист {}", sheets.len() + 1)
                        } else {
                            name.chars().take(64).collect()
                        },
                        rel_id,
                        hidden,
                    });
                }
                _ => {}
            },
            Node::Eof => break,
            _ => {}
        }
    }
    Ok((sheets, date1904))
}

fn parse_shared_strings(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
    collector: &mut Collector,
) -> ExtractResult<Vec<String>> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut strings = Vec::new();
    let mut current: Option<String> = None;
    let mut in_text = false;
    loop {
        match reader.next_node()? {
            Node::Start(start) => match name_of(&start) {
                "si" => current = Some(String::new()),
                "t" => in_text = true,
                // Фонетические подсказки (японский текст) не являются содержимым ячейки.
                "rPh" => reader.skip_subtree()?,
                _ => {}
            },
            Node::Empty(start) if name_of(&start) == "si" => strings.push(String::new()),
            Node::Text(text) => {
                if in_text {
                    if let Some(value) = current.as_mut() {
                        value.push_str(&text);
                    }
                }
            }
            Node::End(name) => match name.as_str() {
                "t" => in_text = false,
                "si" => {
                    if strings.len() >= limits.max_shared_strings {
                        collector.warn(
                            WarningCode::CellsTruncated,
                            format!(
                                "В книге больше {} уникальных строк — остальные не прочитаны",
                                limits.max_shared_strings
                            ),
                            None,
                        );
                        break;
                    }
                    strings.push(clean_text(&current.take().unwrap_or_default()));
                }
                _ => {}
            },
            Node::Eof => break,
            _ => {}
        }
    }
    Ok(strings)
}

/// Индексы `cellXfs`, у которых формат числа — дата/время.
fn parse_date_styles(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
) -> ExtractResult<HashSet<usize>> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut custom: HashMap<u32, String> = HashMap::new();
    let mut formats: Vec<u32> = Vec::new();
    let mut in_cell_xfs = false;
    loop {
        match reader.next_node()? {
            Node::Start(start) | Node::Empty(start) => match name_of(&start) {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (
                        attr(&start, "numFmtId").and_then(|value| value.parse().ok()),
                        attr(&start, "formatCode"),
                    ) {
                        custom.insert(id, code);
                    }
                }
                "cellXfs" => in_cell_xfs = true,
                "xf" if in_cell_xfs => formats.push(
                    attr(&start, "numFmtId")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(0),
                ),
                _ => {}
            },
            Node::End(name) if name == "cellXfs" => in_cell_xfs = false,
            Node::Eof => break,
            _ => {}
        }
    }
    Ok(formats
        .into_iter()
        .enumerate()
        .filter(|(_, id)| is_date_format(*id, custom.get(id).map(String::as_str)))
        .map(|(index, _)| index)
        .collect())
}

fn is_date_format(id: u32, code: Option<&str>) -> bool {
    if matches!(id, 14..=22 | 27..=36 | 45..=47 | 50..=58) {
        return true;
    }
    let Some(code) = code else {
        return false;
    };
    // Убираем строки в кавычках, экранированные символы и секции [цвет]/[$-локаль].
    let mut cleaned = String::new();
    let mut chars = code.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                for inner in chars.by_ref() {
                    if inner == '"' {
                        break;
                    }
                }
            }
            '\\' | '_' | '*' => {
                chars.next();
            }
            '[' => {
                let section: String = chars.by_ref().take_while(|inner| *inner != ']').collect();
                if section.eq_ignore_ascii_case("h")
                    || section.eq_ignore_ascii_case("hh")
                    || section.eq_ignore_ascii_case("mm")
                    || section.eq_ignore_ascii_case("ss")
                {
                    cleaned.push('h');
                }
            }
            other => cleaned.push(other.to_ascii_lowercase()),
        }
    }
    let lower = cleaned.as_str();
    lower != "general"
        && lower
            .chars()
            .any(|ch| matches!(ch, 'd' | 'y' | 'h' | 's' | 'm' | 'д' | 'г' | 'ч'))
        && !lower
            .chars()
            .all(|ch| matches!(ch, '0' | '#' | '.' | ',' | ' ' | '%' | 'e' | '+' | '-'))
}

/// Серийный номер даты Excel → ISO 8601 (без часового пояса — так хранит Excel).
pub fn excel_serial_to_iso(serial: f64, date1904: bool) -> Option<String> {
    if !serial.is_finite() || !(0.0..2_958_466.0).contains(&serial) {
        return None;
    }
    let mut days = serial.floor() as i64;
    let mut seconds = ((serial - serial.floor()) * 86_400.0).round() as i64;
    if seconds >= 86_400 {
        days += 1;
        seconds -= 86_400;
    }
    let unix_days = if date1904 {
        days - 24_107
    } else {
        // В системе 1900 Excel считает несуществующее 29.02.1900 (серийный номер 60).
        if days < 60 {
            days - 25_568
        } else {
            days - 25_569
        }
    };
    let (year, month, day) = civil_from_days(unix_days);
    let date = format!("{year:04}-{month:02}-{day:02}");
    if seconds == 0 {
        return Some(date);
    }
    Some(format!(
        "{date}T{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    ))
}

/// Дни от 1970-01-01 → (год, месяц, день) по алгоритму Хиннанта.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

struct SheetContext<'c> {
    shared: &'c [String],
    date_styles: &'c HashSet<usize>,
    date1904: bool,
    total_cells: u64,
    formulas: u64,
}

#[derive(Default)]
struct CellState {
    reference: Option<String>,
    kind: Option<String>,
    style: Option<usize>,
    value: String,
    inline: String,
    formula: Option<String>,
    shared_index: Option<String>,
    shared_master: bool,
    in_value: bool,
    in_formula: bool,
    in_inline_text: bool,
}

fn parse_sheet(
    xml: &str,
    sheet: &SheetRef,
    context: &mut SheetContext<'_>,
    collector: &mut Collector,
    budget: &Budget,
    limits: &ExtractionLimits,
) -> ExtractResult<()> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut declared: Option<String> = None;
    let mut cell: Option<CellState> = None;
    let mut row_cells: Vec<(String, String, Option<String>)> = Vec::new();
    let mut preview: Vec<SheetCell> = Vec::new();
    let mut shared_masters: HashMap<String, (String, String)> = HashMap::new();
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    let mut cell_count = 0_u64;
    let mut formula_count = 0_u64;
    let mut implicit_column = 0_u32;
    let mut implicit_row = 0_u32;
    let mut truncated = false;
    let location = Some(format!("лист «{}»", sheet.name));

    collector.push_text(&format!(
        "## Лист «{}»{}\n",
        sheet.name,
        if sheet.hidden {
            " (скрытый)"
        } else {
            ""
        }
    ));

    let flush_row = |row_cells: &mut Vec<(String, String, Option<String>)>,
                     collector: &mut Collector| {
        if row_cells.is_empty() {
            return;
        }
        let line = row_cells
            .iter()
            .map(|(reference, value, formula)| match formula {
                Some(formula) => format!("{reference}: {value} [{formula}]"),
                None => format!("{reference}: {value}"),
            })
            .collect::<Vec<_>>()
            .join(" | ");
        collector.push_text(&line);
        collector.push_text("\n");
        row_cells.clear();
    };

    loop {
        let node = reader.next_node()?;
        let empty = matches!(node, Node::Empty(_));
        match node {
            Node::Start(start) | Node::Empty(start) if cell.is_none() => {
                match name_of(&start) {
                    "dimension" => declared = attr(&start, "ref").filter(|value| value.len() <= 32),
                    "row" => {
                        flush_row(&mut row_cells, collector);
                        implicit_row = attr(&start, "r")
                            .and_then(|value| value.parse().ok())
                            .unwrap_or(implicit_row + 1);
                        implicit_column = 0;
                    }
                    "c" => {
                        implicit_column += 1;
                        let reference =
                            attr(&start, "r").filter(|value| parse_cell_ref(value).is_some());
                        if let Some((column, _)) = reference.as_deref().and_then(parse_cell_ref) {
                            implicit_column = column;
                        }
                        cell = Some(CellState {
                            reference: Some(reference.unwrap_or_else(|| {
                                format!(
                                    "{}{}",
                                    column_letters(implicit_column),
                                    implicit_row.max(1)
                                )
                            })),
                            kind: attr(&start, "t"),
                            style: attr(&start, "s").and_then(|value| value.parse().ok()),
                            ..CellState::default()
                        });
                        if empty {
                            // `<c r="A1" s="1"/>` — оформленная пустая ячейка.
                            cell = None;
                        }
                    }
                    _ => {}
                }
            }
            Node::Start(start) => {
                let Some(state) = cell.as_mut() else { continue };
                match name_of(&start) {
                    "v" => state.in_value = true,
                    "f" => {
                        state.in_formula = true;
                        state.formula = Some(String::new());
                        if attr(&start, "t").as_deref() == Some("shared") {
                            state.shared_index = attr(&start, "si");
                            state.shared_master = attr(&start, "ref").is_some();
                        }
                    }
                    "t" => state.in_inline_text = true,
                    "rPh" => reader.skip_subtree()?,
                    _ => {}
                }
            }
            Node::Empty(start) => {
                let Some(state) = cell.as_mut() else { continue };
                if name_of(&start) == "f" && attr(&start, "t").as_deref() == Some("shared") {
                    state.shared_index = attr(&start, "si");
                    state.formula = Some(String::new());
                }
            }
            Node::Text(text) => {
                let Some(state) = cell.as_mut() else { continue };
                if state.in_value {
                    state.value.push_str(&text);
                } else if state.in_formula {
                    if let Some(formula) = state.formula.as_mut() {
                        formula.push_str(&text);
                    }
                } else if state.in_inline_text {
                    state.inline.push_str(&text);
                }
            }
            Node::End(name) => match name.as_str() {
                "v" => {
                    if let Some(state) = cell.as_mut() {
                        state.in_value = false;
                    }
                }
                "f" => {
                    if let Some(state) = cell.as_mut() {
                        state.in_formula = false;
                    }
                }
                "t" => {
                    if let Some(state) = cell.as_mut() {
                        state.in_inline_text = false;
                    }
                }
                "c" => {
                    let Some(state) = cell.take() else { continue };
                    let reference = state.reference.clone().unwrap_or_default();
                    let formula = resolve_formula(&state, &reference, &mut shared_masters);
                    let value = cell_value(&state, context);
                    if value.is_empty() && formula.is_none() {
                        continue;
                    }
                    cell_count += 1;
                    context.total_cells += 1;
                    if formula.is_some() {
                        formula_count += 1;
                        context.formulas += 1;
                    }
                    if let Some((column, row)) = parse_cell_ref(&reference) {
                        bounds = Some(match bounds {
                            None => (column, row, column, row),
                            Some((c0, r0, c1, r1)) => {
                                (c0.min(column), r0.min(row), c1.max(column), r1.max(row))
                            }
                        });
                    }
                    if preview.len() < limits.max_structure_cells_per_sheet {
                        preview.push(SheetCell {
                            reference: reference.clone(),
                            value: value.clone(),
                            formula: formula.clone(),
                        });
                    }
                    row_cells.push((reference, value, formula));
                    if cell_count >= limits.max_cells_per_sheet
                        || context.total_cells >= limits.max_total_cells
                    {
                        truncated = true;
                        collector.warn(
                            WarningCode::CellsTruncated,
                            format!("Извлечены первые {cell_count} заполненных ячеек листа"),
                            location.clone(),
                        );
                        break;
                    }
                }
                "row" => flush_row(&mut row_cells, collector),
                _ => {}
            },
            Node::Eof => break,
        }
    }
    flush_row(&mut row_cells, collector);
    let used_range = bounds
        .map(|(c0, r0, c1, r1)| format!("{}{r0}:{}{r1}", column_letters(c0), column_letters(c1)));
    let (column_count, row_count) = bounds
        .map(|(c0, r0, c1, r1)| (c1 - c0 + 1, r1 - r0 + 1))
        .unwrap_or((0, 0));
    if let Some(range) = &used_range {
        collector.push_text(&format!("Диапазон с данными: {range}\n\n"));
    } else {
        collector.push_text("Лист пуст\n\n");
    }
    if (preview.len() as u64) < cell_count && !truncated {
        collector.warn(
            WarningCode::RowsTruncated,
            format!(
                "В просмотре первые {} ячеек; все ячейки — во вкладке «Текст»",
                limits.max_structure_cells_per_sheet
            ),
            location,
        );
    }
    collector.push_structure(Block::Sheet {
        name: sheet.name.clone(),
        hidden: sheet.hidden,
        declared_range: declared,
        used_range,
        row_count,
        column_count,
        cell_count,
        formula_count,
        cells: preview,
        truncated,
    });
    Ok(())
}

fn resolve_formula(
    state: &CellState,
    reference: &str,
    masters: &mut HashMap<String, (String, String)>,
) -> Option<String> {
    let text = state.formula.as_deref()?.trim().to_owned();
    match (&state.shared_index, text.is_empty()) {
        (Some(index), false) => {
            if state.shared_master || !masters.contains_key(index) {
                masters.insert(index.clone(), (reference.to_owned(), text.clone()));
            }
            Some(format!("={}", clean_text(&text)))
        }
        (Some(index), true) => masters.get(index).map(|(master, text)| {
            format!(
                "={} (общая формула из {master}; ссылки смещены)",
                clean_text(text)
            )
        }),
        (None, false) => Some(format!("={}", clean_text(&text))),
        (None, true) => None,
    }
}

fn cell_value(state: &CellState, context: &SheetContext<'_>) -> String {
    let raw = state.value.trim();
    let value = match state.kind.as_deref() {
        Some("s") => raw
            .parse::<usize>()
            .ok()
            .and_then(|index| context.shared.get(index).cloned())
            .unwrap_or_default(),
        Some("inlineStr") => state.inline.clone(),
        Some("b") => match raw {
            "1" => "TRUE".to_owned(),
            "0" => "FALSE".to_owned(),
            other => other.to_owned(),
        },
        Some("str") | Some("e") | Some("d") => raw.to_owned(),
        _ => {
            let is_date = state
                .style
                .is_some_and(|style| context.date_styles.contains(&style));
            match (is_date, raw.parse::<f64>()) {
                (true, Ok(serial)) => {
                    excel_serial_to_iso(serial, context.date1904).unwrap_or_else(|| raw.to_owned())
                }
                _ => raw.to_owned(),
            }
        }
    };
    clean_text(&value).replace('\n', " ").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{excel_serial_to_iso, is_date_format};

    #[test]
    fn excel_dates_convert_in_both_systems() {
        assert_eq!(
            excel_serial_to_iso(45_000.0, false).as_deref(),
            Some("2023-03-15")
        );
        assert_eq!(
            excel_serial_to_iso(1.0, false).as_deref(),
            Some("1900-01-01")
        );
        assert_eq!(
            excel_serial_to_iso(61.0, false).as_deref(),
            Some("1900-03-01")
        );
        assert_eq!(
            excel_serial_to_iso(45_000.5, false).as_deref(),
            Some("2023-03-15T12:00:00")
        );
        assert_eq!(
            excel_serial_to_iso(0.0, true).as_deref(),
            Some("1904-01-01")
        );
        assert_eq!(excel_serial_to_iso(f64::NAN, false), None);
        assert_eq!(excel_serial_to_iso(-1.0, false), None);
    }

    #[test]
    fn date_formats_are_recognized_without_false_positives() {
        assert!(is_date_format(14, None));
        assert!(is_date_format(200, Some("dd.mm.yyyy")));
        assert!(is_date_format(201, Some("[$-419]d mmmm yyyy")));
        assert!(is_date_format(202, Some("[h]:mm")));
        assert!(!is_date_format(203, Some("0.00")));
        assert!(!is_date_format(204, Some("#,##0 \"руб.\"")));
        assert!(!is_date_format(205, Some("General")));
        assert!(!is_date_format(2, None));
    }
}
