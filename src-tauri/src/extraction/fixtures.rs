//! Синтетические fixtures без личных данных.
//!
//! Файлы из `tests/fixtures/extraction` созданы скриптом `generate.py`
//! (настоящие PDF/DOCX/XLSX/PPTX из распространённых библиотек). Вредоносные
//! и пограничные Office-пакеты собираются здесь вручную.

use crate::lms::zip_safe::test_zip::{build, RawEntry};

pub const LECTURE_PDF: &[u8] =
    include_bytes!("../../tests/fixtures/extraction/lecture-cyrillic.pdf");
pub const SCANNED_PDF: &[u8] = include_bytes!("../../tests/fixtures/extraction/scanned.pdf");
pub const ENCRYPTED_PDF: &[u8] = include_bytes!("../../tests/fixtures/extraction/encrypted.pdf");
pub const SEMINAR_DOCX: &[u8] = include_bytes!("../../tests/fixtures/extraction/seminar.docx");
pub const DATASET_XLSX: &[u8] = include_bytes!("../../tests/fixtures/extraction/dataset.xlsx");
pub const SLIDES_PPTX: &[u8] = include_bytes!("../../tests/fixtures/extraction/slides.pptx");
pub const DIAGRAM_PNG: &[u8] = include_bytes!("../../tests/fixtures/extraction/diagram.png");

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#;

pub fn root_rels(main: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{main}"/></Relationships>"#
    )
}

pub fn rels(items: &[(&str, &str, &str)]) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (id, kind, target) in items {
        let external = if target.starts_with("http") || target.starts_with("file:") {
            r#" TargetMode="External""#
        } else {
            ""
        };
        out.push_str(&format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"{external}/>"#
        ));
    }
    out.push_str("</Relationships>");
    out
}

/// Пакет из пар (путь, содержимое). `[Content_Types].xml` добавляется сам.
pub fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut entries = vec![RawEntry::file(
        "[Content_Types].xml",
        CONTENT_TYPES.as_bytes(),
    )];
    entries.extend(parts.iter().map(|(path, data)| RawEntry::file(path, data)));
    build(&entries, false)
}

pub const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;

pub fn docx_with(body: &str, document_rels: Option<&str>, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document {W}><w:body>{body}</w:body></w:document>"#
    );
    let root = root_rels("word/document.xml");
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", root.as_bytes()),
        ("word/document.xml", document.as_bytes()),
    ];
    if let Some(document_rels) = document_rels {
        parts.push(("word/_rels/document.xml.rels", document_rels.as_bytes()));
    }
    parts.extend_from_slice(extra);
    package(&parts)
}

pub fn paragraph(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}

pub fn xlsx_with(sheet_xml: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let workbook = r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Лист1" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let workbook_rels = rels(&[("rId1", "worksheet", "worksheets/sheet1.xml")]);
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet_xml}</sheetData></worksheet>"#
    );
    let root = root_rels("xl/workbook.xml");
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ];
    parts.extend_from_slice(extra);
    package(&parts)
}

/// Минимальный текстовый PDF (Helvetica, WinAnsi) без внешних библиотек.
pub fn simple_pdf(pages: &[&str]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let page_count = pages.len();
    let kids: Vec<String> = (0..page_count)
        .map(|index| format!("{} 0 R", 4 + index * 2))
        .collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {page_count} >>",
        kids.join(" ")
    ));
    objects.push(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    );
    for (index, text) in pages.iter().enumerate() {
        let content_id = 5 + index * 2;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>"
        ));
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
