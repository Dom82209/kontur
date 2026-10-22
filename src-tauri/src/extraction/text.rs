//! TXT, Markdown, CSV/TSV и Jupyter-ноутбуки.

use super::model::{
    clean_text, Block, Budget, Collector, ExtractError, ExtractResult, Extracted, ExtractionLimits,
    ExtractionMetadata, SourceFormat, WarningCode,
};

/// Раскодированный текст и использованная кодировка.
pub struct Decoded {
    pub text: String,
    pub encoding: &'static str,
    pub fallback: bool,
}

/// UTF-8 (с BOM или без), UTF-16 LE/BE по BOM, иначе Windows-1251 —
/// так сохраняют CSV русские версии Excel. Двоичный файл отклоняется.
pub fn decode(bytes: &[u8]) -> ExtractResult<Decoded> {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return std::str::from_utf8(rest)
            .map(|text| Decoded {
                text: text.to_owned(),
                encoding: "UTF-8",
                fallback: false,
            })
            .map_err(|_| {
                ExtractError::Corrupt("Файл с меткой UTF-8 содержит неверные байты".into())
            });
    }
    for (bom, little, label) in [
        (&b"\xFF\xFE"[..], true, "UTF-16LE"),
        (&b"\xFE\xFF"[..], false, "UTF-16BE"),
    ] {
        if let Some(rest) = bytes.strip_prefix(bom) {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|pair| {
                    if little {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .collect();
            return String::from_utf16(&units)
                .map(|text| Decoded {
                    text,
                    encoding: label,
                    fallback: false,
                })
                .map_err(|_| ExtractError::Corrupt("Неверная последовательность UTF-16".into()));
        }
    }
    let head = &bytes[..bytes.len().min(8192)];
    if head.contains(&0) {
        return Err(ExtractError::Unsupported(
            "Похоже на двоичный файл, а не на текст".into(),
        ));
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(Decoded {
            text: text.to_owned(),
            encoding: "UTF-8",
            fallback: false,
        }),
        Err(_) => Ok(Decoded {
            text: bytes.iter().map(|&byte| decode_cp1251(byte)).collect(),
            encoding: "Windows-1251",
            fallback: true,
        }),
    }
}

fn decode_cp1251(byte: u8) -> char {
    const HIGH: [char; 64] = [
        'Ђ', 'Ѓ', '‚', 'ѓ', '„', '…', '†', '‡', '€', '‰', 'Љ', '‹', 'Њ', 'Ќ', 'Ћ', 'Џ', 'ђ', '‘',
        '’', '“', '”', '•', '–', '—', '\u{FFFD}', '™', 'љ', '›', 'њ', 'ќ', 'ћ', 'џ', '\u{00A0}',
        'Ў', 'ў', 'Ј', '¤', 'Ґ', '¦', '§', 'Ё', '©', 'Є', '«', '¬', '\u{00AD}', '®', 'Ї', '°', '±',
        'І', 'і', 'ґ', 'µ', '¶', '·', 'ё', '№', 'є', '»', 'ј', 'Ѕ', 'ѕ', 'ї',
    ];
    match byte {
        0x00..=0x7F => byte as char,
        0x80..=0xBF => HIGH[usize::from(byte - 0x80)],
        // А–я подряд.
        _ => char::from_u32(0x0410 + u32::from(byte - 0xC0)).unwrap_or('\u{FFFD}'),
    }
}

fn prepare(
    bytes: &[u8],
    collector: &mut Collector,
    metadata: &mut ExtractionMetadata,
) -> ExtractResult<String> {
    let decoded = decode(bytes)?;
    metadata.encoding = Some(decoded.encoding.to_owned());
    if decoded.fallback {
        collector.warn(
            WarningCode::EncodingFallback,
            "Файл не в UTF-8: прочитан как Windows-1251",
            None,
        );
    }
    Ok(clean_text(&decoded.text))
}

pub fn extract_plain(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let mut collector = Collector::new(limits);
    let mut metadata = ExtractionMetadata::default();
    let text = prepare(bytes, &mut collector, &mut metadata)?;
    for (index, paragraph) in text.split("\n\n").enumerate() {
        if index.is_multiple_of(512) {
            budget.check()?;
        }
        let paragraph = paragraph.trim_matches('\n');
        if paragraph.trim().is_empty() {
            continue;
        }
        collector.push(Block::Paragraph {
            text: paragraph.to_owned(),
        });
        if collector.is_full() {
            break;
        }
    }
    metadata.word_count = Some(word_count(&text));
    finish_text(collector, SourceFormat::Text, metadata)
}

pub fn extract_markdown(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let mut collector = Collector::new(limits);
    let mut metadata = ExtractionMetadata::default();
    let text = prepare(bytes, &mut collector, &mut metadata)?;
    markdown_blocks(&text, &mut collector, budget)?;
    metadata.word_count = Some(word_count(&text));
    metadata.title = collector_first_heading(&text);
    finish_text(collector, SourceFormat::Markdown, metadata)
}

fn collector_first_heading(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty())
}

