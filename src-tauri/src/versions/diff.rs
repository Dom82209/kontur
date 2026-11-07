//! Этап 10: структурное сравнение содержимого двух версий (чистые функции).
//!
//! Текст и Markdown сравниваются по блокам (заголовки, абзацы, пункты списков),
//! PDF — по извлечённому тексту страниц, Excel — по листам и ячейкам (значения,
//! формулы, типы; формулы не пересчитываются), набор файлов — по SHA-256.
//! Неподдерживаемые и двоичные форматы сравниваются только по размеру и хешу —
//! с честным «требуется ручная проверка». Полные тексты не сохраняются:
//! фрагменты ограничены `MAX_FRAGMENT_CHARS`, число изменений — `MAX_CHANGES`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::extraction::{
    engine::extract_bytes,
    image::OcrMode,
    model::{Block, Budget},
    ExtractionLimits, ExtractionStatus,
};

/// Версия алгоритма сравнения и извлечения: при её смене прежние сравнения устаревают.
pub const EXTRACTOR_VERSION: i64 = 1;
pub const MAX_FRAGMENT_CHARS: usize = 240;
pub const MAX_CHANGES: usize = 400;
/// Выше этого числа блоков LCS не строится (квадратичная память).
const MAX_DIFF_BLOCKS: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeCategory {
    File,
    Text,
    Heading,
    List,
    Structure,
    Page,
    Sheet,
    Cell,
    Formula,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDto {
    pub kind: ChangeKind,
    pub category: ChangeCategory,
    pub file: String,
    /// Раздел, страница, лист/ячейка.
    pub location: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonStatus {
    Identical,
    Complete,
    Partial,
    ManualRequired,
}

impl ComparisonStatus {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Identical => "identical",
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::ManualRequired => "manual_required",
        }
    }
    pub fn from_db(value: &str) -> Self {
        match value {
            "identical" => Self::Identical,
            "complete" => Self::Complete,
            "partial" => Self::Partial,
            _ => Self::ManualRequired,
        }
    }
}

/// Файл версии, прочитанный для сравнения.
#[derive(Debug, Clone)]
pub struct FileInput {
    pub name: String,
    pub relative_path: String,
    pub size_bytes: u64,
    /// `None` — файл не прочитан (нет на диске, нет доступа).
    pub sha256: Option<String>,
    /// `None` — не прочитан или слишком большой для разбора.
    pub bytes: Option<Vec<u8>>,
    /// Основной файл версии (остальные — вложения).
    pub main: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComparisonBody {
    pub changes: Vec<ChangeDto>,
    pub warnings: Vec<String>,
    pub manual_checks: Vec<String>,
    pub truncated: bool,
}

impl ComparisonBody {
    fn push(&mut self, change: ChangeDto) {
        if self.changes.len() < MAX_CHANGES {
            self.changes.push(change);
        } else {
            self.truncated = true;
        }
    }

    pub fn status(&self) -> ComparisonStatus {
        if !self.manual_checks.is_empty() {
            ComparisonStatus::ManualRequired
        } else if self.truncated || !self.warnings.is_empty() && self.partial_warning() {
            ComparisonStatus::Partial
        } else if self.changes.is_empty() {
            ComparisonStatus::Identical
        } else {
            ComparisonStatus::Complete
        }
    }

    fn partial_warning(&self) -> bool {
        self.warnings.iter().any(|w| w.starts_with(PARTIAL_PREFIX))
    }

