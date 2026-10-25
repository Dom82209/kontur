//! Разбор ответа провайдера в черновик для ручного подтверждения.
//!
//! Ответ никогда не применяется автоматически: из него строится черновик
//! (те же поля, что у ручного ввода), а пользователь проверяет, правит и
//! подтверждает его. Частичный ответ даёт черновик с пропущенными полями и
//! список замечаний; повреждённый — пустой черновик и сохранённый сырой текст.

use serde_json::{Map, Value};

use super::model::{AiIssueDto, AiTask, ParseStatus};

const MAX_SUMMARY_CHARS: usize = 20_000;
const MAX_PROMPT_CHARS: usize = 100_000;
const MAX_ITEM_CHARS: usize = 2_000;
const MAX_ITEMS: usize = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedResponse {
    pub status: ParseStatus,
    pub draft: Value,
    pub issues: Vec<AiIssueDto>,
}

/// Находит JSON-объект: весь ответ, блок ```json … ``` или первый
/// сбалансированный `{…}` (с учётом строк и экранирования).
pub fn find_json_object(text: &str) -> Option<Map<String, Value>> {
    let trimmed = text.trim();
    if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(trimmed) {
        return Some(object);
    }
    let mut search = trimmed;
    while let Some(start) = search.find("```") {
        let after = &search[start + 3..];
        let body_start = after.find('\n').map_or(0, |index| index + 1);
        let body = &after[body_start..];
        if let Some(end) = body.find("```") {
            if let Ok(Value::Object(object)) = serde_json::from_str::<Value>(body[..end].trim()) {
                return Some(object);
            }
            search = &body[end + 3..];
        } else {
            break;
        }
    }
    let bytes = trimmed.as_bytes();
    let mut attempts = 0;
    let mut index = 0;
    while index < bytes.len() && attempts < 20 {
        if bytes[index] != b'{' {
            index += 1;
            continue;
        }
        attempts += 1;
        if let Some(end) = balanced_end(&trimmed[index..]) {
            if let Ok(Value::Object(object)) =
                serde_json::from_str::<Value>(&trimmed[index..index + end])
            {
                return Some(object);
            }
        }
        index += 1;
    }
    None
}

fn balanced_end(text: &str) -> Option<usize> {
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, c) in text.char_indices() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

struct Builder {
    object: Map<String, Value>,
    draft: Map<String, Value>,
    issues: Vec<AiIssueDto>,
    missing_required: Vec<&'static str>,
    used: Vec<String>,
}

impl Builder {
    fn new(object: Map<String, Value>) -> Self {
        Self {
            object,
            draft: Map::new(),
            issues: Vec::new(),
            missing_required: Vec::new(),
            used: Vec::new(),
        }
    }

    fn take(&mut self, keys: &[&str]) -> Option<Value> {
        for key in keys {
            if let Some(value) = self.object.get(*key) {
                self.used.push((*key).to_owned());
                return Some(value.clone());
            }
        }
        None
    }

    fn text(&mut self, field: &'static str, aliases: &[&str], limit: usize, required: bool) {
        let mut keys = vec![field];
        keys.extend_from_slice(aliases);
        match self.take(&keys) {
            Some(Value::String(text)) if !text.trim().is_empty() => {
                let count = text.chars().count();
                let value = if count > limit {
                    self.issues.push(AiIssueDto::new(
                        "field_truncated",
                        format!("Поле «{field}» длиннее {limit} символов — обрезано"),
                    ));
                    text.chars().take(limit).collect()
                } else {
                    text.trim().to_owned()
                };
                self.draft.insert(field.to_owned(), Value::String(value));
            }
            Some(Value::String(_)) | None => {
                if required {
                    self.missing_required.push(field);
                }
            }
            Some(_) => {
                self.issues.push(AiIssueDto::new(
                    "invalid_field",
                    format!("Поле «{field}» должно быть строкой"),
                ));
                if required {
                    self.missing_required.push(field);
                }
            }
        }
    }

    fn choice(&mut self, field: &'static str, aliases: &[&str], allowed: &[&str], required: bool) {
        let mut keys = vec![field];
        keys.extend_from_slice(aliases);
        match self.take(&keys) {
            Some(Value::String(raw)) => {
                let normalized = raw.trim().to_lowercase().replace(['-', ' '], "_");
                if allowed.contains(&normalized.as_str()) {
                    self.draft
                        .insert(field.to_owned(), Value::String(normalized));
                } else {
                    self.issues.push(AiIssueDto::new(
                        "invalid_value",
                        format!("«{field}»: недопустимое значение «{}»", truncate(&raw, 40)),
                    ));
                    if required {
                        self.missing_required.push(field);
                    }
                }
            }
            Some(_) => {
                self.issues.push(AiIssueDto::new(
                    "invalid_value",
                    format!("«{field}» должно быть строкой"),
                ));
                if required {
                    self.missing_required.push(field);
                }
            }
            None => {
                if required {
                    self.missing_required.push(field);
                }
            }
        }
    }

