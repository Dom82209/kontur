use super::diff::{
    compare_sets, ChangeCategory, ChangeKind, ComparisonStatus, FileInput, EXTRACTOR_VERSION,
    FORMULAS_NOT_RECALCULATED, MAX_FRAGMENT_CHARS,
};
use super::*;
use crate::extraction::fixtures::{package, rels, root_rels, simple_pdf};

fn file(path: &str, bytes: &[u8]) -> FileInput {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    FileInput {
        name: path.rsplit('/').next().unwrap().to_owned(),
        relative_path: path.to_owned(),
        size_bytes: bytes.len() as u64,
        sha256: Some(hex(&hasher.finalize())),
        bytes: Some(bytes.to_vec()),
        main: true,
    }
}

fn attachment(path: &str, bytes: &[u8]) -> FileInput {
    FileInput {
        main: false,
        ..file(path, bytes)
    }
}

fn of(changes: &[ChangeDto], category: ChangeCategory, kind: ChangeKind) -> Vec<&ChangeDto> {
    changes
        .iter()
        .filter(|c| c.category == category && c.kind == kind)
        .collect()
}

#[test]
fn identical_versions_have_no_changes() {
    let a = file("ДЗ/Решение.md", b"# A\n\nText");
    let body = compare_sets(std::slice::from_ref(&a), std::slice::from_ref(&a));
    assert!(body.changes.is_empty());
    assert_eq!(body.status(), ComparisonStatus::Identical);
    assert_eq!(body.summary(), "Содержимое версий совпадает");
}

#[test]
fn plain_text_diff_reports_added_removed_and_modified_fragments() {
    let a = file(
        "Решение.txt",
        "Первый абзац остаётся.\n\nСреднее равно 10 по всем месяцам.\n\nЭтот абзац удалят."
            .as_bytes(),
    );
    let b = file(
        "Решение.txt",
        "Первый абзац остаётся.\n\nСреднее равно 12 по всем месяцам.\n\nНовый вывод про сезонность."
            .as_bytes(),
    );
    let body = compare_sets(&[a], &[b]);
    let modified = of(&body.changes, ChangeCategory::Text, ChangeKind::Modified);
    assert_eq!(modified.len(), 1);
    assert!(modified[0].before.as_deref().unwrap().contains("10"));
    assert!(modified[0].after.as_deref().unwrap().contains("12"));
    assert_eq!(
        of(&body.changes, ChangeCategory::Text, ChangeKind::Removed).len(),
        1
    );
    assert_eq!(
        of(&body.changes, ChangeCategory::Text, ChangeKind::Added).len(),
        1
    );
    assert_eq!(body.status(), ComparisonStatus::Complete);
    // Фрагменты короткие: полный текст не хранится.
    let long = "слово ".repeat(200);
    let c = file("Решение.txt", long.as_bytes());
    let body = compare_sets(&[file("Решение.txt", b"x")], &[c]);
    assert!(body
        .changes
        .iter()
        .filter_map(|c| c.after.as_deref())
        .all(|t| t.chars().count() <= MAX_FRAGMENT_CHARS));
}

#[test]
fn markdown_diff_tracks_headings_lists_and_structure() {
    let a = file(
        "Решение.md",
        "# Решение\n\n## Данные\n\nОписание данных.\n\n- среднее\n- медиана\n".as_bytes(),
    );
    let b = file(
        "Решение.md",
        "# Решение\n\n### Данные\n\nОписание данных.\n\n- среднее\n- медиана\n- мода\n\n## Выводы\n\nСезонность есть.\n"
            .as_bytes(),
    );
    let body = compare_sets(&[a], &[b]);
    let level = of(&body.changes, ChangeCategory::Heading, ChangeKind::Modified);
    assert_eq!(level.len(), 1);
    assert_eq!(level[0].detail.as_deref(), Some("Уровень заголовка 2 → 3"));
    let heading = of(&body.changes, ChangeCategory::Heading, ChangeKind::Added);
    assert_eq!(heading[0].after.as_deref(), Some("Выводы"));
    let list = of(&body.changes, ChangeCategory::List, ChangeKind::Added);
    assert_eq!(list[0].after.as_deref(), Some("мода"));
    assert_eq!(list[0].location.as_deref(), Some("Данные"));
    let added_text = of(&body.changes, ChangeCategory::Text, ChangeKind::Added);
    assert_eq!(added_text[0].location.as_deref(), Some("Выводы"));
    let structure = of(
        &body.changes,
        ChangeCategory::Structure,
        ChangeKind::Modified,
    );
    assert_eq!(structure.len(), 1);
    assert!(body.summary().contains("структура"));
}