    pub fn summary(&self) -> String {
        if self.changes.is_empty() && self.manual_checks.is_empty() {
            return "Содержимое версий совпадает".to_owned();
        }
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        for change in &self.changes {
            let label = match change.category {
                ChangeCategory::File => "файлы",
                ChangeCategory::Text | ChangeCategory::Page => "текст",
                ChangeCategory::Heading | ChangeCategory::Structure => "структура",
                ChangeCategory::List => "списки",
                ChangeCategory::Sheet => "листы",
                ChangeCategory::Cell => "ячейки",
                ChangeCategory::Formula => "формулы",
                ChangeCategory::Binary => "двоичные файлы",
            };
            *counts.entry(label).or_default() += 1;
        }
        let mut parts: Vec<String> = counts
            .into_iter()
            .map(|(label, count)| format!("{label}: {count}"))
            .collect();
        if self.truncated {
            parts.push(format!("показаны первые {MAX_CHANGES} изменений"));
        }
        if !self.manual_checks.is_empty() {
            parts.push(format!(
                "требуется ручная проверка: {}",
                self.manual_checks.len()
            ));
        }
        format!("Изменения — {}", parts.join(", "))
    }
}

/// Предупреждения с этим префиксом означают неполное сравнение (`partial`).
pub const PARTIAL_PREFIX: &str = "Сравнение неполное: ";
pub const FORMULAS_NOT_RECALCULATED: &str =
    "Формулы не пересчитываются: сравниваются сохранённые значения и текст формул";

pub fn fragment(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_FRAGMENT_CHARS {
        return trimmed.to_owned();
    }
    let cut: String = trimmed.chars().take(MAX_FRAGMENT_CHARS - 1).collect();
    format!("{cut}…")
}

// ───────────────────────── Набор файлов ─────────────────────────

pub fn compare_sets(base: &[FileInput], target: &[FileInput]) -> ComparisonBody {
    let mut body = ComparisonBody::default();
    let base_by_path: BTreeMap<&str, &FileInput> =
        base.iter().map(|f| (f.relative_path.as_str(), f)).collect();
    let target_by_path: BTreeMap<&str, &FileInput> = target
        .iter()
        .map(|f| (f.relative_path.as_str(), f))
        .collect();
    let mut removed: Vec<&FileInput> = base
        .iter()
        .filter(|f| !target_by_path.contains_key(f.relative_path.as_str()))
        .collect();
    let mut added: Vec<&FileInput> = target
        .iter()
        .filter(|f| !base_by_path.contains_key(f.relative_path.as_str()))
        .collect();
    for file in [base, target]
        .concat()
        .iter()
        .filter(|f| f.sha256.is_none())
    {
        push_unique(
            &mut body.manual_checks,
            format!("Файл «{}» не прочитан — сравните вручную", file.name),
        );
    }

    // Переименование: тот же SHA-256 под другим путём.
    let mut renamed = Vec::new();
    removed.retain(|old| {
        let Some(hash) = &old.sha256 else { return true };
        if let Some(index) = added
            .iter()
            .position(|new| new.sha256.as_ref() == Some(hash))
        {
            renamed.push((*old, added.remove(index)));
            false
        } else {
            true
        }
    });
    // Основной файл заменён файлом с другим именем — сравниваем содержимое как изменение.
    let main_pair = match (
        removed.iter().position(|f| f.main),
        added.iter().position(|f| f.main),
    ) {
        (Some(old), Some(new)) => Some((removed.remove(old), added.remove(new))),
        _ => None,
    };

    for (old, new) in renamed {
        body.push(ChangeDto {
            kind: ChangeKind::Renamed,
            category: ChangeCategory::File,
            file: new.name.clone(),
            location: None,
            before: Some(old.relative_path.clone()),
            after: Some(new.relative_path.clone()),
            detail: Some("Содержимое совпадает (тот же SHA-256)".to_owned()),
        });
    }
    for file in &removed {
        body.push(ChangeDto {
            kind: ChangeKind::Removed,
            category: ChangeCategory::File,
            file: file.name.clone(),
            location: None,
            before: Some(file.relative_path.clone()),
            after: None,
            detail: None,
        });
        if !file.main {
            body.manual_checks.push(format!(
                "Убрано вложение «{}» — проверьте, не требуется ли оно по заданию",
                file.name
            ));
        }
    }
    for file in &added {
        body.push(ChangeDto {
            kind: ChangeKind::Added,
            category: ChangeCategory::File,
            file: file.name.clone(),
            location: None,
            before: None,
            after: Some(file.relative_path.clone()),
            detail: None,
        });
    }
    let mut pairs: Vec<(&FileInput, &FileInput)> = base
        .iter()
        .filter_map(|old| {
            target_by_path
                .get(old.relative_path.as_str())
                .map(|new| (old, *new))
        })
        .collect();
    if let Some((old, new)) = main_pair {
        pairs.insert(0, (old, new));
    }
    for (old, new) in pairs {
        if old.sha256.is_some() && old.sha256 == new.sha256 {
            continue;
        }
        if old.sha256.is_none() || new.sha256.is_none() {
            continue;
        }
        body.push(ChangeDto {
            kind: ChangeKind::Modified,
            category: ChangeCategory::File,
            file: new.name.clone(),
            location: None,
            before: (old.relative_path != new.relative_path).then(|| old.relative_path.clone()),
            after: (old.relative_path != new.relative_path).then(|| new.relative_path.clone()),
            detail: Some(format!(
                "Размер {} → {} байт",
                old.size_bytes, new.size_bytes
            )),
        });
        compare_contents(old, new, &mut body);
    }
    body
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Markdown,
    Docx,
    Pdf,
    Xlsx,
    Other,
}

fn format_of(name: &str) -> Format {
    let lower = name.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "md" | "markdown" => Format::Markdown,
        "txt" => Format::Text,
        "docx" => Format::Docx,
        "pdf" => Format::Pdf,
        "xlsx" | "xlsm" => Format::Xlsx,
        _ => Format::Other,
    }
}

