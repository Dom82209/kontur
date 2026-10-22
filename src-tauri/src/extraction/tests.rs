//! Тесты этапа 7. Все файлы создаются только во временных каталогах (`TempDir`).

use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use uuid::Uuid;

use super::{
    dto::{BatchItemOutcome, MaterialKind, MaterialRef},
    engine::{extract_bytes, options_key, run_isolated, OcrKey},
    fixtures::*,
    image::OcrMode,
    model::{
        Block, Budget, ExtractError, Extracted, ExtractionLimits, ExtractionStatus, Interrupt,
        SourceFormat, WarningCode,
    },
    ocr::{OcrEngine, OcrError},
    ooxml,
    service::ExtractionService,
    store, text,
};
use crate::{
    db::Database,
    error::ErrorCode,
    intake::{RootKind, RootsService},
    lms::zip_safe::test_zip::{build, RawEntry},
};

fn run(name: &str, bytes: &[u8]) -> Extracted {
    run_with(name, bytes, &ExtractionLimits::default())
}

fn run_with(name: &str, bytes: &[u8], limits: &ExtractionLimits) -> Extracted {
    extract_bytes(name, bytes, limits, &Budget::unlimited(), OcrMode::Disabled).unwrap()
}

fn has_warning(extracted: &Extracted, code: WarningCode) -> bool {
    extracted
        .warnings
        .iter()
        .any(|warning| warning.code == code)
}

// ───────────────────────── PDF ─────────────────────────

#[test]
fn text_pdf_yields_pages_metadata_and_cyrillic_text() {
    let extracted = run("Лекция.pdf", LECTURE_PDF);
    assert_eq!(extracted.format, SourceFormat::Pdf);
    assert_eq!(
        extracted.status,
        ExtractionStatus::Complete,
        "{:?}",
        extracted.warnings
    );
    assert_eq!(extracted.metadata.page_count, Some(3));
    assert_eq!(
        extracted.metadata.title.as_deref(),
        Some("Лекция: описательная статистика")
    );
    assert_eq!(
        extracted.metadata.author.as_deref(),
        Some("Контур fixtures")
    );
    let pages: Vec<u32> = extracted
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Page { number, .. } => Some(*number),
            _ => None,
        })
        .collect();
    assert_eq!(pages, vec![1, 2, 3]);
    assert!(extracted.text.contains("Среднее значение и медиана"));
    assert!(extracted.text.contains("--- Страница 2 ---"));
    assert_eq!(extracted.text_chars, extracted.text.chars().count());
}

#[test]
fn scanned_pdf_is_reported_as_needing_ocr_without_failing() {
    let extracted = run("скан.pdf", SCANNED_PDF);
    assert_eq!(extracted.status, ExtractionStatus::NeedsOcr);
    assert_eq!(extracted.metadata.page_count, Some(2));
    assert!(extracted.text.is_empty());
    assert!(extracted
        .status_message
        .as_deref()
        .unwrap()
        .contains("скан"));
}

#[test]
fn password_protected_pdf_is_encrypted_status() {
    let extracted = run("закрыто.pdf", ENCRYPTED_PDF);
    assert_eq!(extracted.status, ExtractionStatus::Encrypted);
    assert!(extracted.text.is_empty());
}

#[test]
fn corrupt_and_truncated_pdfs_do_not_panic() {
    let garbage = run("broken.pdf", b"%PDF-1.7\n this is not a pdf at all");
    assert_eq!(garbage.status, ExtractionStatus::Corrupt);
    let half = &LECTURE_PDF[..LECTURE_PDF.len() / 2];
    let truncated = run("half.pdf", half);
    assert!(matches!(
        truncated.status,
        ExtractionStatus::Corrupt | ExtractionStatus::Partial | ExtractionStatus::Complete
    ));
    let fake = run("fake.pdf", b"just text with a pdf extension");
    assert_eq!(fake.status, ExtractionStatus::Unsupported);
}

#[test]
fn pdf_page_limit_marks_result_partial() {
    let limits = ExtractionLimits {
        max_pdf_pages: 1,
        ..ExtractionLimits::default()
    };
    let extracted = run_with("Лекция.pdf", LECTURE_PDF, &limits);
    assert_eq!(extracted.status, ExtractionStatus::Partial);
    assert!(has_warning(&extracted, WarningCode::PagesTruncated));
    assert_eq!(
        extracted
            .blocks
            .iter()
            .filter(|block| matches!(block, Block::Page { .. }))
            .count(),
        1
    );
}

#[test]
fn handmade_pdf_text_is_extracted() {
    let pdf = simple_pdf(&["First page text", "Second page text"]);
    let extracted = run("simple.pdf", &pdf);
    assert_eq!(extracted.metadata.page_count, Some(2));
    assert!(extracted.text.contains("First page text"));
    assert!(extracted.text.contains("Second page text"));
}

// ───────────────────────── DOCX ─────────────────────────