/// Разбор Markdown на блоки: заголовки ATX, списки, блоки кода, абзацы.
/// HTML внутри Markdown не интерпретируется и остаётся текстом.
pub fn markdown_blocks(
    text: &str,
    collector: &mut Collector,
    budget: &Budget,
) -> ExtractResult<()> {
    let mut paragraph: Vec<&str> = Vec::new();
    let mut code: Option<(Option<String>, Vec<&str>)> = None;
    let flush = |paragraph: &mut Vec<&str>, collector: &mut Collector| {
        if !paragraph.is_empty() {
            collector.push(Block::Paragraph {
                text: paragraph.join("\n"),
            });
            paragraph.clear();
        }
    };
    for (index, line) in text.lines().enumerate() {
        if index.is_multiple_of(1024) {
            budget.check()?;
        }
        if collector.is_full() {
            return Ok(());
        }
        let trimmed = line.trim_start();
        if let Some((language, lines)) = code.as_mut() {
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                collector.push(Block::Code {
                    language: language.clone(),
                    text: lines.join("\n"),
                });
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("```")
            .or_else(|| trimmed.strip_prefix("~~~"))
        {
            flush(&mut paragraph, collector);
            let language = rest.trim();
            code = Some((
                (!language.is_empty()).then(|| language.chars().take(32).collect()),
                Vec::new(),
            ));
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut paragraph, collector);
            continue;
        }
        let hashes = trimmed.chars().take_while(|&ch| ch == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            flush(&mut paragraph, collector);
            collector.push(Block::Heading {
                level: hashes as u8,
                text: trimmed[hashes..]
                    .trim()
                    .trim_end_matches('#')
                    .trim()
                    .to_owned(),
            });
            continue;
        }
        let indent = (line.len() - trimmed.len()) / 2;
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            flush(&mut paragraph, collector);
            collector.push(Block::ListItem {
                level: indent.min(8) as u8,
                ordered: false,
                text: item.trim().to_owned(),
            });
            continue;
        }
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits > 0
            && digits < 10
            && (trimmed[digits..].starts_with(". ") || trimmed[digits..].starts_with(") "))
        {
            flush(&mut paragraph, collector);
            collector.push(Block::ListItem {
                level: indent.min(8) as u8,
                ordered: true,
                text: trimmed[digits + 2..].trim().to_owned(),
            });
            continue;
        }
        paragraph.push(line);
    }
    if let Some((language, lines)) = code {
        collector.push(Block::Code {
            language,
            text: lines.join("\n"),
        });
    }
    flush(&mut paragraph, collector);
    Ok(())
}

pub fn extract_delimited(
    bytes: &[u8],
    format: SourceFormat,
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let mut collector = Collector::new(limits);
    let mut metadata = ExtractionMetadata::default();
    let text = prepare(bytes, &mut collector, &mut metadata)?;
    let delimiter = if format == SourceFormat::Tsv {
        '\t'
    } else {
        detect_delimiter(&text)
    };
    metadata.delimiter = Some(match delimiter {
        '\t' => "tab".to_owned(),
        other => other.to_string(),
    });

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row_count = 0_u64;
    let mut column_count = 0_usize;
    let mut table_truncated = false;
    let mut columns_truncated = false;
    let mut parser = DelimitedParser::new(&text, delimiter);
    while let Some(row) = parser.next_row() {
        row_count += 1;
        if row_count.is_multiple_of(1024) {
            budget.check()?;
        }
        column_count = column_count.max(row.len());
        if !collector.is_full() {
            collector.push_text(&row.join("\t"));
            collector.push_text("\n");
        }
        if rows.len() < limits.max_table_rows {
            let mut row = row;
            if row.len() > limits.max_table_columns {
                row.truncate(limits.max_table_columns);
                columns_truncated = true;
            }
            rows.push(row);
        } else {
            table_truncated = true;
        }
        if collector.is_full() && table_truncated {
            // Остальные строки только считаются: лимит текста уже исчерпан.
            continue;
        }
    }
    if parser.unterminated_quote {
        collector.warn(
            WarningCode::PartUnreadable,
            "Незакрытая кавычка в конце файла: последняя строка прочитана как есть",
            None,
        );
    }
    if table_truncated {
        collector.warn(
            WarningCode::RowsTruncated,
            format!(
                "В таблице просмотра первые {} строк; все строки — во вкладке «Текст»",
                limits.max_table_rows
            ),
            None,
        );
    }
    if columns_truncated {
        collector.warn(
            WarningCode::CellsTruncated,
            format!(
                "В таблице просмотра первые {} столбцов",
                limits.max_table_columns
            ),
            None,
        );
    }
    metadata.row_count = Some(row_count);
    metadata.column_count = Some(column_count.min(u32::MAX as usize) as u32);
    collector.push_structure(Block::Table {
        rows,
        truncated: table_truncated,
    });
    let extracted = collector.finish(format, metadata);
    Ok(downgrade_preview_only(extracted))
}