fn binary(old: &FileInput, new: &FileInput, body: &mut ComparisonBody, why: &str) {
    body.push(ChangeDto {
        kind: ChangeKind::Modified,
        category: ChangeCategory::Binary,
        file: new.name.clone(),
        location: None,
        before: old
            .sha256
            .as_deref()
            .map(|h| format!("SHA {}", &h[..h.len().min(12)])),
        after: new
            .sha256
            .as_deref()
            .map(|h| format!("SHA {}", &h[..h.len().min(12)])),
        detail: Some(format!("{} → {} байт", old.size_bytes, new.size_bytes)),
    });
    body.manual_checks
        .push(format!("Сравните «{}» вручную: {why}", new.name));
}

fn compare_contents(old: &FileInput, new: &FileInput, body: &mut ComparisonBody) {
    let format = format_of(&new.name);
    if format != format_of(&old.name) {
        return binary(old, new, body, "у версий разные форматы файла");
    }
    let (Some(old_bytes), Some(new_bytes)) = (&old.bytes, &new.bytes) else {
        return binary(old, new, body, "файл слишком большой для разбора");
    };
    match format {
        Format::Markdown | Format::Text => {
            let (Ok(a), Ok(b)) = (
                std::str::from_utf8(old_bytes),
                std::str::from_utf8(new_bytes),
            ) else {
                return binary(old, new, body, "текст не в UTF-8");
            };
            let markdown = format == Format::Markdown;
            compare_units(
                &new.name,
                &text_units(a, markdown),
                &text_units(b, markdown),
                body,
            );
        }
        Format::Docx => match (extract(&old.name, old_bytes), extract(&new.name, new_bytes)) {
            (Some(a), Some(b)) => {
                note_partial(&a, &b, &new.name, body);
                compare_units(
                    &new.name,
                    &block_units(&a.blocks),
                    &block_units(&b.blocks),
                    body,
                )
            }
            _ => binary(old, new, body, "текст документа не извлечён"),
        },
        Format::Pdf => compare_pdf(old, new, old_bytes, new_bytes, body),
        Format::Xlsx => match (extract(&old.name, old_bytes), extract(&new.name, new_bytes)) {
            (Some(a), Some(b)) => compare_workbooks(&new.name, &a, &b, body),
            _ => binary(old, new, body, "книга Excel не прочитана"),
        },
        Format::Other => binary(
            old,
            new,
            body,
            "формат не поддерживается для сравнения содержимого",
        ),
    }
}

struct Doc {
    status: ExtractionStatus,
    blocks: Vec<Block>,
    truncated: bool,
}