#[test]
fn docx_headings_lists_tables_and_metadata() {
    let extracted = run("Семинар.docx", SEMINAR_DOCX);
    assert_eq!(extracted.format, SourceFormat::Docx);
    assert_eq!(
        extracted.status,
        ExtractionStatus::Complete,
        "{:?}",
        extracted.warnings
    );
    assert_eq!(extracted.metadata.title.as_deref(), Some("Семинар 4"));
    assert!(extracted.blocks.contains(&Block::Heading {
        level: 1,
        text: "Семинар 4. Регрессия".into()
    }));
    assert!(extracted.blocks.contains(&Block::Heading {
        level: 2,
        text: "Задачи".into()
    }));
    assert!(extracted.blocks.iter().any(|block| matches!(
        block,
        Block::ListItem { ordered: false, text, .. } if text == "Загрузить данные"
    )));
    assert!(extracted.blocks.iter().any(|block| matches!(
        block,
        Block::ListItem { ordered: true, text, .. } if text == "Оценить коэффициенты"
    )));
    let table = extracted
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::Table { rows, .. } => Some(rows.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(table[0], vec!["Месяц", "Продажи"]);
    assert_eq!(table.len(), 3);
    assert!(extracted.text.contains("# Семинар 4. Регрессия"));
    assert!(extracted.text.contains("| Январь | 120 |"));
}

#[test]
fn docx_with_doctype_is_rejected_without_expanding_entities() {
    let xxe = format!(
        r#"<?xml version="1.0"?><!DOCTYPE w:document [<!ENTITY xxe SYSTEM "file:///etc/passwd">]><w:document {W}><w:body><w:p><w:r><w:t>&xxe;</w:t></w:r></w:p></w:body></w:document>"#
    );
    let root = root_rels("word/document.xml");
    let docx = package(&[
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", xxe.as_bytes()),
    ]);
    let extracted = run("xxe.docx", &docx);
    assert_eq!(extracted.status, ExtractionStatus::Rejected);
    assert!(extracted.text.is_empty());
    assert!(extracted.status_message.unwrap().contains("DOCTYPE"));
}

#[test]
fn billion_laughs_is_rejected_quickly() {
    let mut dtd = String::from(r#"<!DOCTYPE lolz [<!ENTITY lol "lol">"#);
    for level in 1..10 {
        let previous = if level == 1 {
            "lol".to_owned()
        } else {
            format!("lol{}", level - 1)
        };
        dtd.push_str(&format!(
            r#"<!ENTITY lol{level} "{}">"#,
            format!("&{previous};").repeat(10)
        ));
    }
    dtd.push_str("]>");
    let xml = format!(
        r#"<?xml version="1.0"?>{dtd}<w:document {W}><w:body><w:p><w:r><w:t>&lol9;</w:t></w:r></w:p></w:body></w:document>"#
    );
    let root = root_rels("word/document.xml");
    let docx = package(&[
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", xml.as_bytes()),
    ]);
    let started = Instant::now();
    let extracted = run("laughs.docx", &docx);
    assert_eq!(extracted.status, ExtractionStatus::Rejected);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn unknown_entities_without_dtd_are_not_expanded() {
    let body = "<w:p><w:r><w:t>До &secret; после &amp; &#x41;</w:t></w:r></w:p>";
    let extracted = run("entity.docx", &docx_with(body, None, &[]));
    assert!(
        extracted.text.contains("До  после & A"),
        "{}",
        extracted.text
    );
    assert!(!extracted.text.contains("secret"));
    assert!(has_warning(&extracted, WarningCode::UnknownEntity));
}

#[test]
fn relationship_traversal_and_external_targets_are_ignored() {
    let document_rels = rels(&[
        ("rId1", "styles", "../../../../etc/passwd"),
        ("rId2", "numbering", "/../../outside.xml"),
        ("rId3", "oleObject", "file:///Users/someone/secret.xlsx"),
        ("rId4", "hyperlink", "https://example.org"),
    ]);
    let body = paragraph("Обычный текст");
    let extracted = run("rels.docx", &docx_with(&body, Some(&document_rels), &[]));
    assert!(extracted.text.contains("Обычный текст"));
    assert!(has_warning(&extracted, WarningCode::ExternalLinksIgnored));
    assert!(!extracted.text.contains("passwd"));
}

#[test]
fn macros_and_embedded_objects_are_reported_not_executed() {
    let body = format!(
        "{}<w:p><w:r><w:object><w:t>ole preview</w:t></w:object></w:r></w:p>",
        paragraph("Документ с макросом")
    );
    let docm = docx_with(
        &body,
        None,
        &[
            (
                "word/vbaProject.bin",
                b"Attribute VB_Name = \"Module1\"\nShell \"rm -rf\"",
            ),
            (
                "word/embeddings/oleObject1.bin",
                b"\xD0\xCF\x11\xE0 embedded",
            ),
        ],
    );
    let extracted = run("macro.docm", &docm);
    assert!(extracted.text.contains("Документ с макросом"));
    assert!(!extracted.text.contains("Shell"));
    assert!(!extracted.text.contains("ole preview"));
    assert!(has_warning(&extracted, WarningCode::MacrosIgnored));
    assert!(has_warning(&extracted, WarningCode::EmbeddedObjectsSkipped));
}

#[test]
fn alternate_content_hidden_and_deleted_text_are_not_duplicated() {
    let body = r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><w:txbxContent><w:p><w:r><w:t>Надпись</w:t></w:r></w:p></w:txbxContent></w:drawing></mc:Choice><mc:Fallback><w:pict><w:txbxContent><w:p><w:r><w:t>Надпись</w:t></w:r></w:p></w:txbxContent></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>
        <w:p><w:r><w:rPr><w:vanish/></w:rPr><w:t>скрыто</w:t></w:r><w:r><w:t>видно</w:t></w:r></w:p>
        <w:p><w:del><w:r><w:delText>удалено</w:delText></w:r></w:del><w:ins><w:r><w:t>вставлено</w:t></w:r></w:ins></w:p>"#;
    let extracted = run("alt.docx", &docx_with(body, None, &[]));
    assert_eq!(
        extracted.text.matches("Надпись").count(),
        1,
        "{}",
        extracted.text
    );
    assert!(extracted.text.contains("видно"));
    assert!(!extracted.text.contains("скрыто"));
    assert!(!extracted.text.contains("удалено"));
    assert!(extracted.text.contains("вставлено"));
    assert!(has_warning(&extracted, WarningCode::HiddenContent));
    assert!(has_warning(&extracted, WarningCode::TrackedChangesSkipped));
}

#[test]
fn nested_tables_flatten_into_parent_cell() {
    let body = "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>x</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>y</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl>";
    let extracted = run("nested.docx", &docx_with(body, None, &[]));
    let tables: Vec<&Block> = extracted
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Table { .. }))
        .collect();
    assert_eq!(tables.len(), 1);
    assert_eq!(
        tables[0],
        &Block::Table {
            rows: vec![vec!["A\nx | y".into(), "B".into()]],
            truncated: false
        }
    );
}

#[test]
fn malicious_office_containers_are_rejected() {
    // zip-бомба: сильно сжимаемая часть больше 1 МБ.
    let huge = format!("<row/>{}", "<c/>".repeat(1_500_000));
    let bomb = xlsx_with(&huge, &[]);
    let result = run("bomb.xlsx", &bomb);
    assert_eq!(
        result.status,
        ExtractionStatus::Rejected,
        "{:?}",
        result.status_message
    );

    // symlink внутри пакета.
    let root = root_rels("word/document.xml");
    let symlink = build(
        &[
            RawEntry::file("[Content_Types].xml", b"<Types/>"),
            RawEntry::file("_rels/.rels", root.as_bytes()),
            RawEntry::symlink("word/document.xml", "/etc/passwd"),
        ],
        false,
    );
    let result = run("link.docx", &symlink);
    assert_eq!(
        result.status,
        ExtractionStatus::Rejected,
        "{:?}",
        result.status_message
    );

    // ZIP Slip в имени части.
    let slip = build(
        &[
            RawEntry::file("[Content_Types].xml", b"<Types/>"),
            RawEntry::file("word/document.xml", b"<w:document/>"),
            RawEntry::file("../../evil.xml", b"x"),
        ],
        false,
    );
    let result = run("slip.docx", &slip);
    assert_eq!(
        result.status,
        ExtractionStatus::Rejected,
        "{:?}",
        result.status_message
    );

    // Совпадающие пути без учёта регистра.
    let duplicate = build(
        &[
            RawEntry::file("[Content_Types].xml", b"<Types/>"),
            RawEntry::file("word/document.xml", b"<w:document/>"),
            RawEntry::file("WORD/Document.xml", b"<w:document/>"),
        ],
        false,
    );
    let result = run("dup.docx", &duplicate);
    assert_eq!(
        result.status,
        ExtractionStatus::Rejected,
        "{:?}",
        result.status_message
    );

    // Перекрывающиеся данные записей (overlapping bomb).
    let overlap = build(
        &[
            RawEntry::file("[Content_Types].xml", b"<Types/>"),
            RawEntry::file("word/document.xml", b"<w:document/>"),
        ],
        true,
    );
    let result = run("overlap.docx", &overlap);
    assert!(
        matches!(
            result.status,
            ExtractionStatus::Rejected | ExtractionStatus::Corrupt
        ),
        "{:?}",
        result.status_message
    );
    assert!(result.text.is_empty());
}

#[test]
fn oversized_office_part_and_truncated_package() {
    let limits = ExtractionLimits {
        max_office_part_bytes: 512,
        ..ExtractionLimits::default()
    };
    let body = paragraph(&"Длинный абзац. ".repeat(100));
    let extracted = run_with("big.docx", &docx_with(&body, None, &[]), &limits);
    assert_eq!(
        extracted.status,
        ExtractionStatus::Rejected,
        "{:?}",
        extracted.status_message
    );

    let truncated = &SEMINAR_DOCX[..SEMINAR_DOCX.len() / 2];
    let extracted = run("half.docx", truncated);
    assert_eq!(extracted.status, ExtractionStatus::Corrupt);
}

// ───────────────────────── XLSX ─────────────────────────

#[test]
fn xlsx_sheets_ranges_values_formulas_and_dates() {
    let extracted = run("Данные.xlsx", DATASET_XLSX);
    assert_eq!(extracted.format, SourceFormat::Xlsx);
    assert_eq!(
        extracted.status,
        ExtractionStatus::Complete,
        "{:?}",
        extracted.warnings
    );
    assert_eq!(extracted.metadata.sheet_count, Some(3));
    let sheets: Vec<&Block> = extracted
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Sheet { .. }))
        .collect();
    assert_eq!(sheets.len(), 3);
    let Block::Sheet {
        name,
        used_range,
        cells,
        formula_count,
        hidden,
        ..
    } = sheets[0]
    else {
        unreachable!()
    };
    assert_eq!(name, "Продажи");
    assert_eq!(used_range.as_deref(), Some("A1:C4"));
    assert!(!hidden);
    assert_eq!(*formula_count, 1);
    let cell = |reference: &str| {
        cells
            .iter()
            .find(|cell| cell.reference == reference)
            .unwrap()
    };
    assert_eq!(cell("A2").value, "Январь");
    assert_eq!(cell("B2").value, "120");
    assert_eq!(cell("C2").value, "2026-01-31");
    assert_eq!(cell("B4").formula.as_deref(), Some("=SUM(B2:B3)"));
    assert!(matches!(sheets[2], Block::Sheet { hidden: true, .. }));
    assert!(has_warning(&extracted, WarningCode::FormulasNotEvaluated));
    assert!(extracted.text.contains("## Лист «Продажи»"));
    assert!(extracted
        .text
        .contains("A2: Январь | B2: 120 | C2: 2026-01-31"));
    assert!(extracted.text.contains("[=SUM(B2:B3)]"));
}