    fn style_score(&mut self) {
        match self.take(&["styleScore", "style_score"]) {
            None | Some(Value::Null) => {
                self.draft.insert("styleScore".to_owned(), Value::Null);
            }
            Some(Value::Number(number)) => match number.as_f64() {
                Some(value) if (0.0..=10.0).contains(&value) => {
                    self.draft
                        .insert("styleScore".to_owned(), Value::from(value.round() as u8));
                }
                _ => {
                    self.issues.push(AiIssueDto::new(
                        "invalid_value",
                        "Расхождение со стилем должно быть числом 0–10 — поле оставлено пустым",
                    ));
                    self.draft.insert("styleScore".to_owned(), Value::Null);
                }
            },
            Some(_) => {
                self.issues.push(AiIssueDto::new(
                    "invalid_value",
                    "Расхождение со стилем должно быть числом 0–10 — поле оставлено пустым",
                ));
                self.draft.insert("styleScore".to_owned(), Value::Null);
            }
        }
    }

    /// Список замечаний: объекты `{text, location?, criterion?}` или строки.
    fn findings(&mut self, field: &'static str, default_criterion: &str) {
        let Some(value) = self.take(&[field]) else {
            self.draft
                .insert(field.to_owned(), Value::Array(Vec::new()));
            return;
        };
        let Value::Array(items) = value else {
            self.issues.push(AiIssueDto::new(
                "invalid_field",
                format!("«{field}» должно быть списком"),
            ));
            self.draft
                .insert(field.to_owned(), Value::Array(Vec::new()));
            return;
        };
        let criteria = [
            "correctness",
            "task_fit",
            "course_grounding",
            "style",
            "teacher",
            "completeness",
        ];
        let mut output = Vec::new();
        for item in items.into_iter().take(MAX_ITEMS) {
            let (text, location, criterion) = match item {
                Value::String(text) => (text, None, None),
                Value::Object(object) => (
                    object
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    object
                        .get("location")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    object
                        .get("criterion")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                ),
                _ => (String::new(), None, None),
            };
            if text.trim().is_empty() {
                self.issues.push(AiIssueDto::new(
                    "invalid_item",
                    format!("Пустой элемент в «{field}» пропущен"),
                ));
                continue;
            }
            let criterion = criterion
                .map(|value| value.trim().to_lowercase())
                .filter(|value| criteria.contains(&value.as_str()))
                .unwrap_or_else(|| default_criterion.to_owned());
            let mut entry = Map::new();
            entry.insert("criterion".into(), Value::String(criterion));
            entry.insert(
                "text".into(),
                Value::String(truncate(&text, MAX_ITEM_CHARS)),
            );
            entry.insert(
                "location".into(),
                location
                    .filter(|value| !value.trim().is_empty())
                    .map_or(Value::Null, |value| Value::String(truncate(&value, 200))),
            );
            output.push(Value::Object(entry));
        }
        self.draft.insert(field.to_owned(), Value::Array(output));
    }

    fn claims(&mut self) {
        let Some(Value::Array(items)) = self.take(&["unsupportedClaims", "unsupported_claims"])
        else {
            self.draft
                .insert("unsupportedClaims".into(), Value::Array(Vec::new()));
            return;
        };
        let mut output = Vec::new();
        for item in items.into_iter().take(MAX_ITEMS) {
            let (text, location) = match item {
                Value::String(text) => (text, String::new()),
                Value::Object(object) => (
                    object
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    object
                        .get("location")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                ),
                _ => continue,
            };
            if text.trim().is_empty() {
                continue;
            }
            let mut entry = Map::new();
            entry.insert(
                "text".into(),
                Value::String(truncate(&text, MAX_ITEM_CHARS)),
            );
            entry.insert(
                "location".into(),
                Value::String(if location.trim().is_empty() {
                    "место не указано".to_owned()
                } else {
                    truncate(&location, 200)
                }),
            );
            output.push(Value::Object(entry));
        }
        self.draft
            .insert("unsupportedClaims".into(), Value::Array(output));
    }