fn extract(name: &str, bytes: &[u8]) -> Option<Doc> {
    let extracted = extract_bytes(
        name,
        bytes,
        &ExtractionLimits::default(),
        &Budget::unlimited(),
        OcrMode::Disabled,
    )
    .ok()?;
    match extracted.status {
        ExtractionStatus::Complete | ExtractionStatus::Partial | ExtractionStatus::NeedsOcr => {
            Some(Doc {
                status: extracted.status,
                truncated: extracted.truncated || extracted.blocks_truncated,
                blocks: extracted.blocks,
            })
        }
        _ => None,
    }
}

fn note_partial(a: &Doc, b: &Doc, name: &str, body: &mut ComparisonBody) {
    if a.truncated
        || b.truncated
        || a.status == ExtractionStatus::Partial
        || b.status == ExtractionStatus::Partial
    {
        push_unique(
            &mut body.warnings,
            format!("{PARTIAL_PREFIX}текст «{name}» извлечён не полностью"),
        );
    }
}

fn compare_pdf(old: &FileInput, new: &FileInput, a: &[u8], b: &[u8], body: &mut ComparisonBody) {
    let (Some(a), Some(b)) = (extract(&old.name, a), extract(&new.name, b)) else {
        return binary(old, new, body, "PDF не прочитан");
    };
    let pages = |doc: &Doc| -> Vec<(u32, String)> {
        doc.blocks
            .iter()
            .filter_map(|block| match block {
                Block::Page { number, text } => Some((*number, text.clone())),
                _ => None,
            })
            .collect()
    };
    let (pa, pb) = (pages(&a), pages(&b));
    let no_text = |p: &[(u32, String)], doc: &Doc| {
        doc.status == ExtractionStatus::NeedsOcr || p.iter().all(|(_, t)| t.trim().is_empty())
    };
    if no_text(&pa, &a) || no_text(&pb, &b) {
        return binary(
            old,
            new,
            body,
            "в PDF нет текстового слоя (нужен OCR, он для сравнения не запускается)",
        );
    }
    note_partial(&a, &b, &new.name, body);
    if pa.len() != pb.len() {
        body.push(ChangeDto {
            kind: ChangeKind::Modified,
            category: ChangeCategory::Structure,
            file: new.name.clone(),
            location: None,
            before: Some(format!("{} стр.", pa.len())),
            after: Some(format!("{} стр.", pb.len())),
            detail: Some("Изменилось число страниц; номера страниц у фрагментов — по версии, где фрагмент есть".to_owned()),
        });
    }
    let units = |p: &[(u32, String)]| -> Vec<Unit> {
        p.iter()
            .flat_map(|(number, text)| {
                text.split('\n')
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(move |line| Unit {
                        kind: UnitKind::Paragraph,
                        section: Some(format!("стр. {number}")),
                        text: line.to_owned(),
                    })
            })
            .collect()
    };
    compare_units_as(
        &new.name,
        &units(&pa),
        &units(&pb),
        body,
        ChangeCategory::Page,
    );
}

// ───────────────────────── Текст и структура ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitKind {
    Heading(u8),
    Paragraph,
    ListItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Unit {
    kind: UnitKind,
    section: Option<String>,
    text: String,
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
        Some((hashes as u8, line[hashes..].trim()))
    } else {
        None
    }
}

fn list_item(line: &str) -> Option<&str> {
    let t = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &t[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(rest.trim());
        }
    }
    None
}

fn text_units(text: &str, markdown: bool) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut section: Option<String> = None;
    let mut paragraph: Vec<&str> = Vec::new();
    let mut in_code = false;
    let flush = |paragraph: &mut Vec<&str>, units: &mut Vec<Unit>, section: &Option<String>| {
        if !paragraph.is_empty() {
            units.push(Unit {
                kind: UnitKind::Paragraph,
                section: section.clone(),
                text: paragraph.join(" "),
            });
            paragraph.clear();
        }
    };
    for raw in text.lines() {
        let line = raw.trim_end();
        if markdown && line.trim_start().starts_with("```") {
            in_code = !in_code;
            flush(&mut paragraph, &mut units, &section);
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut paragraph, &mut units, &section);
            continue;
        }
        if markdown && !in_code {
            if let Some((level, title)) = heading(line) {
                flush(&mut paragraph, &mut units, &section);
                units.push(Unit {
                    kind: UnitKind::Heading(level),
                    section: section.clone(),
                    text: title.to_owned(),
                });
                section = Some(title.to_owned());
                continue;
            }
            if let Some(item) = list_item(line) {
                flush(&mut paragraph, &mut units, &section);
                units.push(Unit {
                    kind: UnitKind::ListItem,
                    section: section.clone(),
                    text: item.to_owned(),
                });
                continue;
            }
        }
        paragraph.push(line.trim());
    }
    flush(&mut paragraph, &mut units, &section);
    units
}

