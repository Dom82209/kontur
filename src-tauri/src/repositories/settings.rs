use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::{dto::AutomationRulesDto, error::BackendError};

use super::ReadRepository;

impl ReadRepository<'_> {
    pub fn automation_rules(&self) -> Result<AutomationRulesDto, BackendError> {
        self.database.with_connection(|connection| {
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
            let defaults = AutomationRulesDto::default();
            let Some(raw) = stored else {
                return Ok(defaults);
            };
            let saved: Value = serde_json::from_str(&raw).map_err(|error| {
                BackendError::database(format!("invalid automation rules JSON: {error}"))
            })?;
            if contains_secret_key(&saved) {
                return Err(BackendError::database(
                    "stored automation rules contain a forbidden secret key",
                ));
            }
            let mut merged = serde_json::to_value(defaults)
                .map_err(|error| BackendError::database(error.to_string()))?;
            merge_json(&mut merged, saved);
            let rules: AutomationRulesDto = serde_json::from_value(merged).map_err(|error| {
                BackendError::database(format!("invalid automation rules: {error}"))
            })?;
            validate_rules(&rules)?;
            Ok(rules)
        })
    }
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

fn validate_rules(rules: &AutomationRulesDto) -> Result<(), BackendError> {
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
    if !rules.confirm.mark_submitted || rules.confirm.allow_overwrite {
        return Err(BackendError::database(
            "automation confirmation invariants are invalid",
        ));
    }
    Ok(())
}

fn contains_secret_key(value: &Value) -> bool {
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