    fn missing_items(&mut self) {
        let Some(Value::Array(items)) = self.take(&["missing"]) else {
            self.draft
                .insert("missing".into(), Value::Array(Vec::new()));
            return;
        };
        let mut output = Vec::new();
        for item in items.into_iter().take(MAX_ITEMS) {
            let (label, kind, required) = match item {
                Value::String(label) => (label, None, true),
                Value::Object(object) => (
                    object
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    object
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    object
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                ),
                _ => continue,
            };
            if label.trim().is_empty() {
                continue;
            }
            let kind = match kind.as_deref() {
                Some("link") => "link",
                Some("file") => "file",
                _ if label.contains("http") || label.to_lowercase().contains("ссылк") => {
                    "link"
                }
                _ => "file",
            };
            let mut entry = Map::new();
            entry.insert("kind".into(), Value::String(kind.to_owned()));
            entry.insert("label".into(), Value::String(truncate(&label, 300)));
            entry.insert("required".into(), Value::Bool(required));
            output.push(Value::Object(entry));
        }
        self.draft.insert("missing".into(), Value::Array(output));
    }

    fn finish(mut self, truncated: bool) -> ParsedResponse {
        let unknown: Vec<String> = self
            .object
            .keys()
            .filter(|key| !self.used.contains(key))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            self.issues.push(AiIssueDto::new(
                "unknown_fields",
                format!("Лишние поля проигнорированы: {}", unknown.join(", ")),
            ));
        }
        if truncated {
            self.issues.push(AiIssueDto::new(
                "response_truncated",
                "Ответ был обрезан по лимиту объёма — проверьте полноту",
            ));
        }
        for field in &self.missing_required {
            self.issues.push(AiIssueDto::new(
                "missing_field",
                format!("Нет обязательного поля «{field}» — заполните его вручную"),
            ));
        }
        let recognized = self.draft.values().any(|value| match value {
            Value::String(text) => !text.is_empty(),
            Value::Array(items) => !items.is_empty(),
            Value::Null => false,
            _ => true,
        });
        let status = if !recognized {
            ParseStatus::Corrupt
        } else if self.missing_required.is_empty() && self.issues.is_empty() {
            ParseStatus::Valid
        } else {
            ParseStatus::Partial
        };
        ParsedResponse {
            status,
            draft: Value::Object(self.draft),
            issues: self.issues,
        }
    }
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

const VERDICTS: &[&str] = &["pass", "needs_fixes", "needs_teacher_review", "blocked"];
const STATUSES: &[&str] = &["pass", "warn", "fail", "manual", "na"];

