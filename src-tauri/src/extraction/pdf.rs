//! PDF: текстовый слой по страницам, количество страниц, metadata и
//! распознавание «скана» (страницы только с изображениями, без текста).
//!
//! Используется `lopdf` без JavaScript, без форм и без внешних ссылок:
//! содержимое только читается. Распаковка каждого потока ограничена
//! [`ExtractionLimits::max_pdf_stream_bytes`] (decompression bomb).

use lopdf::{Document, LoadOptions, Object};

use super::model::{
    clean_text, compact_ranges, Block, Budget, Collector, ExtractError, ExtractResult, Extracted,
    ExtractionLimits, ExtractionMetadata, ExtractionStatus, SourceFormat, WarningCode,
};

/// Сколько непробельных символов страницы считается «текстовым слоем».
const MIN_PAGE_CHARS: usize = 3;

pub fn extract(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let options = LoadOptions {
        max_decompressed_size: Some(limits.max_pdf_stream_bytes),
        ..LoadOptions::default()
    };
    let document = match Document::load_mem_with_options(bytes, options) {
        Ok(document) => document,
        Err(
            lopdf::Error::InvalidPassword
            | lopdf::Error::Decryption(_)
            | lopdf::Error::UnsupportedSecurityHandler(_),
        ) => {
            return Err(ExtractError::Encrypted(
                "PDF защищён паролем — текст не извлекается".into(),
            ))
        }
        Err(lopdf::Error::Decompress(_)) => {
            return Err(ExtractError::Rejected(
                "Поток PDF распаковывается в слишком большой объём (возможна bomb-атака)".into(),
            ))
        }
        Err(_) => {
            return Err(ExtractError::Corrupt(
                "PDF повреждён или имеет неподдерживаемую структуру".into(),
            ))
        }
    };
    budget.check()?;
    if document.is_encrypted() {
        return Err(ExtractError::Encrypted(
            "PDF защищён паролем — текст не извлекается".into(),
        ));
    }

    let mut metadata = read_metadata(&document);
    let pages = document.get_pages();
    metadata.page_count = Some(pages.len().min(u32::MAX as usize) as u32);
    let mut collector = Collector::new(limits);

    let mut text_pages = 0_u32;
    let mut scanned_pages: Vec<u32> = Vec::new();
    let mut unreadable: Vec<u32> = Vec::new();
    for (processed, (&number, &page_id)) in pages.iter().enumerate() {
        budget.check()?;
        if processed >= limits.max_pdf_pages as usize {
            collector.warn(
                WarningCode::PagesTruncated,
                format!(
                    "Извлечены первые {} страниц из {}",
                    limits.max_pdf_pages,
                    pages.len()
                ),
                None,
            );
            break;
        }
        if collector.is_full() {
            break;
        }
        let text = match document.extract_text_with_limit(&[number], limits.max_pdf_stream_bytes) {
            Ok(text) => clean_text(&text),
            Err(_) => {
                unreadable.push(number);
                String::new()
            }
        };
        let text = tidy_page(&text);
        if text.chars().filter(|ch| !ch.is_whitespace()).count() >= MIN_PAGE_CHARS {
            text_pages += 1;
            collector.push(Block::Page { number, text });
        } else if page_has_images(&document, page_id) {
            scanned_pages.push(number);
        }
    }
    budget.check()?;

    if !unreadable.is_empty() {
        collector.warn(
            WarningCode::PageUnreadable,
            format!(
                "Не удалось прочитать текст страниц: {}",
                compact_ranges(&unreadable)
            ),
            None,
        );
    }
    if text_pages > 0 && !scanned_pages.is_empty() {
        collector.warn(
            WarningCode::PageWithoutText,
            format!(
                "Страницы без текстового слоя (похожи на сканы): {}",
                compact_ranges(&scanned_pages)
            ),
            None,
        );
    }
    let mut extracted = collector.finish(SourceFormat::Pdf, metadata);
    if text_pages == 0 && !scanned_pages.is_empty() {
        extracted.status = ExtractionStatus::NeedsOcr;
        extracted.status_message = Some(
            "PDF похож на скан: текстового слоя нет. Распознавание PDF-сканов локально пока не поддерживается — изображения страниц можно распознать отдельно".into(),
        );
    } else if text_pages == 0 && unreadable.is_empty() {
        extracted.warnings.push(super::model::ExtractionWarning {
            code: WarningCode::EmptyDocument,
            message: "В PDF нет текста".into(),
            location: None,
        });
    }
    Ok(extracted)
}

/// Убирает пустые строки в начале/конце и лишние пробелы в строках.
fn tidy_page(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn page_has_images(document: &Document, page_id: lopdf::ObjectId) -> bool {
    document
        .get_page_images(page_id)
        .map(|images| !images.is_empty())
        .unwrap_or(false)
}

fn read_metadata(document: &Document) -> ExtractionMetadata {
    let mut metadata = ExtractionMetadata {
        pdf_version: Some(document.version.chars().take(8).collect()),
        ..ExtractionMetadata::default()
    };
    let info = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|object| match object {
            Object::Reference(id) => document.get_dictionary(*id).ok(),
            Object::Dictionary(dictionary) => Some(dictionary),
            _ => None,
        });
    let Some(info) = info else {
        return metadata;
    };
    let field = |key: &[u8]| -> Option<String> {
        let object = info.get(key).ok()?;
        let object = match object {
            Object::Reference(id) => document.get_object(*id).ok()?,
            other => other,
        };
        let text = lopdf::decode_text_string(object).ok()?;
        let text = clean_text(&text)
            .trim()
            .chars()
            .take(500)
            .collect::<String>();
        (!text.is_empty()).then_some(text)
    };
    metadata.title = field(b"Title");
    metadata.author = field(b"Author");
    metadata.subject = field(b"Subject");
    metadata.application = field(b"Creator");
    metadata.producer = field(b"Producer");
    metadata.created = field(b"CreationDate").map(|value| pdf_date(&value));
    metadata.modified = field(b"ModDate").map(|value| pdf_date(&value));
    metadata
}

/// `D:20261021093000+03'00'` → `2026-10-21T09:30:00+03:00`; иначе как есть.
fn pdf_date(value: &str) -> String {
    let raw = value.strip_prefix("D:").unwrap_or(value);
    let digits: String = raw.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 8 {
        return value.to_owned();
    }
    let part = |range: std::ops::Range<usize>, default: &str| -> String {
        digits.get(range).unwrap_or(default).to_owned()
    };
    let mut out = format!(
        "{}-{}-{}",
        part(0..4, ""),
        part(4..6, "01"),
        part(6..8, "01")
    );
    if digits.len() >= 12 {
        out.push_str(&format!(
            "T{}:{}:{}",
            part(8..10, "00"),
            part(10..12, "00"),
            part(12..14, "00")
        ));
        let rest = &raw[digits.len()..];
        if rest.starts_with('Z') {
            out.push('Z');
        } else if let Some(sign) = rest.chars().next().filter(|ch| *ch == '+' || *ch == '-') {
            let offset: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            if offset.len() >= 4 {
                out.push_str(&format!("{sign}{}:{}", &offset[0..2], &offset[2..4]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::pdf_date;

    #[test]
    fn pdf_dates_are_normalized() {
        assert_eq!(
            pdf_date("D:20261021093000+03'00'"),
            "2026-10-21T09:30:00+03:00"
        );
        assert_eq!(pdf_date("D:20261021"), "2026-10-21");
        assert_eq!(pdf_date("D:20261021093000Z"), "2026-10-21T09:30:00Z");
        assert_eq!(pdf_date("вчера"), "вчера");
    }
}