fn block_units(blocks: &[Block]) -> Vec<Unit> {
    let mut section = None;
    let mut units = Vec::new();
    for block in blocks {
        match block {
            Block::Heading { level, text } => {
                units.push(Unit {
                    kind: UnitKind::Heading(*level),
                    section: section.clone(),
                    text: text.trim().to_owned(),
                });
                section = Some(text.trim().to_owned());
            }
            Block::Paragraph { text } if !text.trim().is_empty() => units.push(Unit {
                kind: UnitKind::Paragraph,
                section: section.clone(),
                text: text.trim().to_owned(),
            }),
            Block::ListItem { text, .. } => units.push(Unit {
                kind: UnitKind::ListItem,
                section: section.clone(),
                text: text.trim().to_owned(),
            }),
            Block::Table { rows, .. } => {
                for row in rows {
                    units.push(Unit {
                        kind: UnitKind::Paragraph,
                        section: section.clone(),
                        text: row.join(" | "),
                    });
                }
            }
            _ => {}
        }
    }
    units
}

fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn similar(a: &str, b: &str) -> bool {
    let (wa, wb) = (words(a), words(b));
    let union = wa.union(&wb).count();
    union > 0 && wa.intersection(&wb).count() * 2 >= union
}

enum Op<'a> {
    Same,
    Del(&'a Unit),
    Ins(&'a Unit),
}

fn diff_ops<'a>(a: &'a [Unit], b: &'a [Unit]) -> Vec<Op<'a>> {
    let key = |u: &Unit| (u.kind, u.text.clone());
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0_u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if key(&a[i]) == key(&b[j]) {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut ops = Vec::new();
    while i < n || j < m {
        if i < n && j < m && key(&a[i]) == key(&b[j]) {
            ops.push(Op::Same);
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            ops.push(Op::Ins(&b[j]));
            j += 1;
        } else {
            ops.push(Op::Del(&a[i]));
            i += 1;
        }
    }
    ops
}

fn compare_units(file: &str, a: &[Unit], b: &[Unit], body: &mut ComparisonBody) {
    compare_units_as(file, a, b, body, ChangeCategory::Text);
}

fn category(unit: &Unit, text: ChangeCategory) -> ChangeCategory {
    match unit.kind {
        UnitKind::Heading(_) => ChangeCategory::Heading,
        UnitKind::ListItem => ChangeCategory::List,
        UnitKind::Paragraph => text,
    }
}

fn compare_units_as(
    file: &str,
    a: &[Unit],
    b: &[Unit],
    body: &mut ComparisonBody,
    text_category: ChangeCategory,
) {
    if a.len() > MAX_DIFF_BLOCKS || b.len() > MAX_DIFF_BLOCKS {
        push_unique(
            &mut body.warnings,
            format!("{PARTIAL_PREFIX}«{file}» слишком длинный для поблочного сравнения"),
        );
        body.manual_checks.push(format!(
            "Сравните «{file}» вручную: документ слишком длинный"
        ));
        return;
    }
    let ops = diff_ops(a, b);
    // Соседние удаление и вставка одного вида с похожим текстом — изменение фрагмента.
    let mut index = 0;
    while index < ops.len() {
        let mut deleted = Vec::new();
        let mut inserted = Vec::new();
        while index < ops.len() && !matches!(ops[index], Op::Same) {
            match ops[index] {
                Op::Del(unit) => deleted.push(unit),
                Op::Ins(unit) => inserted.push(unit),
                Op::Same => {}
            }
            index += 1;
        }
        if deleted.is_empty() && inserted.is_empty() {
            index += 1;
            continue;
        }
        let mut used = vec![false; inserted.len()];
        for old in deleted {
            let pair = inserted.iter().enumerate().position(|(k, new)| {
                !used[k]
                    && std::mem::discriminant(&new.kind) == std::mem::discriminant(&old.kind)
                    && similar(&old.text, &new.text)
            });
            match pair {
                Some(k) => {
                    used[k] = true;
                    let new = inserted[k];
                    let level_changed = match (old.kind, new.kind) {
                        (UnitKind::Heading(x), UnitKind::Heading(y)) if x != y => {
                            Some(format!("Уровень заголовка {x} → {y}"))
                        }
                        _ => None,
                    };
                    body.push(ChangeDto {
                        kind: ChangeKind::Modified,
                        category: category(new, text_category),
                        file: file.to_owned(),
                        location: new.section.clone(),
                        before: Some(fragment(&old.text)),
                        after: Some(fragment(&new.text)),
                        detail: level_changed,
                    });
                }
                None => body.push(ChangeDto {
                    kind: ChangeKind::Removed,
                    category: category(old, text_category),
                    file: file.to_owned(),
                    location: old.section.clone(),
                    before: Some(fragment(&old.text)),
                    after: None,
                    detail: None,
                }),
            }
        }
        for (k, new) in inserted.into_iter().enumerate() {
            if !used[k] {
                body.push(ChangeDto {
                    kind: ChangeKind::Added,
                    category: category(new, text_category),
                    file: file.to_owned(),
                    location: new.section.clone(),
                    before: None,
                    after: Some(fragment(&new.text)),
                    detail: None,
                });
            }
        }
    }
    // Сводка структуры: число заголовков, абзацев и пунктов списков.
    let count = |units: &[Unit]| {
        let mut c = (0, 0, 0);
        for u in units {
            match u.kind {
                UnitKind::Heading(_) => c.0 += 1,
                UnitKind::Paragraph => c.1 += 1,
                UnitKind::ListItem => c.2 += 1,
            }
        }
        c
    };
    let (ca, cb) = (count(a), count(b));
    if ca != cb {
        body.push(ChangeDto {
            kind: ChangeKind::Modified,
            category: ChangeCategory::Structure,
            file: file.to_owned(),
            location: None,
            before: Some(format!(
                "заголовков {}, абзацев {}, пунктов {}",
                ca.0, ca.1, ca.2
            )),
            after: Some(format!(
                "заголовков {}, абзацев {}, пунктов {}",
                cb.0, cb.1, cb.2
            )),
            detail: None,
        });
    }
}

// ───────────────────────── Excel ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    value: String,
    formula: Option<String>,
}

fn cell_type(cell: &Cell) -> &'static str {
    if cell.formula.is_some() {
        "формула"
    } else if cell.value.trim().parse::<f64>().is_ok() {
        "число"
    } else if matches!(cell.value.as_str(), "TRUE" | "FALSE") {
        "логическое"
    } else {
        "текст"
    }
}