#[test]
fn pdf_with_text_is_compared_by_pages() {
    let a = file(
        "Отчёт.pdf",
        &simple_pdf(&["Revenue is 10", "Conclusion stays"]),
    );
    let b = file(
        "Отчёт.pdf",
        &simple_pdf(&["Revenue is 12", "Conclusion stays"]),
    );
    let body = compare_sets(&[a], &[b]);
    let pages = of(&body.changes, ChangeCategory::Page, ChangeKind::Modified);
    assert_eq!(pages.len(), 1, "{:?}", body.changes);
    assert_eq!(pages[0].location.as_deref(), Some("стр. 1"));
    assert!(body.manual_checks.is_empty());
}

#[test]
fn pdf_without_text_requires_manual_check() {
    let a = file("Скан.pdf", &simple_pdf(&[""]));
    let b = file("Скан.pdf", &simple_pdf(&["", ""]));
    let body = compare_sets(&[a], &[b]);
    assert_eq!(body.status(), ComparisonStatus::ManualRequired);
    assert!(body.manual_checks[0].contains("текстового слоя"));
    assert_eq!(
        of(&body.changes, ChangeCategory::Binary, ChangeKind::Modified).len(),
        1
    );
}

fn workbook(sheets: &[(&str, &str)]) -> Vec<u8> {
    let entries: String = sheets
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            format!(
                r#"<sheet name="{name}" sheetId="{}" r:id="rId{}"/>"#,
                i + 1,
                i + 1
            )
        })
        .collect();
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{entries}</sheets></workbook>"#
    );
    let targets: Vec<(String, String)> = (0..sheets.len())
        .map(|i| {
            (
                format!("rId{}", i + 1),
                format!("worksheets/sheet{}.xml", i + 1),
            )
        })
        .collect();
    let rel_items: Vec<(&str, &str, &str)> = targets
        .iter()
        .map(|(id, target)| (id.as_str(), "worksheet", target.as_str()))
        .collect();
    let workbook_rels = rels(&rel_items);
    let xml: Vec<(String, String)> = sheets
        .iter()
        .enumerate()
        .map(|(i, (_, data))| {
            (
                format!("xl/worksheets/sheet{}.xml", i + 1),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{data}</sheetData></worksheet>"#
                ),
            )
        })
        .collect();
    let root = root_rels("xl/workbook.xml");
    let mut parts: Vec<(&str, &[u8])> = vec![
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
    ];
    for (path, body) in &xml {
        parts.push((path.as_str(), body.as_bytes()));
    }
    package(&parts)
}