#[test]
fn xlsx_shared_inline_bool_error_and_shared_formulas() {
    let sheet = r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Встроенная строка</t></is></c><c r="B1" t="b"><v>1</v></c><c r="C1" t="e"><v>#DIV/0!</v></c></row>
        <row r="2"><c r="A2"><f t="shared" ref="A2:A3" si="0">B2*2</f><v>4</v></c></row>
        <row r="3"><c r="A3"><f t="shared" si="0"/><v>6</v></c></row>"#;
    let extracted = run("cells.xlsx", &xlsx_with(sheet, &[]));
    assert!(
        extracted.text.contains("A1: Встроенная строка"),
        "{}",
        extracted.text
    );
    assert!(extracted.text.contains("B1: TRUE"));
    assert!(extracted.text.contains("C1: #DIV/0!"));
    assert!(extracted.text.contains("A2: 4 [=B2*2]"));
    assert!(extracted.text.contains("общая формула из A2"));
}

#[test]
fn xlsx_cell_limit_truncates_sheet() {
    let limits = ExtractionLimits {
        max_cells_per_sheet: 2,
        ..ExtractionLimits::default()
    };
    let sheet =
        r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><v>2</v></c><c r="C1"><v>3</v></c></row>"#;
    let extracted = run_with("limit.xlsx", &xlsx_with(sheet, &[]), &limits);
    assert_eq!(extracted.status, ExtractionStatus::Partial);
    assert!(has_warning(&extracted, WarningCode::CellsTruncated));
    assert!(!extracted.text.contains("C1"));
}

#[test]
fn xlsx_macros_and_external_links_are_never_followed() {
    let workbook = r#"<workbook xmlns:r="r"><sheets><sheet name="Лист1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let workbook_rels = rels(&[
        ("rId1", "worksheet", "worksheets/sheet1.xml"),
        ("rId2", "externalLink", "externalLinks/externalLink1.xml"),
    ]);
    let sheet = r#"<worksheet><sheetData><row r="1"><c r="A1"><f>[1]Sheet1!A1</f><v>7</v></c></row></sheetData></worksheet>"#;
    let root = root_rels("xl/workbook.xml");
    let xlsm = package(&[
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/vbaProject.bin", b"Sub AutoOpen()"),
    ]);
    let extracted = run("macro.xlsm", &xlsm);
    assert!(has_warning(&extracted, WarningCode::MacrosIgnored));
    assert!(has_warning(&extracted, WarningCode::ExternalLinksIgnored));
    assert!(extracted.text.contains("A1: 7 [=[1]Sheet1!A1]"));
    assert!(!extracted.text.contains("AutoOpen"));
}

// ───────────────────────── PPTX ─────────────────────────

#[test]
fn pptx_titles_text_notes_and_tables() {
    let extracted = run("Неделя 5.pptx", SLIDES_PPTX);
    assert_eq!(extracted.format, SourceFormat::Pptx);
    assert_eq!(extracted.metadata.slide_count, Some(3));
    let slides: Vec<&Block> = extracted
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Slide { .. }))
        .collect();
    assert_eq!(slides.len(), 3);
    let Block::Slide {
        number,
        title,
        body,
        notes,
        ..
    } = slides[1]
    else {
        unreachable!()
    };
    assert_eq!(*number, 2);
    assert_eq!(title.as_deref(), Some("Метрики удержания"));
    assert!(body.contains(&"Retention первого дня".to_owned()));
    assert_eq!(notes.as_deref(), Some("Напомнить про определение когорты."));
    let Block::Slide { body, title, .. } = slides[2] else {
        unreachable!()
    };
    assert_eq!(title.as_deref(), Some("Таблица когорт"));
    assert!(body.contains(&"| Когорта | D7 |".to_owned()), "{body:?}");
    assert!(extracted.text.contains("--- Слайд 1 ---"));
    assert!(extracted.text.contains("# Когортный анализ"));
    assert!(extracted.text.contains("Заметки докладчика: Напомнить"));
}

#[test]
fn pptx_slide_limit_is_reported() {
    let limits = ExtractionLimits {
        max_slides: 1,
        ..ExtractionLimits::default()
    };
    let extracted = run_with("slides.pptx", SLIDES_PPTX, &limits);
    assert_eq!(extracted.status, ExtractionStatus::Partial);
    assert!(has_warning(&extracted, WarningCode::SlidesTruncated));
}

