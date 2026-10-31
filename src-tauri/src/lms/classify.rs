//! Предварительная классификация записей экспорта по структуре путей.
//!
//! Используются только имена папок и файлов — содержимое документов не
//! читается, AI не запускается. Ни одно решение не раскладывает файл: результат
//! показывается в preview, пользователь подтверждает или исправляет его, а
//! неполная или неуверенная классификация сохраняется как `ambiguous`.

use crate::dto::{ClassificationDto, ClassificationSource, InboxStatus, MaterialCategory};

/// Предмет, с которым сопоставляется курс экспорта.
#[derive(Debug, Clone)]
pub struct SubjectMatcher {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClassifyContext {
    pub subjects: Vec<SubjectMatcher>,
    /// Порог AutomationRules.lowConfidenceThreshold.
    pub threshold: f64,
    /// Название курса: общий корневой каталог архива или имя архива.
    pub course_title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntryClassification {
    pub classification: ClassificationDto,
    pub status: InboxStatus,
}

const WEIGHT_SUBJECT: f64 = 0.4;
const WEIGHT_WEEK: f64 = 0.3;
const WEIGHT_CATEGORY: f64 = 0.3;
const MAX_WEEK: i64 = 60;

/// Классифицирует запись по её пути внутри архива.
pub fn classify_path(path: &str, context: &ClassifyContext) -> EntryClassification {
    let segments: Vec<&str> = path.split('/').collect();
    let (directories, file) = segments.split_at(segments.len().saturating_sub(1));
    let file_name = file.first().copied().unwrap_or(path);
    let (stem, extension) = split_extension(file_name);
    let mut reasons = Vec::new();

    // ── Предмет ──
    let mut subject_score = 0.0;
    let subject_id = match match_subject(context, directories, stem) {
        SubjectMatch::Unique { id, label, via } => {
            reasons.push(format!("{via} совпадает с предметом «{label}»"));
            subject_score = 1.0;
            Some(id)
        }
        SubjectMatch::Conflict(labels) => {
            reasons.push(format!(
                "Подходит несколько предметов: {} — выберите вручную",
                labels.join(", ")
            ));
            None
        }
        SubjectMatch::None => {
            reasons.push(match &context.course_title {
                Some(title) => format!("Курс «{title}» не сопоставлен ни с одним предметом"),
                None => "Предмет по названию курса не определён".to_owned(),
            });
            None
        }
    };

    // ── Неделя ──
    let mut week_score = 0.0;
    let mut weeks: Vec<(i64, &str, bool)> = Vec::new();
    for directory in directories {
        if let Some(week) = find_week(directory) {
            weeks.push((week, directory, true));
        }
    }
    if let Some(week) = find_week(stem) {
        weeks.push((week, file_name, false));
    }
    let distinct: Vec<i64> = {
        let mut values: Vec<i64> = weeks.iter().map(|(week, _, _)| *week).collect();
        values.sort_unstable();
        values.dedup();
        values
    };
    let week_number = match distinct.as_slice() {
        [] => {
            reasons.push("Номер недели в пути не найден".to_owned());
            None
        }
        [week] => {
            let (_, source, in_directory) = weeks[0];
            reasons.push(format!(
                "{} «{source}» → неделя {week}",
                if in_directory {
                    "Папка"
                } else {
                    "Имя файла"
                }
            ));
            week_score = if weeks.iter().any(|(_, _, dir)| *dir) {
                1.0
            } else {
                0.8
            };
            Some(*week)
        }
        many => {
            reasons.push(format!(
                "В пути несколько номеров недель ({}) — уточните вручную",
                many.iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            None
        }
    };

    // ── Тип материала ──
    let mut category_score = 0.0;
    let mut kind: Option<(Kind, &str, bool)> = None;
    let mut conflict = false;
    for directory in directories.iter().rev() {
        let found = kinds_in(directory);
        if found.len() > 1 {
            conflict = true;
            break;
        }
        if let Some(found) = found.first() {
            kind = Some((*found, directory, true));
            break;
        }
    }
    if kind.is_none() && !conflict {
        let found = kinds_in(stem);
        if found.len() == 1 {
            kind = Some((found[0], file_name, false));
        } else if found.len() > 1 {
            conflict = true;
        }
    }
    let category = if conflict {
        reasons.push("Путь подходит под несколько типов материала — уточните вручную".to_owned());
        MaterialCategory::Unknown
    } else {
        match kind {
            None => {
                reasons.push("Тип материала по пути не определён".to_owned());
                MaterialCategory::Unknown
            }
            Some((Kind::Homework, source, in_directory)) => {
                let (category, certain, why) = homework_part(stem, extension);
                reasons.push(format!(
                    "{} «{source}» → домашнее задание; {why}",
                    if in_directory {
                        "Папка"
                    } else {
                        "Имя файла"
                    }
                ));
                category_score = match (in_directory, certain) {
                    (true, true) => 1.0,
                    (false, true) | (true, false) => 0.8,
                    (false, false) => 0.6,
                };
                category
            }
            Some((other, source, in_directory)) => {
                reasons.push(format!(
                    "{} «{source}» → {}",
                    if in_directory {
                        "Папка"
                    } else {
                        "Имя файла"
                    },
                    other.label()
                ));
                category_score = if in_directory { 1.0 } else { 0.8 };
                other.category()
            }
        }
    };

    let confidence = ((subject_score * WEIGHT_SUBJECT
        + week_score * WEIGHT_WEEK
        + category_score * WEIGHT_CATEGORY)
        * 100.0)
        .round()
        / 100.0;
    let complete =
        subject_id.is_some() && week_number.is_some() && category != MaterialCategory::Unknown;
    let status = if complete && confidence >= context.threshold {
        InboxStatus::Classified
    } else {
        InboxStatus::Ambiguous
    };
    EntryClassification {
        classification: ClassificationDto {
            subject_id,
            week_number,
            category,
            confidence,
            reasons,
            source: ClassificationSource::Auto,
            homework_id: None,
        },
        status,
    }
}

/// Ручная классификация из preview: уверенность 1, источник `manual`.
pub fn manual_classification(
    subject_id: Option<String>,
    week_number: Option<i64>,
    category: MaterialCategory,
) -> EntryClassification {
    let complete =
        subject_id.is_some() && week_number.is_some() && category != MaterialCategory::Unknown;
    EntryClassification {
        classification: ClassificationDto {
            subject_id,
            week_number,
            category,
            confidence: 1.0,
            reasons: vec!["Подтверждено пользователем в предпросмотре импорта".to_owned()],
            source: ClassificationSource::Manual,
            homework_id: None,
        },
        status: if complete {
            InboxStatus::Classified
        } else {
            InboxStatus::Ambiguous
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Lecture,
    Seminar,
    Homework,
    Material,
}

impl Kind {
    fn category(self) -> MaterialCategory {
        match self {
            Self::Lecture => MaterialCategory::Lecture,
            Self::Seminar => MaterialCategory::Seminar,
            Self::Homework => MaterialCategory::HomeworkTask,
            Self::Material => MaterialCategory::Material,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Lecture => "лекция",
            Self::Seminar => "семинар",
            Self::Homework => "домашнее задание",
            Self::Material => "дополнительный материал",
        }
    }
}

fn kinds_in(text: &str) -> Vec<Kind> {
    let words = words(text);
    let has = |prefixes: &[&str]| {
        words
            .iter()
            .any(|word| prefixes.iter().any(|prefix| word.starts_with(prefix)))
    };
    let exact = |values: &[&str]| words.iter().any(|word| values.contains(&word.as_str()));
    let mut found = Vec::new();
    if has(&["лекци", "lecture", "lect"]) {
        found.push(Kind::Lecture);
    }
    if has(&["семинар", "seminar", "практик", "practice", "practical"]) {
        found.push(Kind::Seminar);
    }
    if has(&["домашн", "homework", "assignment"]) || exact(&["дз", "hw"]) {
        found.push(Kind::Homework);
    }
    if has(&[
        "дополнит",
        "additional",
        "extra",
        "литератур",
        "reading",
        "материал",
    ]) || exact(&["доп"])
    {
        found.push(Kind::Material);
    }
    found
}

/// Условие или данные ДЗ: по ключевым словам имени, затем по расширению.
fn homework_part(stem: &str, extension: &str) -> (MaterialCategory, bool, &'static str) {
    let words = words(stem);
    let has = |prefixes: &[&str]| {
        words
            .iter()
            .any(|word| prefixes.iter().any(|prefix| word.starts_with(prefix)))
    };
    let data_words = has(&["данн", "data", "dataset", "датасет"]);
    let task_words = has(&["услови", "задани", "task", "statement", "problem"]);
    match (task_words, data_words) {
        (true, false) => {
            return (
                MaterialCategory::HomeworkTask,
                true,
                "условие по имени файла",
            )
        }
        (false, true) => {
            return (
                MaterialCategory::HomeworkData,
                true,
                "данные по имени файла",
            )
        }
        _ => {}
    }
    match extension {
        "csv" | "tsv" | "xlsx" | "xls" | "json" | "parquet" | "xml" | "sqlite" | "db" | "sav"
        | "dta" => (MaterialCategory::HomeworkData, true, "данные по типу файла"),
        "pdf" | "doc" | "docx" | "md" | "tex" | "html" | "rtf" | "odt" => (
            MaterialCategory::HomeworkTask,
            true,
            "условие по типу файла",
        ),
        "ipynb" => (
            MaterialCategory::HomeworkTask,
            false,
            "ноутбук — проверьте, условие это или данные",
        ),
        _ => (
            MaterialCategory::HomeworkTask,
            false,
            "условие или данные — уточните вручную",
        ),
    }
}

/// Номер недели: «Неделя 3», «неделя_03», «Week 3», «W03», «3 неделя», «нед. 3».
pub fn find_week(text: &str) -> Option<i64> {
    let tokens = words(text);
    let mut found = None;
    for (index, token) in tokens.iter().enumerate() {
        let is_week_word =
            token.starts_with("недел") || token == "нед" || token == "week" || token == "w";
        let candidate = if is_week_word {
            tokens.get(index + 1).and_then(|next| parse_number(next))
        } else if parse_number(token).is_some() {
            tokens
                .get(index + 1)
                .filter(|next| next.starts_with("недел") || next.as_str() == "week")
                .and_then(|_| parse_number(token))
        } else {
            None
        };
        if let Some(week) = candidate.filter(|week| (1..=MAX_WEEK).contains(week)) {
            match found {
                None => found = Some(week),
                Some(existing) if existing == week => {}
                Some(_) => return None,
            }
        }
    }
    found
}

fn parse_number(token: &str) -> Option<i64> {
    if token.is_empty() || token.len() > 3 || !token.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

/// Слова в нижнем регистре: разделители `_ - . , ( ) [ ]` и пробелы, граница
/// «буквы → цифры» тоже разделяет («неделя03» → «неделя», «03»).
fn words(text: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut previous_digit: Option<bool> = None;
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            let digit = ch.is_ascii_digit();
            if previous_digit.is_some_and(|previous| previous != digit) && !current.is_empty() {
                output.push(std::mem::take(&mut current));
            }
            current.push(ch);
            previous_digit = Some(digit);
        } else {
            if !current.is_empty() {
                output.push(std::mem::take(&mut current));
            }
            previous_digit = None;
        }
    }
    if !current.is_empty() {
        output.push(current);
    }
    output
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && extension.len() <= 10 => (stem, extension),
        _ => (name, ""),
    }
}

/// Нижний регистр расширения (без точки) для внешних модулей.
pub fn extension_of(name: &str) -> String {
    split_extension(name).1.to_lowercase()
}

enum SubjectMatch {
    Unique {
        id: String,
        label: String,
        via: String,
    },
    Conflict(Vec<String>),
    None,
}

fn match_subject(context: &ClassifyContext, directories: &[&str], stem: &str) -> SubjectMatch {
    let mut texts: Vec<(String, String)> = Vec::new();
    if let Some(title) = &context.course_title {
        texts.push((title.to_lowercase(), format!("Курс «{title}»")));
    }
    for directory in directories {
        texts.push((directory.to_lowercase(), format!("Папка «{directory}»")));
    }
    texts.push((stem.to_lowercase(), format!("Имя файла «{stem}»")));

    // Для каждого предмета — самое длинное совпадение; выигрывает самое длинное.
    let mut best: Vec<(usize, &SubjectMatcher, String)> = Vec::new();
    for subject in &context.subjects {
        let mut needles: Vec<String> = vec![subject.name.to_lowercase()];
        if subject.short_name.chars().count() >= 2 {
            needles.push(subject.short_name.to_lowercase());
        }
        needles.extend(
            subject
                .keywords
                .iter()
                .map(|keyword| keyword.trim().to_lowercase())
                .filter(|keyword| keyword.chars().count() >= 2),
        );
        let mut subject_best: Option<(usize, String)> = None;
        for (text, via) in &texts {
            for needle in &needles {
                if contains_word(text, needle) {
                    let length = needle.chars().count();
                    if subject_best.as_ref().is_none_or(|(best, _)| length > *best) {
                        subject_best = Some((length, via.clone()));
                    }
                }
            }
        }
        if let Some((length, via)) = subject_best {
            best.push((length, subject, via));
        }
    }
    let Some(longest) = best.iter().map(|(length, _, _)| *length).max() else {
        return SubjectMatch::None;
    };
    let winners: Vec<&(usize, &SubjectMatcher, String)> = best
        .iter()
        .filter(|(length, _, _)| *length == longest)
        .collect();
    if winners.len() == 1 {
        let (_, subject, via) = winners[0];
        SubjectMatch::Unique {
            id: subject.id.clone(),
            label: subject.name.clone(),
            via: via.clone(),
        }
    } else {
        SubjectMatch::Conflict(
            winners
                .iter()
                .map(|(_, subject, _)| subject.name.clone())
                .collect(),
        )
    }
}

/// Подстрока, ограниченная не-буквенными символами (чтобы «ОБА» не совпало с «работа»).
fn contains_word(text: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(position) = text[start..].find(needle) {
        let begin = start + position;
        let end = begin + needle.len();
        let before = text[..begin].chars().next_back();
        let after = text[end..].chars().next();
        let boundary = |ch: Option<char>| ch.is_none_or(|ch| !ch.is_alphanumeric());
        if boundary(before) && boundary(after) {
            return true;
        }
        start = begin + needle.len().max(1);
        while !text.is_char_boundary(start) {
            start += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> ClassifyContext {
        ClassifyContext {
            subjects: vec![
                SubjectMatcher {
                    id: "oba".to_owned(),
                    name: "Основы бизнес-аналитики".to_owned(),
                    short_name: "ОБА".to_owned(),
                    keywords: vec!["бизнес-аналит".to_owned()],
                },
                SubjectMatcher {
                    id: "stat".to_owned(),
                    name: "Статистика".to_owned(),
                    short_name: "Стат".to_owned(),
                    keywords: vec![],
                },
            ],
            threshold: 0.7,
            course_title: Some("Основы бизнес-аналитики".to_owned()),
        }
    }

    #[test]
    fn weeks_are_found_in_common_spellings() {
        assert_eq!(find_week("Неделя 3"), Some(3));
        assert_eq!(find_week("неделя_03"), Some(3));
        assert_eq!(find_week("Неделя07"), Some(7));
        assert_eq!(find_week("Week 12"), Some(12));
        assert_eq!(find_week("W05 Регрессия"), Some(5));
        assert_eq!(find_week("5 неделя"), Some(5));
        assert_eq!(find_week("нед. 4"), Some(4));
        assert_eq!(find_week("Лекция 3"), None);
        assert_eq!(find_week("Неделя 3 и неделя 4"), None);
        assert_eq!(find_week("Неделя 300"), None);
    }

    #[test]
    fn full_path_is_classified_with_reasons() {
        let result = classify_path(
            "Основы бизнес-аналитики/Неделя 3/Лекции/Лекция 3. Воронки.pdf",
            &context(),
        );
        assert_eq!(result.classification.subject_id.as_deref(), Some("oba"));
        assert_eq!(result.classification.week_number, Some(3));
        assert_eq!(result.classification.category, MaterialCategory::Lecture);
        assert_eq!(result.status, InboxStatus::Classified);
        assert_eq!(result.classification.source, ClassificationSource::Auto);
        assert!(result.classification.confidence >= 0.99);
    }

    #[test]
    fn homework_task_and_data_are_split() {
        let task = classify_path("Курс/Неделя 2/Домашнее задание/Условие ДЗ.pdf", &context());
        assert_eq!(task.classification.category, MaterialCategory::HomeworkTask);
        let data = classify_path("Курс/Неделя 2/ДЗ/sales.csv", &context());
        assert_eq!(data.classification.category, MaterialCategory::HomeworkData);
        let notebook = classify_path("Курс/Неделя 2/ДЗ/hw2.ipynb", &context());
        assert_eq!(
            notebook.classification.category,
            MaterialCategory::HomeworkTask
        );
        assert!(notebook.classification.confidence < task.classification.confidence);
    }

    #[test]
    fn unknown_course_makes_entry_ambiguous() {
        let mut context = context();
        context.course_title = Some("Философия".to_owned());
        let result = classify_path("Философия/Неделя 1/Семинары/Семинар 1.pdf", &context);
        assert_eq!(result.classification.subject_id, None);
        assert_eq!(result.classification.category, MaterialCategory::Seminar);
        assert_eq!(result.status, InboxStatus::Ambiguous);
    }

    #[test]
    fn conflicting_markers_are_not_guessed() {
        let weeks = classify_path("Курс/Неделя 3/Неделя 4/x.pdf", &context());
        assert_eq!(weeks.classification.week_number, None);
        assert_eq!(weeks.status, InboxStatus::Ambiguous);
        let kinds = classify_path("Курс/Неделя 3/Лекции и семинары/x.pdf", &context());
        assert_eq!(kinds.classification.category, MaterialCategory::Unknown);
        let mut tied = context();
        tied.course_title = None;
        tied.subjects[1].short_name = "ОБА".to_owned();
        let subject = classify_path("ОБА/Неделя 3/Лекции/x.pdf", &tied);
        assert_eq!(subject.classification.subject_id, None);
    }

    #[test]
    fn short_name_matches_only_whole_words() {
        let mut context = context();
        context.course_title = None;
        let result = classify_path("Работа/Неделя 3/Лекции/x.pdf", &context);
        assert_eq!(result.classification.subject_id, None);
        let result = classify_path("ОБА/Неделя 3/Лекции/x.pdf", &context);
        assert_eq!(result.classification.subject_id.as_deref(), Some("oba"));
    }

    #[test]
    fn manual_classification_is_full_confidence_and_complete_only_when_full() {
        let full =
            manual_classification(Some("oba".to_owned()), Some(2), MaterialCategory::Lecture);
        assert_eq!(full.status, InboxStatus::Classified);
        assert_eq!(full.classification.confidence, 1.0);
        let partial = manual_classification(None, Some(2), MaterialCategory::Lecture);
        assert_eq!(partial.status, InboxStatus::Ambiguous);
    }
}