#[test]
fn excel_diff_reports_sheets_values_formulas_and_types() {
    let a = file(
        "Расчёт.xlsx",
        &workbook(&[
            (
                "Расчёт",
                r#"<row r="1"><c r="A1"><v>10</v></c><c r="B1"><f>A1*2</f><v>20</v></c><c r="C1" t="inlineStr"><is><t>итог</t></is></c></row>"#,
            ),
            ("Черновик", r#"<row r="1"><c r="A1"><v>1</v></c></row>"#),
        ]),
    );
    let b = file(
        "Расчёт.xlsx",
        &workbook(&[
            (
                "Расчёт",
                r#"<row r="1"><c r="A1"><v>12</v></c><c r="B1"><f>A1*3</f><v>36</v></c><c r="C1"><v>5</v></c><c r="D1"><v>7</v></c></row>"#,
            ),
            (
                "Выводы",
                r#"<row r="1"><c r="A1" t="inlineStr"><is><t>ок</t></is></c></row>"#,
            ),
        ]),
    );
    let body = compare_sets(&[a], &[b]);
    assert_eq!(
        of(&body.changes, ChangeCategory::Sheet, ChangeKind::Added)[0]
            .location
            .as_deref(),
        Some("Выводы")
    );
    assert_eq!(
        of(&body.changes, ChangeCategory::Sheet, ChangeKind::Removed)[0]
            .location
            .as_deref(),
        Some("Черновик")
    );
    let cells = of(&body.changes, ChangeCategory::Cell, ChangeKind::Modified);
    let a1 = cells
        .iter()
        .find(|c| c.location.as_deref() == Some("Расчёт!A1"))
        .unwrap();
    assert_eq!(
        (a1.before.as_deref(), a1.after.as_deref()),
        (Some("10"), Some("12"))
    );
    let c1 = cells
        .iter()
        .find(|c| c.location.as_deref() == Some("Расчёт!C1"))
        .unwrap();
    assert!(c1.detail.as_deref().unwrap().contains("тип текст → число"));
    let formula = of(&body.changes, ChangeCategory::Formula, ChangeKind::Modified);
    assert_eq!(formula[0].location.as_deref(), Some("Расчёт!B1"));
    assert!(formula[0].before.as_deref().unwrap().contains("A1*2"));
    assert!(formula[0].after.as_deref().unwrap().contains("A1*3"));
    let added = of(&body.changes, ChangeCategory::Cell, ChangeKind::Added);
    assert_eq!(added[0].location.as_deref(), Some("Расчёт!D1"));
    // Формулы не пересчитываются — это честно указано.
    assert!(body.warnings.iter().any(|w| w == FORMULAS_NOT_RECALCULATED));
    assert_eq!(body.status(), ComparisonStatus::Complete);
}

#[test]
fn file_sets_detect_added_removed_renamed_and_attachments() {
    let main_a = file("ДЗ/Решение.md", b"# A");
    let main_b = file("ДЗ/Решение.md", b"# A");
    let base = vec![
        main_a,
        attachment("ДЗ/Данные.csv", b"a,b"),
        attachment("ДЗ/Схема.png", b"\x89PNG"),
    ];
    let target = vec![
        main_b,
        attachment("ДЗ/data.csv", b"a,b"),
        attachment("ДЗ/Приложение.txt", b"new"),
    ];
    let body = compare_sets(&base, &target);
    let renamed = of(&body.changes, ChangeCategory::File, ChangeKind::Renamed);
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].before.as_deref(), Some("ДЗ/Данные.csv"));
    assert_eq!(renamed[0].after.as_deref(), Some("ДЗ/data.csv"));
    assert_eq!(
        of(&body.changes, ChangeCategory::File, ChangeKind::Removed)[0].file,
        "Схема.png"
    );
    assert_eq!(
        of(&body.changes, ChangeCategory::File, ChangeKind::Added)[0].file,
        "Приложение.txt"
    );
    // Убранное вложение требует ручной проверки обязательности.
    assert!(body.manual_checks.iter().any(|m| m.contains("Схема.png")));
    assert_eq!(body.status(), ComparisonStatus::ManualRequired);
}

#[test]
fn unsupported_format_is_compared_by_size_and_hash_only() {
    let a = file("ДЗ/Модель.bpmn.bin", b"\x00\x01binary");
    let b = file("ДЗ/Модель.bpmn.bin", b"\x00\x02binary-longer");
    let body = compare_sets(&[a], &[b]);
    let binary = of(&body.changes, ChangeCategory::Binary, ChangeKind::Modified);
    assert_eq!(binary.len(), 1);
    assert!(binary[0].before.as_deref().unwrap().starts_with("SHA "));
    assert_eq!(body.status(), ComparisonStatus::ManualRequired);
    assert!(body.manual_checks[0].contains("не поддерживается"));
    // Непрочитанный файл тоже честно требует ручной проверки.
    let mut unread = file("ДЗ/Решение.md", b"x");
    unread.sha256 = None;
    unread.bytes = None;
    let body = compare_sets(&[file("ДЗ/Решение.md", b"x")], &[unread]);
    assert_eq!(body.status(), ComparisonStatus::ManualRequired);
}

#[test]
fn comparison_becomes_stale_when_files_or_extractor_change() {
    let files = vec![file("a.md", b"1"), file("b.md", b"2")];
    let fp = set_fingerprint(&files);
    let reversed: Vec<FileInput> = files.iter().rev().cloned().collect();
    assert_eq!(fp, set_fingerprint(&reversed), "порядок файлов не влияет");
    let stored = VersionComparisonDto {
        id: "c".into(),
        homework_id: "hw".into(),
        base_version_id: "v1".into(),
        target_version_id: "v2".into(),
        base_fingerprint: fp.clone(),
        target_fingerprint: fp.clone(),
        extractor_version: EXTRACTOR_VERSION,
        status: ComparisonStatus::Identical,
        summary: String::new(),
        changes: Vec::new(),
        warnings: Vec::new(),
        manual_checks: Vec::new(),
        created_at: String::new(),
        stale: false,
    };
    assert!(!is_stale(&stored, &fp, &fp));
    let changed = set_fingerprint(&[file("a.md", b"1!"), file("b.md", b"2")]);
    assert!(is_stale(&stored, &fp, &changed));
    let old_algorithm = VersionComparisonDto {
        extractor_version: EXTRACTOR_VERSION - 1,
        ..stored
    };
    assert!(is_stale(&old_algorithm, &fp, &fp));
}
