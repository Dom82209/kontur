use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::{dto::AutomationRulesDto, error::BackendError};

use super::ReadRepository;

impl ReadRepository<'_> {
    pub fn automation_rules(&self) -> Result<AutomationRulesDto, BackendError> {
        self.database.with_connection(load_automation_rules)
    }
}

/// Правила автоматизации с безопасными defaults (используется и внутри транзакций).
pub(crate) fn load_automation_rules(
    connection: &rusqlite::Connection,
) -> Result<AutomationRulesDto, BackendError> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT value_json FROM app_settings
                     WHERE key IN ('automation_rules', 'automation')
                     ORDER BY CASE key WHEN 'automation_rules' THEN 0 ELSE 1 END
                     LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let mut defaults = AutomationRulesDto::default();
    let Some(raw) = stored else {
        // Ключевые слова предметов есть и без сохранённых правил.
        defaults.subject_rules = active_subject_rules(connection)?;
        return Ok(defaults);
    };
    let mut saved: Value = serde_json::from_str(&raw).map_err(|error| {
        BackendError::database(format!("invalid automation rules JSON: {error}"))
    })?;
    strip_retired_keys(&mut saved);
    if contains_secret_key(&saved) {
        return Err(BackendError::database(
            "stored automation rules contain a forbidden secret key",
        ));
    }
    let saved_tasks = saved.pointer("/workflow/tasks").is_some();
    let mut merged = serde_json::to_value(defaults)
        .map_err(|error| BackendError::database(error.to_string()))?;
    merge_json(&mut merged, saved);
    let mut rules: AutomationRulesDto = serde_json::from_value(merged)
        .map_err(|error| BackendError::database(format!("invalid automation rules: {error}")))?;
    if !saved_tasks {
        // Настройки до этапа 9: провайдеры задач берутся из прежних ролей.
        rules.workflow.tasks = crate::dto::AiTaskProvidersDto::from_roles(&rules.workflow.roles);
    }
    // Ключевые слова живут в `subject_keywords` (этап 3); JSON-копия не используется.
    rules.subject_rules = active_subject_rules(connection)?;
    // Производные значения и инварианты всегда берутся из контракта, а не из JSON.
    rules.inbox_path = AutomationRulesDto::default().inbox_path;
    rules.confirm = AutomationRulesDto::default().confirm;
    validate_rules(&rules)?;
    Ok(rules)
}

/// Поля, которые до этапа 6 сохранялись, но не исполнялись backend: папки
/// (теперь — управляемые каталоги), начало семестра и способ определения недели
/// (теперь — семестр), шаблон версии, резервная копия и автораскладка.
const RETIRED_KEYS: &[&str] = &[
    "downloadsWatchPath",
    "obsidianVaultPath",
    "versionPattern",
    "weekDetection",
    "semesterStart",
    "backupBeforeChange",
    "autoRouteHighConfidence",
];

fn strip_retired_keys(saved: &mut Value) {
    if let Value::Object(object) = saved {
        for key in RETIRED_KEYS {
            object.remove(*key);
        }
    }
}

fn active_subject_rules(
    connection: &rusqlite::Connection,
) -> Result<Vec<crate::dto::SubjectRuleDto>, BackendError> {
    let mut keywords = super::subjects::subject_keywords(connection)?;
    let mut statement = connection.prepare(
        "SELECT id FROM subjects WHERE archived_at IS NULL ORDER BY name COLLATE NOCASE, id",
    )?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids
        .into_iter()
        .filter_map(|subject_id| {
            let keywords = keywords.remove(&subject_id)?;
            Some(crate::dto::SubjectRuleDto {
                keywords,
                subject_id,
            })
        })
        .collect())
}

fn merge_json(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                if let Some(target) = base.get_mut(&key) {
                    merge_json(target, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

pub(crate) fn validate_rules(rules: &AutomationRulesDto) -> Result<(), BackendError> {
    if !(0.0..=1.0).contains(&rules.low_confidence_threshold) {
        return Err(BackendError::database(
            "automation lowConfidenceThreshold must be between 0 and 1",
        ));
    }
    if !(0..=10).contains(&rules.workflow.style_threshold) {
        return Err(BackendError::database(
            "automation styleThreshold must be between 0 and 10",
        ));
    }
    let confirm = &rules.confirm;
    if !confirm.mark_submitted
        || confirm.allow_overwrite
        || !confirm.low_confidence_route
        || !confirm.mark_final
    {
        return Err(BackendError::database(
            "automation confirmation invariants are invalid",
        ));
    }
    Ok(())
}

/// Проверки при сохранении: то, что пользователь вводит руками. При чтении
/// старые значения не ломают загрузку — их заблокирует место использования.
pub(crate) fn validate_rules_for_save(rules: &AutomationRulesDto) -> Result<(), BackendError> {
    validate_rules(rules)?;
    if rules.inbox_path != AutomationRulesDto::default().inbox_path {
        return Err(BackendError::conflict(
            "Папка «Входящих» фиксирована и не задаётся правилами",
        ));
    }
    crate::naming::validate_pattern(&rules.final_name_pattern)?;
    for part in [
        &rules.student_name.last_name,
        &rules.student_name.first_name,
    ] {
        crate::naming::validate_name_part(part)?;
    }
    Ok(())
}

pub(crate) fn contains_secret_key(value: &Value) -> bool {
    const FORBIDDEN: &[&str] = &[
        "apikey",
        "password",
        "cookie",
        "accesstoken",
        "refreshtoken",
        "sessionsecret",
    ];
    match value {
        Value::Object(object) => object.iter().any(|(key, nested)| {
            let normalized: String = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect();
            FORBIDDEN
                .iter()
                .any(|fragment| normalized.contains(fragment))
                || contains_secret_key(nested)
        }),
        Value::Array(items) => items.iter().any(contains_secret_key),
        _ => false,
    }
}