fn describe(cell: &Cell) -> String {
    fragment(&match &cell.formula {
        Some(formula) => format!("{} [={}]", cell.value, formula.trim_start_matches('=')),
        None => cell.value.clone(),
    })
}

struct SheetData {
    used_range: Option<String>,
    cells: BTreeMap<String, Cell>,
    truncated: bool,
}

fn sheets(doc: &Doc) -> BTreeMap<String, SheetData> {
    doc.blocks
        .iter()
        .filter_map(|block| match block {
            Block::Sheet {
                name,
                used_range,
                cells,
                truncated,
                ..
            } => Some((
                name.clone(),
                SheetData {
                    used_range: used_range.clone(),
                    truncated: *truncated,
                    cells: cells
                        .iter()
                        .map(|c| {
                            (
                                c.reference.clone(),
                                Cell {
                                    value: c.value.clone(),
                                    formula: c.formula.clone(),
                                },
                            )
                        })
                        .collect(),
                },
            )),
            _ => None,
        })
        .collect()
}

/// Сортировка ссылок A1: сначала строка, затем столбец.
fn cell_order(reference: &str) -> (u64, u64) {
    let letters: String = reference
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    let column = letters.chars().fold(0_u64, |acc, c| {
        acc * 26 + u64::from(c.to_ascii_uppercase() as u8 - b'A' + 1)
    });
    let row = reference[letters.len()..].parse().unwrap_or(0);
    (row, column)
}