// ───────────────────────── Текстовые форматы ─────────────────────────

#[test]
fn plain_text_utf8_utf16_and_binary() {
    let extracted = run("notes.txt", "Первый абзац\r\n\r\nВторой абзац".as_bytes());
    assert_eq!(extracted.status, ExtractionStatus::Complete);
    assert_eq!(extracted.text, "Первый абзац\n\nВторой абзац");
    assert_eq!(extracted.metadata.word_count, Some(4));

    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "Привет".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let extracted = run("utf16.txt", &utf16);
    assert_eq!(extracted.text, "Привет");
    assert_eq!(extracted.metadata.encoding.as_deref(), Some("UTF-16LE"));

    let binary = run("data.txt", b"\x00\x01\x02binary\x00");
    assert_eq!(binary.status, ExtractionStatus::Unsupported);
}

#[test]
fn plain_text_decodes_utf16_little_and_big_endian() {
    for (bom, little, label) in [
        (&b"\xFF\xFE"[..], true, "UTF-16LE"),
        (&b"\xFE\xFF"[..], false, "UTF-16BE"),
    ] {
        let mut bytes = bom.to_vec();
        for unit in "Привет".encode_utf16() {
            let encoded = if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            bytes.extend_from_slice(&encoded);
        }
        let decoded = text::decode(&bytes).unwrap();
        assert_eq!(decoded.text, "Привет");
        assert_eq!(decoded.encoding, label);
    }
}

#[test]
fn plain_text_rejects_incomplete_utf16_code_unit() {
    let error = match text::decode(b"\xFF\xFEA") {
        Err(error) => error,
        Ok(_) => panic!("неполная UTF-16 кодовая единица должна быть отклонена"),
    };
    assert!(matches!(error, ExtractError::Corrupt(_)));
}

#[test]
fn ooxml_decodes_utf16_little_and_big_endian() {
    for (bom, little) in [(&b"\xFF\xFE"[..], true), (&b"\xFE\xFF"[..], false)] {
        let mut bytes = bom.to_vec();
        for unit in "<root>Текст</root>".encode_utf16() {
            let encoded = if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            };
            bytes.extend_from_slice(&encoded);
        }
        assert_eq!(ooxml::xml_string(bytes).unwrap(), "<root>Текст</root>");
    }
}

#[test]
fn ooxml_rejects_incomplete_utf16_code_unit() {
    let error = ooxml::xml_string(b"\xFE\xFFX".to_vec()).unwrap_err();
    assert!(matches!(error, ExtractError::Corrupt(_)));
}

#[test]
fn csv_in_windows_1251_with_semicolons_and_quotes() {
    let mut csv: Vec<u8> = Vec::new();
    // «Месяц;Продажи» в Windows-1251.
    csv.extend_from_slice(&[
        0xCC, 0xE5, 0xF1, 0xFF, 0xF6, b';', 0xCF, 0xF0, 0xEE, 0xE4, 0xE0, 0xE6, 0xE8,
    ]);
    csv.extend_from_slice(b"\r\n\"Jan; early\";120\r\n\"Feb \"\"x\"\"\nline\";135\r\n");
    let extracted = run("sales.csv", &csv);
    assert_eq!(extracted.format, SourceFormat::Csv);
    assert_eq!(
        extracted.status,
        ExtractionStatus::Complete,
        "{:?}",
        extracted.warnings
    );
    assert!(has_warning(&extracted, WarningCode::EncodingFallback));
    assert_eq!(extracted.metadata.delimiter.as_deref(), Some(";"));
    assert_eq!(extracted.metadata.row_count, Some(3));
    assert_eq!(extracted.metadata.column_count, Some(2));
    let Some(Block::Table { rows, .. }) = extracted.blocks.first() else {
        panic!("table expected")
    };
    assert_eq!(rows[0], vec!["Месяц", "Продажи"]);
    assert_eq!(rows[1], vec!["Jan; early", "120"]);
    assert_eq!(rows[2], vec!["Feb \"x\"\nline", "135"]);
}

#[test]
fn tsv_and_markdown_structure() {
    let tsv = run("table.tsv", b"a\tb\n1\t2\n");
    assert_eq!(tsv.metadata.delimiter.as_deref(), Some("tab"));
    assert_eq!(tsv.metadata.row_count, Some(2));

    let markdown = "# Конспект\n\nВведение\nв тему.\n\n## Формулы\n\n- первое\n  - вложенное\n1. шаг\n\n```python\nprint('hi')\n```\n";
    let extracted = run("notes.md", markdown.as_bytes());
    assert_eq!(extracted.format, SourceFormat::Markdown);
    assert_eq!(extracted.metadata.title.as_deref(), Some("Конспект"));
    assert!(extracted.blocks.contains(&Block::Heading {
        level: 2,
        text: "Формулы".into()
    }));
    assert!(extracted.blocks.contains(&Block::ListItem {
        level: 1,
        ordered: false,
        text: "вложенное".into()
    }));
    assert!(extracted.blocks.contains(&Block::ListItem {
        level: 0,
        ordered: true,
        text: "шаг".into()
    }));
    assert!(extracted.blocks.contains(&Block::Code {
        language: Some("python".into()),
        text: "print('hi')".into()
    }));
    assert!(extracted.blocks.contains(&Block::Paragraph {
        text: "Введение\nв тему.".into()
    }));
}

#[test]
fn notebook_cells_without_outputs() {
    let notebook = r##"{"cells":[{"cell_type":"markdown","source":["# Задание 1\n","Посчитайте среднее"]},{"cell_type":"code","source":"import pandas as pd\ndf = pd.read_csv('data.csv')","outputs":[{"output_type":"stream","text":"secret output"}]}],"metadata":{"language_info":{"name":"python"}}}"##;
    let extracted = run("hw.ipynb", notebook.as_bytes());
    assert_eq!(extracted.format, SourceFormat::Notebook);
    assert_eq!(extracted.metadata.language.as_deref(), Some("python"));
    assert!(extracted.blocks.contains(&Block::Heading {
        level: 1,
        text: "Задание 1".into()
    }));
    assert!(extracted.text.contains("```python\nimport pandas as pd"));
    assert!(!extracted.text.contains("secret output"));
    assert!(has_warning(&extracted, WarningCode::OutputsSkipped));

    let broken = run("broken.ipynb", b"{not json");
    assert_eq!(broken.status, ExtractionStatus::Corrupt);
}

#[test]
fn text_limit_truncates_and_marks_partial() {
    let limits = ExtractionLimits {
        max_text_chars: 50,
        ..ExtractionLimits::default()
    };
    let text = "слово ".repeat(100);
    let extracted = run_with("long.txt", text.as_bytes(), &limits);
    assert_eq!(extracted.status, ExtractionStatus::Partial);
    assert!(extracted.truncated);
    assert!(extracted.text_chars <= 50);
    assert!(has_warning(&extracted, WarningCode::TextTruncated));
}