/// Разбирает ответ задачи. `expects_prompt` — анализ материалов просил
/// провайдера сразу подготовить и первичный промт.
pub fn parse_response(
    task: AiTask,
    raw: &str,
    truncated: bool,
    expects_prompt: bool,
) -> ParsedResponse {
    if raw.trim().is_empty() {
        return ParsedResponse {
            status: ParseStatus::Empty,
            draft: Value::Object(Map::new()),
            issues: vec![AiIssueDto::new("empty_response", "Ответ пустой")],
        };
    }
    let Some(object) = find_json_object(raw) else {
        // Промт исправлений и первичный промт допустимо принять обычным текстом.
        if matches!(task, AiTask::InitialPrompt | AiTask::RevisionPrompt) {
            let field = if task == AiTask::InitialPrompt {
                "promptText"
            } else {
                "text"
            };
            let mut draft = Map::new();
            draft.insert(
                field.into(),
                Value::String(truncate(raw.trim(), MAX_PROMPT_CHARS)),
            );
            return ParsedResponse {
                status: ParseStatus::Partial,
                draft: Value::Object(draft),
                issues: vec![AiIssueDto::new(
                    "plain_text",
                    "Ответ без JSON — весь текст взят как промт; проверьте его",
                )],
            };
        }
        return ParsedResponse {
            status: ParseStatus::Corrupt,
            draft: Value::Object(Map::new()),
            issues: vec![AiIssueDto::new(
                "no_json",
                "В ответе нет JSON-объекта нужного формата — сырой текст сохранён, заполните результат вручную",
            )],
        };
    };
    let mut builder = Builder::new(object);
    match task {
        AiTask::MaterialAnalysis => {
            builder.text("summary", &["analysis"], MAX_SUMMARY_CHARS, true);
            builder.text(
                "promptText",
                &["prompt_text", "prompt"],
                MAX_PROMPT_CHARS,
                expects_prompt,
            );
        }
        AiTask::InitialPrompt => {
            builder.text(
                "promptText",
                &["prompt_text", "prompt", "text"],
                MAX_PROMPT_CHARS,
                true,
            );
        }
        AiTask::RevisionPrompt => {
            builder.text(
                "text",
                &["promptText", "prompt_text", "prompt"],
                MAX_PROMPT_CHARS,
                true,
            );
        }
        AiTask::SolutionReview | AiTask::TeacherReview => {
            builder.choice("verdict", &[], VERDICTS, true);
            builder.text("summary", &[], MAX_SUMMARY_CHARS, true);
            builder.choice("correctness", &[], STATUSES, true);
            builder.choice("taskFit", &["task_fit"], STATUSES, true);
            builder.choice("courseGrounding", &["course_grounding"], STATUSES, true);
            builder.choice("teacher", &[], STATUSES, true);
            builder.style_score();
            builder.findings("critical", "correctness");
            builder.findings("remarks", "task_fit");
            builder.claims();
            builder.missing_items();
        }
    }
    builder.finish(truncated)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{find_json_object, parse_response};
    use crate::ai::model::{AiTask, ParseStatus};

    #[test]
    fn finds_json_in_fences_and_prose() {
        assert!(find_json_object("```json\n{\"a\": 1}\n```").is_some());
        assert!(find_json_object("Вот ответ: {\"a\": \"}{\"} — готово").is_some());
        assert!(find_json_object("нет json").is_none());
        assert!(find_json_object("{\"a\": ").is_none());
    }

    #[test]
    fn valid_review_becomes_a_complete_draft() {
        let raw = json!({
            "verdict": "needs_fixes",
            "summary": "Ошибка в п. 2",
            "correctness": "fail",
            "taskFit": "pass",
            "courseGrounding": "warn",
            "teacher": "na",
            "styleScore": 6,
            "critical": [{"text": "Неверная формула NPV", "location": "Лист 2", "criterion": "correctness"}],
            "remarks": ["Нет вывода"],
            "unsupportedClaims": [{"text": "Рынок вырастет на 50%", "location": "п. 3"}],
            "missing": ["Ссылка на Colab"]
        })
        .to_string();
        let parsed = parse_response(AiTask::SolutionReview, &raw, false, false);
        assert_eq!(parsed.status, ParseStatus::Valid, "{:?}", parsed.issues);
        assert_eq!(parsed.draft["styleScore"], 6);
        assert_eq!(parsed.draft["remarks"][0]["criterion"], "task_fit");
        assert_eq!(parsed.draft["missing"][0]["kind"], "link");
    }

    #[test]
    fn partial_and_corrupt_responses_are_reported() {
        let partial = parse_response(
            AiTask::SolutionReview,
            "{\"verdict\": \"maybe\", \"summary\": \"кратко\", \"styleScore\": 42, \"extra\": 1}",
            true,
            false,
        );
        assert_eq!(partial.status, ParseStatus::Partial);
        let codes: Vec<&str> = partial
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect();
        assert!(codes.contains(&"invalid_value"));
        assert!(codes.contains(&"missing_field"));
        assert!(codes.contains(&"unknown_fields"));
        assert!(codes.contains(&"response_truncated"));
        assert_eq!(partial.draft["styleScore"], serde_json::Value::Null);
        assert!(partial.draft.get("verdict").is_none());

        let corrupt = parse_response(AiTask::SolutionReview, "Извините, не могу", false, false);
        assert_eq!(corrupt.status, ParseStatus::Corrupt);
        let empty = parse_response(AiTask::MaterialAnalysis, "  ", false, true);
        assert_eq!(empty.status, ParseStatus::Empty);
        let wrong_shape = parse_response(AiTask::MaterialAnalysis, "{\"foo\": 1}", false, true);
        assert_eq!(wrong_shape.status, ParseStatus::Corrupt);
    }

    #[test]
    fn analysis_requires_prompt_only_when_requested() {
        let raw = "{\"summary\": \"Итог\"}";
        assert_eq!(
            parse_response(AiTask::MaterialAnalysis, raw, false, false).status,
            ParseStatus::Valid
        );
        assert_eq!(
            parse_response(AiTask::MaterialAnalysis, raw, false, true).status,
            ParseStatus::Partial
        );
        let plain = parse_response(AiTask::RevisionPrompt, "Исправь пункт 2", false, false);
        assert_eq!(plain.status, ParseStatus::Partial);
        assert_eq!(plain.draft["text"], "Исправь пункт 2");
    }
}