fn compare_workbooks(file: &str, a: &Doc, b: &Doc, body: &mut ComparisonBody) {
    push_unique(&mut body.warnings, FORMULAS_NOT_RECALCULATED.to_owned());
    let (sa, sb) = (sheets(a), sheets(b));
    for name in sa.keys().filter(|n| !sb.contains_key(*n)) {
        body.push(ChangeDto {
            kind: ChangeKind::Removed,
            category: ChangeCategory::Sheet,
            file: file.to_owned(),
            location: Some(name.clone()),
            before: Some(name.clone()),
            after: None,
            detail: None,
        });
    }
    for (name, sheet) in sb.iter().filter(|(n, _)| !sa.contains_key(*n)) {
        body.push(ChangeDto {
            kind: ChangeKind::Added,
            category: ChangeCategory::Sheet,
            file: file.to_owned(),
            location: Some(name.clone()),
            before: None,
            after: Some(name.clone()),
            detail: Some(format!("Непустых ячеек: {}", sheet.cells.len())),
        });
    }
    for (name, old) in &sa {
        let Some(new) = sb.get(name) else { continue };
        if old.truncated || new.truncated {
            push_unique(
                &mut body.warnings,
                format!(
                    "{PARTIAL_PREFIX}лист «{name}» прочитан не полностью — сравнены первые ячейки"
                ),
            );
        }
        if old.used_range != new.used_range {
            body.push(ChangeDto {
                kind: ChangeKind::Modified,
                category: ChangeCategory::Structure,
                file: file.to_owned(),
                location: Some(name.clone()),
                before: old.used_range.clone(),
                after: new.used_range.clone(),
                detail: Some("Изменился используемый диапазон".to_owned()),
            });
        }
        let mut refs: Vec<&String> = old.cells.keys().chain(new.cells.keys()).collect();
        refs.sort_by_key(|r| cell_order(r));
        refs.dedup();
        for reference in refs {
            let location = Some(format!("{name}!{reference}"));
            match (old.cells.get(reference), new.cells.get(reference)) {
                (Some(x), None) => body.push(ChangeDto {
                    kind: ChangeKind::Removed,
                    category: ChangeCategory::Cell,
                    file: file.to_owned(),
                    location,
                    before: Some(describe(x)),
                    after: None,
                    detail: Some(cell_type(x).to_owned()),
                }),
                (None, Some(y)) => body.push(ChangeDto {
                    kind: ChangeKind::Added,
                    category: ChangeCategory::Cell,
                    file: file.to_owned(),
                    location,
                    before: None,
                    after: Some(describe(y)),
                    detail: Some(cell_type(y).to_owned()),
                }),
                (Some(x), Some(y)) if x != y => {
                    let mut what = Vec::new();
                    if x.value != y.value {
                        what.push("значение".to_owned());
                    }
                    if x.formula != y.formula {
                        what.push("формула".to_owned());
                    }
                    if cell_type(x) != cell_type(y) {
                        what.push(format!("тип {} → {}", cell_type(x), cell_type(y)));
                    }
                    body.push(ChangeDto {
                        kind: ChangeKind::Modified,
                        category: if x.formula != y.formula {
                            ChangeCategory::Formula
                        } else {
                            ChangeCategory::Cell
                        },
                        file: file.to_owned(),
                        location,
                        before: Some(describe(x)),
                        after: Some(describe(y)),
                        detail: Some(what.join(", ")),
                    });
                }
                _ => {}
            }
        }
    }
}