// ───────────────────────── Неподдерживаемые и подмены ─────────────────────────

#[test]
fn unsupported_legacy_encrypted_and_archive_files_get_clear_status() {
    let mut ole = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec();
    ole.extend_from_slice(&[0_u8; 512]);
    let legacy = run("old.doc", &ole);
    assert_eq!(legacy.status, ExtractionStatus::Unsupported);
    assert!(legacy.status_message.unwrap().contains(".docx"));

    let mut encrypted = ole.clone();
    for unit in "EncryptedPackage".encode_utf16() {
        encrypted.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(
        run("secret.docx", &encrypted).status,
        ExtractionStatus::Encrypted
    );

    let archive = build(&[RawEntry::file("readme.txt", b"hello")], false);
    assert_eq!(
        run("materials.zip", &archive).status,
        ExtractionStatus::Unsupported
    );

    let executable = run("tool.exe", b"MZ\x90\x00\x03\x00 binary");
    assert_eq!(executable.status, ExtractionStatus::Unsupported);
    assert_eq!(executable.format, SourceFormat::Unknown);
}

#[test]
fn content_wins_over_extension_with_a_warning() {
    let extracted = run("not-a-doc.docx", LECTURE_PDF);
    assert_eq!(extracted.format, SourceFormat::Pdf);
    assert!(has_warning(&extracted, WarningCode::ExtensionMismatch));
}

// ───────────────────────── Изображения и OCR ─────────────────────────

struct FakeOcr {
    text: Option<&'static str>,
    hang: bool,
    panic: bool,
}

impl OcrEngine for FakeOcr {
    fn name(&self) -> &str {
        "fake"
    }
    fn languages(&self) -> &str {
        "rus+eng"
    }
    fn recognize(&self, _image: &[u8], budget: &Budget) -> Result<String, OcrError> {
        if self.panic {
            panic!("ocr crashed");
        }
        if self.hang {
            // Имитирует зависший разборщик, который не проверяет бюджет.
            thread::sleep(Duration::from_secs(30));
        }
        budget.check().map_err(OcrError::Interrupted)?;
        self.text
            .map(str::to_owned)
            .ok_or_else(|| OcrError::Failed("Не распознано".into()))
    }
}

#[test]
fn image_metadata_without_ocr_and_with_optional_ocr() {
    let limits = ExtractionLimits::default();
    let budget = Budget::unlimited();
    let disabled = extract_bytes(
        "schema.png",
        DIAGRAM_PNG,
        &limits,
        &budget,
        OcrMode::Disabled,
    )
    .unwrap();
    assert_eq!(disabled.format, SourceFormat::Image);
    assert_eq!(disabled.status, ExtractionStatus::NeedsOcr);
    assert_eq!(disabled.metadata.image_format.as_deref(), Some("png"));
    assert_eq!(disabled.metadata.image_width, Some(64));
    assert_eq!(disabled.metadata.image_height, Some(32));
    assert!(has_warning(&disabled, WarningCode::OcrDisabled));

    let unavailable = extract_bytes(
        "schema.png",
        DIAGRAM_PNG,
        &limits,
        &budget,
        OcrMode::Unavailable,
    )
    .unwrap();
    assert_eq!(unavailable.status, ExtractionStatus::NeedsOcr);
    assert!(has_warning(&unavailable, WarningCode::OcrUnavailable));

    let engine = FakeOcr {
        text: Some("Распознанный текст\n\nВторой абзац"),
        hang: false,
        panic: false,
    };
    let recognized = extract_bytes(
        "schema.png",
        DIAGRAM_PNG,
        &limits,
        &budget,
        OcrMode::Enabled(&engine),
    )
    .unwrap();
    assert_eq!(recognized.status, ExtractionStatus::Complete);
    assert!(recognized.ocr_used);
    assert_eq!(recognized.text, "Распознанный текст\n\nВторой абзац");

    let failing = FakeOcr {
        text: None,
        hang: false,
        panic: false,
    };
    let failed = extract_bytes(
        "schema.png",
        DIAGRAM_PNG,
        &limits,
        &budget,
        OcrMode::Enabled(&failing),
    )
    .unwrap();
    assert_eq!(failed.status, ExtractionStatus::NeedsOcr);
    assert!(has_warning(&failed, WarningCode::OcrFailed));

    let broken = run("broken.png", b"\x89PNG\r\n\x1a\nnot really");
    assert!(matches!(
        broken.status,
        ExtractionStatus::Corrupt | ExtractionStatus::Unsupported
    ));
}

// ───────────────────────── Время, отмена, panic ─────────────────────────

#[test]
fn zero_time_budget_interrupts_cooperatively() {
    let limits = ExtractionLimits {
        time_limit: Duration::ZERO,
        ..ExtractionLimits::default()
    };
    let budget = Budget::new(
        Duration::ZERO,
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        extract_bytes(
            "Лекция.pdf",
            LECTURE_PDF,
            &limits,
            &budget,
            OcrMode::Disabled
        )
        .unwrap_err(),
        Interrupt::Timeout
    );
    let result = run_isolated(
        "Лекция.pdf",
        Arc::new(LECTURE_PDF.to_vec()),
        limits,
        Arc::new(AtomicBool::new(false)),
        None,
        false,
    )
    .unwrap();
    assert_eq!(result.status, ExtractionStatus::Timeout);
}

#[test]
fn hung_parser_is_abandoned_by_the_watchdog() {
    let limits = ExtractionLimits {
        time_limit: Duration::from_millis(100),
        ..ExtractionLimits::default()
    };
    let engine: Arc<dyn OcrEngine> = Arc::new(FakeOcr {
        text: Some("x"),
        hang: true,
        panic: false,
    });
    let started = Instant::now();
    let result = run_isolated(
        "scan.png",
        Arc::new(DIAGRAM_PNG.to_vec()),
        limits,
        Arc::new(AtomicBool::new(false)),
        Some(engine),
        true,
    )
    .unwrap();
    assert_eq!(result.status, ExtractionStatus::Timeout);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn cancellation_returns_promptly_even_if_parser_hangs() {
    let engine: Arc<dyn OcrEngine> = Arc::new(FakeOcr {
        text: Some("x"),
        hang: true,
        panic: false,
    });
    let cancel = Arc::new(AtomicBool::new(false));
    let trigger = Arc::clone(&cancel);
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        trigger.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let result = run_isolated(
        "scan.png",
        Arc::new(DIAGRAM_PNG.to_vec()),
        ExtractionLimits::default(),
        cancel,
        Some(engine),
        true,
    );
    canceller.join().unwrap();
    assert_eq!(result.unwrap_err(), Interrupt::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn parser_panic_becomes_corrupt_status() {
    let engine: Arc<dyn OcrEngine> = Arc::new(FakeOcr {
        text: None,
        hang: false,
        panic: true,
    });
    let result = run_isolated(
        "scan.png",
        Arc::new(DIAGRAM_PNG.to_vec()),
        ExtractionLimits::default(),
        Arc::new(AtomicBool::new(false)),
        Some(engine),
        true,
    )
    .unwrap();
    assert_eq!(result.status, ExtractionStatus::Corrupt);
}

#[test]
fn cache_keys_separate_text_formats_and_ocr_states() {
    assert_eq!(options_key("a.csv", OcrKey::Disabled), "csv");
    assert_eq!(options_key("a.md", OcrKey::Enabled), "markdown");
    assert_eq!(options_key("a.png", OcrKey::Disabled), "image");
    assert_eq!(options_key("a.png", OcrKey::Enabled), "image+ocr");
    assert_eq!(options_key("a.png", OcrKey::Unavailable), "image+no-ocr");
    assert_eq!(options_key("README", OcrKey::Enabled), "auto");
}

// ───────────────────────── Сервис на TempDir ─────────────────────────

struct Env {
    _tmp: TempDir,
    root: PathBuf,
    vault: PathBuf,
    database: Database,
}

fn env() -> Env {
    let tmp = TempDir::new().unwrap();
    let root = fs::canonicalize(tmp.path()).unwrap();
    let vault = root.join("vault");
    fs::create_dir_all(vault.join("Предметы/00 Входящие")).unwrap();
    fs::create_dir_all(vault.join("Предметы/ОБА/Неделя 05/01 Лекции")).unwrap();
    let database = Database::open_in_memory().unwrap();
    database.migrate().unwrap();
    RootsService::new(&database, None)
        .set_root(RootKind::Vault, &vault)
        .unwrap();
    database
        .with_connection(|connection| {
            connection.execute_batch(
                "INSERT INTO subjects(id, name, short_name, folder_path)
                     VALUES ('oba', 'Основы бизнес-аналитики', 'ОБА', 'Предметы/ОБА');
                 INSERT INTO weeks(id, subject_id, number, folder_path, starts_at, ends_at)
                     VALUES ('w5', 'oba', 5, 'Предметы/ОБА/Неделя 05',
                             '2026-10-05T00:00:00Z', '2026-10-11T23:59:59Z');",
            )?;
            Ok(())
        })
        .unwrap();
    Env {
        _tmp: tmp,
        root,
        vault,
        database,
    }
}

fn sha_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn add_inbox(env: &Env, name: &str, bytes: &[u8]) -> MaterialRef {
    let relative = format!("Предметы/00 Входящие/{name}");
    fs::write(env.vault.join(&relative), bytes).unwrap();
    let id = Uuid::new_v4().to_string();
    let kind = match name.rsplit('.').next().unwrap_or("") {
        "pdf" => "pdf",
        "docx" => "doc",
        "xlsx" | "csv" => "sheet",
        "pptx" => "slides",
        "png" => "image",
        "md" | "txt" => "text",
        _ => "other",
    };
    env.database
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO inbox_files(id, name, kind, size_bytes, added_at, origin, status, relative_path, sha256)
                 VALUES (?1, ?2, ?3, ?4, '2026-10-22T10:00:00Z', 'drop', 'new', ?5, ?6)",
                rusqlite::params![id, name, kind, bytes.len() as i64, relative, sha_hex(bytes)],
            )?;
            Ok(())
        })
        .unwrap();
    MaterialRef {
        kind: MaterialKind::Inbox,
        id,
    }
}

fn add_week_file(env: &Env, name: &str, bytes: &[u8]) -> MaterialRef {
    let relative = format!("Предметы/ОБА/Неделя 05/01 Лекции/{name}");
    fs::write(env.vault.join(&relative), bytes).unwrap();
    let id = Uuid::new_v4().to_string();
    env.database
        .with_connection(|connection| {
            connection.execute(
                "INSERT INTO files(id, week_id, category, name, kind, size_bytes, relative_path)
                 VALUES (?1, 'w5', 'lecture', ?2, 'pdf', ?3, ?4)",
                rusqlite::params![id, name, bytes.len() as i64, relative],
            )?;
            Ok(())
        })
        .unwrap();
    MaterialRef {
        kind: MaterialKind::File,
        id,
    }
}

fn service(env: &Env) -> ExtractionService<'_> {
    ExtractionService::new(&env.database, None, ExtractionLimits::default(), None)
}

