use rusqlite::{params, OptionalExtension, Transaction};

use crate::{
    dto::{AutomationRulesDto, ClassificationDto, InboxStatus, OperationKind},
    error::{AppError, BackendError, ErrorCode},
};

use super::{
    parse_json,
    settings::{contains_secret_key, validate_rules},
};

pub struct InboxClassificationRow {
    pub status: InboxStatus,
    pub classification: Option<ClassificationDto>,
}

pub fn upsert_automation_rules(
    transaction: &Transaction<'_>,
    rules: &AutomationRulesDto,
) -> Result<(), BackendError> {
    validate_rules(rules)?;
    // Ключевые слова предметов — в `subject_keywords`; JSON их не хранит.
    for rule in &rules.subject_rules {
        crate::subjects::replace_keywords(transaction, &rule.subject_id, &rule.keywords)?;
    }
    let mut stored = rules.clone();
    stored.subject_rules.clear();
    let value =
        serde_json::to_value(&stored).map_err(|error| BackendError::database(error.to_string()))?;
    if contains_secret_key(&value) {
        return Err(BackendError::conflict(
            "automation rules contain a forbidden secret field",
        ));
    }
    let value_json =
        serde_json::to_string(&value).map_err(|error| BackendError::database(error.to_string()))?;
    transaction.execute(
        "INSERT INTO app_settings(key, value_json, created_at, updated_at)
         VALUES ('automation_rules', ?1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
         ON CONFLICT(key) DO UPDATE SET
           value_json = excluded.value_json,
           updated_at = CURRENT_TIMESTAMP",
        [value_json],
    )?;
    Ok(())
}

pub fn read_inbox_classification(
    transaction: &Transaction<'_>,
    file_id: &str,
) -> Result<InboxClassificationRow, BackendError> {
    transaction
        .query_row(
            "SELECT status, classification_json FROM inbox_files WHERE id = ?1",
            [file_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .ok_or_else(|| BackendError::not_found("inbox file not found"))
        .and_then(|(status, classification_json)| {
            let status = InboxStatus::try_from_db(&status).map_err(BackendError::database)?;
            let classification = classification_json
                .as_deref()
                .map(|raw| parse_json(raw, "inbox_files.classification_json"))
                .transpose()?;
            Ok(InboxClassificationRow {
                status,
                classification,
            })
        })
}

pub fn subject_exists(
    transaction: &Transaction<'_>,
    subject_id: &str,
) -> Result<bool, BackendError> {
    Ok(transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM subjects WHERE id = ?1)",
        [subject_id],
        |row| row.get(0),
    )?)
}

pub fn subject_archived(
    transaction: &Transaction<'_>,
    subject_id: &str,
) -> Result<bool, BackendError> {
    Ok(transaction.query_row(
        "SELECT archived_at IS NOT NULL FROM subjects WHERE id = ?1",
        [subject_id],
        |row| row.get(0),
    )?)
}

pub fn week_exists_for_subject(
    transaction: &Transaction<'_>,
    subject_id: &str,
    week_number: i64,
) -> Result<bool, BackendError> {
    Ok(transaction.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM weeks WHERE subject_id = ?1 AND number = ?2
         )",
        params![subject_id, week_number],
        |row| row.get(0),
    )?)
}

pub fn update_inbox_classification(
    transaction: &Transaction<'_>,
    file_id: &str,
    classification: &ClassificationDto,
    status: InboxStatus,
) -> Result<(), BackendError> {
    let classification_json = serde_json::to_string(classification)
        .map_err(|error| BackendError::database(error.to_string()))?;
    let changed = transaction.execute(
        "UPDATE inbox_files
         SET classification_json = ?2, status = ?3, updated_at = CURRENT_TIMESTAMP
         WHERE id = ?1",
        params![file_id, classification_json, inbox_status_str(status)],
    )?;
    if changed == 0 {
        return Err(BackendError::not_found("inbox file not found"));
    }
    Ok(())
}

pub fn insert_running_operation(
    transaction: &Transaction<'_>,
    operation_id: &str,
    kind: OperationKind,
    title: &str,
) -> Result<(), BackendError> {
    transaction.execute(
        "INSERT INTO operations(
           id, kind, status, progress, undoable, started_at, title, detail, paths_json
         ) VALUES (
           ?1, ?2, 'running', 0, 0,
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?3, NULL, '[]'
         )",
        params![operation_id, kind.as_db_str(), title],
    )?;
    Ok(())
}

pub fn complete_operation(
    transaction: &Transaction<'_>,
    operation_id: &str,
) -> Result<(), BackendError> {
    let changed = transaction.execute(
        "UPDATE operations
         SET status = 'completed', progress = 100,
             completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?1 AND status = 'running'",
        [operation_id],
    )?;
    if changed != 1 {
        return Err(BackendError::database(
            "operation lifecycle could not be completed",
        ));
    }
    Ok(())
}

pub fn insert_failed_operation(
    transaction: &Transaction<'_>,
    operation_id: &str,
    kind: OperationKind,
    title: &str,
    error: &AppError,
) -> Result<(), BackendError> {
    let error_json = serde_json::to_string(&SafeOperationError {
        code: error.code,
        message: "Операция не выполнена",
    })
    .map_err(|serialization_error| BackendError::database(serialization_error.to_string()))?;
    transaction.execute(
        "INSERT INTO operations(
           id, kind, status, progress, undoable, started_at, completed_at,
           title, detail, paths_json, error_json
         ) VALUES (
           ?1, ?2, 'failed', 0, 0,
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
           strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
           ?3, 'Операция не выполнена', '[]', ?4
         )",
        params![operation_id, kind.as_db_str(), title, error_json],
    )?;
    Ok(())
}

#[derive(serde::Serialize)]
struct SafeOperationError<'a> {
    code: ErrorCode,
    message: &'a str,
}

fn inbox_status_str(status: InboxStatus) -> &'static str {
    match status {
        InboxStatus::New => "new",
        InboxStatus::Classifying => "classifying",
        InboxStatus::Classified => "classified",
        InboxStatus::Ambiguous => "ambiguous",
        InboxStatus::Routed => "routed",
    }
}