/// Обрезка только таблицы просмотра не теряет текст: статус остаётся `complete`.
fn downgrade_preview_only(mut extracted: Extracted) -> Extracted {
    use super::model::ExtractionStatus;
    let only_preview = extracted.warnings.iter().all(|warning| {
        matches!(
            warning.code,
            WarningCode::RowsTruncated
                | WarningCode::CellsTruncated
                | WarningCode::EncodingFallback
                | WarningCode::ExtensionMismatch
        )
    });
    if extracted.status == ExtractionStatus::Partial && only_preview && !extracted.truncated {
        extracted.status = ExtractionStatus::Complete;
    }
    extracted
}

fn detect_delimiter(text: &str) -> char {
    let sample: Vec<&str> = text.lines().take(20).collect();
    let mut best = (',', 0_usize);
    for candidate in [';', ',', '\t', '|'] {
        let counts: Vec<usize> = sample
            .iter()
            .map(|line| count_outside_quotes(line, candidate))
            .collect();
        let Some(&first) = counts.first() else {
            continue;
        };
        if first == 0 {
            continue;
        }
        let consistent = counts.iter().filter(|&&count| count == first).count();
        let score = first * consistent;
        if score > best.1 {
            best = (candidate, score);
        }
    }
    best.0
}

fn count_outside_quotes(line: &str, delimiter: char) -> usize {
    let mut quoted = false;
    let mut count = 0;
    for ch in line.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch == delimiter && !quoted {
            count += 1;
        }
    }
    count
}

/// Разбор CSV по RFC 4180: кавычки, удвоенные кавычки, переводы строк в кавычках.
struct DelimitedParser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    delimiter: char,
    unterminated_quote: bool,
    done: bool,
}

impl<'a> DelimitedParser<'a> {
    fn new(text: &'a str, delimiter: char) -> Self {
        Self {
            chars: text.chars().peekable(),
            delimiter,
            unterminated_quote: false,
            done: false,
        }
    }

    fn next_row(&mut self) -> Option<Vec<String>> {
        if self.done {
            return None;
        }
        let mut row = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        let mut started = false;
        loop {
            let Some(ch) = self.chars.next() else {
                self.done = true;
                if quoted {
                    self.unterminated_quote = true;
                }
                if !started && row.is_empty() {
                    return None;
                }
                row.push(field);
                return Some(row);
            };
            started = true;
            if quoted {
                if ch == '"' {
                    if self.chars.peek() == Some(&'"') {
                        self.chars.next();
                        field.push('"');
                    } else {
                        quoted = false;
                    }
                } else {
                    field.push(ch);
                }
                continue;
            }
            match ch {
                '"' if field.is_empty() => quoted = true,
                '\n' => {
                    row.push(field);
                    return Some(row);
                }
                ch if ch == self.delimiter => row.push(std::mem::take(&mut field)),
                ch => field.push(ch),
            }
        }
    }
}

/// Jupyter Notebook: текст markdown-ячеек и код. Результаты выполнения не извлекаются.
pub fn extract_notebook(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let mut collector = Collector::new(limits);
    let mut metadata = ExtractionMetadata::default();
    let text = prepare(bytes, &mut collector, &mut metadata)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| ExtractError::Corrupt("Ноутбук не является корректным JSON".into()))?;
    let cells = value
        .get("cells")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ExtractError::Corrupt("В ноутбуке нет списка ячеек".into()))?;
    let language = value
        .pointer("/metadata/language_info/name")
        .or_else(|| value.pointer("/metadata/kernelspec/language"))
        .and_then(serde_json::Value::as_str)
        .map(|name| name.chars().take(32).collect::<String>());
    metadata.language = language.clone();
    let mut outputs = 0_usize;
    for (index, cell) in cells.iter().enumerate() {
        if index.is_multiple_of(64) {
            budget.check()?;
        }
        if collector.is_full() {
            break;
        }
        let source = match cell.get("source") {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(serde_json::Value::Array(lines)) => lines
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<String>(),
            _ => String::new(),
        };
        let source = clean_text(&source);
        if cell
            .get("outputs")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| !items.is_empty())
        {
            outputs += 1;
        }
        if source.trim().is_empty() {
            continue;
        }
        match cell.get("cell_type").and_then(serde_json::Value::as_str) {
            Some("markdown") => markdown_blocks(&source, &mut collector, budget)?,
            Some("code") => collector.push(Block::Code {
                language: language.clone(),
                text: source,
            }),
            _ => collector.push(Block::Paragraph { text: source }),
        }
    }
    if outputs > 0 {
        collector.warn(
            WarningCode::OutputsSkipped,
            format!("Результаты выполнения {outputs} ячеек не извлекаются — только исходный код и текст"),
            None,
        );
    }
    finish_text(collector, SourceFormat::Notebook, metadata)
}

fn finish_text(
    collector: Collector,
    format: SourceFormat,
    metadata: ExtractionMetadata,
) -> ExtractResult<Extracted> {
    let mut extracted = collector.finish(format, metadata);
    if extracted.text.is_empty() {
        extracted.warnings.push(super::model::ExtractionWarning {
            code: WarningCode::EmptyDocument,
            message: "Файл не содержит текста".into(),
            location: None,
        });
    }
    Ok(extracted)
}

pub fn word_count(text: &str) -> u64 {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count() as u64
}