fn no_cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn count(env: &Env, sql: &str) -> i64 {
    env.database
        .with_connection(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?))
        .unwrap()
}

#[test]
fn analyze_reads_original_without_changing_it_and_stores_normalized_result() {
    let env = env();
    let material = add_inbox(&env, "Лекция.pdf", LECTURE_PDF);
    let path = env.vault.join("Предметы/00 Входящие/Лекция.pdf");
    let before = fs::metadata(&path).unwrap();

    let result = service(&env)
        .analyze(&material, false, no_cancel())
        .unwrap();
    assert_eq!(result.status, ExtractionStatus::Complete);
    assert!(!result.cached);
    assert_eq!(result.sha256, sha_hex(LECTURE_PDF));
    assert_eq!(result.context, "Входящие");
    assert!(result.text.contains("Среднее значение"));

    let after = fs::metadata(&path).unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        LECTURE_PDF,
        "original is immutable"
    );
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(
        fs::read_dir(env.vault.join("Предметы/00 Входящие"))
            .unwrap()
            .count(),
        1,
        "no temp or derived files next to the original"
    );

    let stored = service(&env).get(&material).unwrap();
    assert_eq!(stored.text, result.text);
    assert_eq!(stored.blocks, result.blocks);
    let list = service(&env).list().unwrap();
    let summary = list[0].extraction.as_ref().unwrap();
    assert_eq!(summary.status, Some(ExtractionStatus::Complete));
    assert!(!summary.stale);
    let serialized = serde_json::to_string(&list).unwrap();
    assert!(
        !serialized.contains(env.root.to_str().unwrap()),
        "no absolute paths in DTO"
    );
}

#[test]
fn same_hash_uses_cache_and_force_rebuilds() {
    let env = env();
    let inbox = add_inbox(&env, "Лекция.pdf", LECTURE_PDF);
    let first = service(&env).analyze(&inbox, false, no_cancel()).unwrap();
    let second = service(&env).analyze(&inbox, false, no_cancel()).unwrap();
    assert!(second.cached);
    assert_eq!(second.extracted_at, first.extracted_at);

    // Тот же контент в файле недели: SHA-256 считается, разбор не повторяется.
    let week = add_week_file(&env, "Лекция 5.pdf", LECTURE_PDF);
    let reused = service(&env).analyze(&week, false, no_cancel()).unwrap();
    assert!(reused.cached);
    assert_eq!(reused.context, "ОБА · Неделя 5 · Лекции");

    let rebuilt = service(&env).analyze(&inbox, true, no_cancel()).unwrap();
    assert!(!rebuilt.cached);
    assert_eq!(count(&env, "SELECT COUNT(*) FROM extractions"), 1);
    assert_eq!(count(&env, "SELECT COUNT(*) FROM material_extractions"), 2);
}

#[test]
fn changed_or_missing_original_is_not_extracted() {
    let env = env();
    let material = add_inbox(&env, "notes.md", b"# v1");
    fs::write(
        env.vault.join("Предметы/00 Входящие/notes.md"),
        b"# changed outside",
    )
    .unwrap();
    let error = service(&env)
        .analyze(&material, false, no_cancel())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(count(&env, "SELECT COUNT(*) FROM extractions"), 0);

    let missing = add_inbox(&env, "gone.txt", b"text");
    fs::remove_file(env.vault.join("Предметы/00 Входящие/gone.txt")).unwrap();
    assert_eq!(
        service(&env)
            .analyze(&missing, false, no_cancel())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    let unknown = MaterialRef {
        kind: MaterialKind::Inbox,
        id: "missing".into(),
    };
    assert_eq!(
        service(&env)
            .analyze(&unknown, false, no_cancel())
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[test]
fn symlinked_original_is_blocked() {
    let env = env();
    let outside = env.root.join("outside.txt");
    fs::write(&outside, b"secret outside vault").unwrap();
    let material = add_inbox(&env, "link.txt", b"placeholder");
    let path = env.vault.join("Предметы/00 Входящие/link.txt");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&outside, &path).unwrap();
    let error = service(&env)
        .analyze(&material, true, no_cancel())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(count(&env, "SELECT COUNT(*) FROM extractions"), 0);
}

#[test]
fn oversized_file_is_stored_as_too_large_without_reading_into_memory() {
    let env = env();
    let material = add_inbox(&env, "big.txt", &vec![b'a'; 4096]);
    let limits = ExtractionLimits {
        max_input_bytes: 1024,
        ..ExtractionLimits::default()
    };
    let result = ExtractionService::new(&env.database, None, limits, None)
        .analyze(&material, false, no_cancel())
        .unwrap();
    assert_eq!(result.status, ExtractionStatus::TooLarge);
    assert_eq!(result.source_size_bytes, 4096);
}

#[test]
fn vault_must_be_configured() {
    let env = env();
    let material = add_inbox(&env, "a.txt", b"text");
    env.database
        .with_connection(|connection| {
            connection.execute("DELETE FROM app_settings WHERE key = 'managed_roots'", [])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        service(&env)
            .analyze(&material, true, no_cancel())
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn batch_reports_progress_failures_and_replays() {
    let env = env();
    let pdf = add_inbox(&env, "Лекция.pdf", LECTURE_PDF);
    let docx = add_inbox(&env, "Семинар.docx", SEMINAR_DOCX);
    let unsupported = add_inbox(&env, "tool.bin", b"\x7fELF binary");
    let missing = MaterialRef {
        kind: MaterialKind::Inbox,
        id: "missing".into(),
    };
    let operation_id = Uuid::new_v4().to_string();
    let mut progress = Vec::new();
    let materials = vec![pdf.clone(), docx.clone(), unsupported, missing, pdf.clone()];
    let outcome = service(&env)
        .run_batch(
            &operation_id,
            &materials,
            false,
            no_cancel(),
            &mut |event| progress.push((event.completed, event.total, event.outcome)),
        )
        .unwrap();
    assert_eq!(progress.first(), Some(&(0, 4, None)), "duplicates removed");
    assert_eq!(progress.len(), 5);
    assert_eq!(outcome.report.extracted, 3);
    assert_eq!(outcome.report.failed, 1);
    assert!(outcome.failure.is_none());
    assert_eq!(outcome.operation.kind, crate::dto::OperationKind::Extract);
    assert_eq!(
        outcome.operation.status,
        crate::domain::OperationStatus::Completed
    );
    let unsupported_item = &outcome.report.items[2];
    assert_eq!(unsupported_item.status, Some(ExtractionStatus::Unsupported));
    assert_eq!(
        outcome.report.items[3].error.as_ref().unwrap().code,
        ErrorCode::NotFound
    );

    let replay = service(&env)
        .run_batch(&operation_id, &materials, false, no_cancel(), &mut |_| {
            panic!("no progress on replay")
        })
        .unwrap();
    assert!(replay.report.replayed);
    assert_eq!(replay.report.extracted, 3);

    let again = service(&env)
        .run_batch(
            &Uuid::new_v4().to_string(),
            &[pdf, docx],
            false,
            no_cancel(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(again.report.cached, 2);

    let report_json: String = env
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT report_json FROM extraction_batches WHERE operation_id = ?1",
                [&operation_id],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert!(!report_json.contains(env.root.to_str().unwrap()));
}

#[test]
fn batch_cancellation_keeps_finished_results() {
    let env = env();
    let first = add_inbox(&env, "a.md", b"# A");
    let second = add_inbox(&env, "b.md", b"# B");
    let third = add_inbox(&env, "c.md", b"# C");
    let cancel = no_cancel();
    let trigger = Arc::clone(&cancel);
    let outcome = service(&env)
        .run_batch(
            &Uuid::new_v4().to_string(),
            &[first.clone(), second, third],
            false,
            cancel,
            &mut |event| {
                if event.completed == 1 {
                    trigger.store(true, Ordering::Relaxed);
                }
            },
        )
        .unwrap();
    assert!(outcome.report.stopped_by_user);
    assert_eq!(outcome.report.extracted, 1);
    assert_eq!(outcome.report.cancelled, 2);
    assert_eq!(outcome.failure.as_ref().unwrap().code, ErrorCode::Cancelled);
    assert_eq!(
        outcome.operation.status,
        crate::domain::OperationStatus::Failed
    );
    assert_eq!(outcome.report.items[1].outcome, BatchItemOutcome::Cancelled);
    assert!(
        service(&env).get(&first).is_ok(),
        "finished item stays cached"
    );
}

#[test]
fn batch_validates_request() {
    let env = env();
    assert_eq!(
        service(&env)
            .run_batch(
                &Uuid::new_v4().to_string(),
                &[],
                false,
                no_cancel(),
                &mut |_| {}
            )
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    assert!(service(&env)
        .run_batch("not-a-uuid", &[], false, no_cancel(), &mut |_| {})
        .is_err());
}

#[test]
fn corpus_selection_accepts_only_extracted_unchanged_materials() {
    let env = env();
    let pdf = add_inbox(&env, "Лекция.pdf", LECTURE_PDF);
    let md = add_inbox(&env, "notes.md", "# Конспект\n\nТекст".as_bytes());
    let image = add_inbox(&env, "schema.png", DIAGRAM_PNG);
    let service = service(&env);

    let error = service
        .set_selection(None, std::slice::from_ref(&pdf))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(error.message.contains("сначала извлеките текст"));

    service.analyze(&pdf, false, no_cancel()).unwrap();
    service.analyze(&md, false, no_cancel()).unwrap();
    service.analyze(&image, false, no_cancel()).unwrap();
    assert_eq!(
        service
            .set_selection(None, std::slice::from_ref(&image))
            .unwrap_err()
            .code,
        ErrorCode::Blocked,
        "image without OCR text cannot be selected"
    );

    let selection = service
        .set_selection(None, &[md.clone(), pdf.clone(), md.clone()])
        .unwrap();
    assert_eq!(selection.items.len(), 2);
    assert!(selection.items.iter().all(|item| item.ready));
    assert!(selection.total_chars > 0);

    let preview = service.preview_corpus(None).unwrap();
    assert_eq!(preview.documents.len(), 2);
    assert_eq!(
        preview.documents[0].name, "notes.md",
        "selection order is kept"
    );
    assert!(preview.documents[1].text.contains("Среднее значение"));
    assert!(preview.excluded.is_empty());

    // Содержимое материала изменилось (новая запись импорта) — выбор устаревает.
    env.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE inbox_files SET sha256 = ?2 WHERE id = ?1",
                rusqlite::params![md.id, "0".repeat(64)],
            )?;
            Ok(())
        })
        .unwrap();
    let selection = service.selection(None).unwrap();
    let stale = selection
        .items
        .iter()
        .find(|item| item.material == md)
        .unwrap();
    assert!(!stale.ready);
    assert!(stale.problem.as_deref().unwrap().contains("изменился"));
    let preview = service.preview_corpus(None).unwrap();
    assert_eq!(preview.documents.len(), 1);
    assert_eq!(preview.excluded.len(), 1);

    // Подборки ДЗ независимы от общей.
    assert!(service.selection(Some("missing")).is_err());
}

#[test]
fn submitted_homework_selection_is_immutable() {
    let env = env();
    let pdf = add_inbox(&env, "Лекция.pdf", LECTURE_PDF);
    service(&env).analyze(&pdf, false, no_cancel()).unwrap();
    env.database
        .with_connection(|connection| {
            connection.execute_batch(
                "INSERT INTO homeworks(id, week_id, title, deadline, stage, workflow_stage, folder_path)
                 VALUES ('hw5', 'w5', 'ДЗ 5', '2026-10-30T20:00:00Z', 'in_progress', 'materials', 'Предметы/ОБА/Неделя 05/03 ДЗ');",
            )?;
            Ok(())
        })
        .unwrap();
    let selection = service(&env)
        .set_selection(Some("hw5"), std::slice::from_ref(&pdf))
        .unwrap();
    assert_eq!(selection.homework_id.as_deref(), Some("hw5"));
    assert!(service(&env).selection(None).unwrap().items.is_empty());
    env.database
        .with_connection(|connection| {
            connection.execute(
                "UPDATE homeworks SET stage = 'submitted', workflow_stage = 'submitted', submitted_at = '2026-10-29T10:00:00Z' WHERE id = 'hw5'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        service(&env)
            .set_selection(Some("hw5"), &[])
            .unwrap_err()
            .code,
        ErrorCode::Immutable
    );
    // SQLite сам по себе тоже не даёт изменить подборку отправленной работы.
    let direct = env.database.with_connection(|connection| {
        connection.execute("DELETE FROM corpus_selection_items", [])?;
        Ok(())
    });
    assert!(direct.is_err());
}

#[test]
fn cache_can_be_cleared_and_rebuilt() {
    let env = env();
    let pdf = add_inbox(&env, "Лекция.pdf", LECTURE_PDF);
    let service = service(&env);
    service.analyze(&pdf, false, no_cancel()).unwrap();
    service
        .set_selection(None, std::slice::from_ref(&pdf))
        .unwrap();
    assert_eq!(service.clear_cache().unwrap(), 1);
    assert_eq!(service.get(&pdf).unwrap_err().code, ErrorCode::NotFound);
    assert!(!service.selection(None).unwrap().items[0].ready);
    let rebuilt = service.analyze(&pdf, false, no_cancel()).unwrap();
    assert!(!rebuilt.cached);
    assert!(
        service.selection(None).unwrap().items[0].ready,
        "same content becomes ready again"
    );
}

#[test]
fn ocr_setting_is_off_by_default_and_changes_cache_key() {
    let env = env();
    let image = add_inbox(&env, "schema.png", DIAGRAM_PNG);
    let engine: Arc<dyn OcrEngine> = Arc::new(FakeOcr {
        text: Some("Схема воронки"),
        hang: false,
        panic: false,
    });
    let service = ExtractionService::new(
        &env.database,
        None,
        ExtractionLimits::default(),
        Some(engine),
    );
    let settings = service.settings().unwrap();
    assert!(!settings.ocr_enabled);
    assert!(settings.ocr.available);
    let without = service.analyze(&image, false, no_cancel()).unwrap();
    assert_eq!(without.status, ExtractionStatus::NeedsOcr);
    assert!(without.text.is_empty());

    service.set_ocr_enabled(true).unwrap();
    let with = service.analyze(&image, false, no_cancel()).unwrap();
    assert!(!with.cached, "OCR result is a different cache entry");
    assert_eq!(with.text, "Схема воронки");
    assert!(with.ocr_used);
    assert!(store::read_settings(&env.database).unwrap().ocr_enabled);

    let missing_engine =
        ExtractionService::new(&env.database, None, ExtractionLimits::default(), None);
    assert!(!missing_engine.settings().unwrap().ocr.available);
    let unavailable = missing_engine.analyze(&image, false, no_cancel()).unwrap();
    assert_eq!(unavailable.status, ExtractionStatus::NeedsOcr);
}

#[test]
fn transient_results_are_not_reused_from_cache() {
    let env = env();
    let material = add_inbox(&env, "Лекция.pdf", LECTURE_PDF);
    let limits = ExtractionLimits {
        time_limit: Duration::ZERO,
        ..ExtractionLimits::default()
    };
    let timed_out = ExtractionService::new(&env.database, None, limits, None)
        .analyze(&material, false, no_cancel())
        .unwrap();
    assert_eq!(timed_out.status, ExtractionStatus::Timeout);
    let retried = service(&env)
        .analyze(&material, false, no_cancel())
        .unwrap();
    assert!(!retried.cached);
    assert_eq!(retried.status, ExtractionStatus::Complete);
}

/// Ручная проверка настоящего Tesseract: `cargo test -- --ignored real_tesseract`.
/// В обычном прогоне не запускается — внешняя программа не обязательна.
#[test]
#[ignore = "требует установленного локального tesseract"]
fn real_tesseract_recognizes_sample_when_installed() {
    let Some(engine) = super::ocr::TesseractOcr::discover() else {
        eprintln!("tesseract не найден — проверка пропущена");
        return;
    };
    let sample = include_bytes!("../../tests/fixtures/extraction/ocr-sample.png");
    let result = run_isolated(
        "sample.png",
        Arc::new(sample.to_vec()),
        ExtractionLimits::default(),
        no_cancel(),
        Some(Arc::new(engine)),
        true,
    )
    .unwrap();
    assert_eq!(
        result.status,
        ExtractionStatus::Complete,
        "{:?}",
        result.status_message
    );
    assert!(result.ocr_used);
    assert!(result.text.contains("Kontur OCR test"), "{}", result.text);
}
