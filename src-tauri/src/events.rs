use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::{
    domain::HomeworkWorkflowStage,
    dto::{
        ClassificationDto, HomeworkVersionDto, InboxFileDto, InboxStatus, OperationDto,
        OperationKind,
    },
    error::{AppError, ErrorCode},
    workflow::dto::{ReviewRunDto, SubmissionReceiptDto},
};

pub mod names {
    pub const INBOX_FILE_ADDED: &str = "inbox:file-added";
    pub const INBOX_CLASSIFICATION_UPDATED: &str = "inbox:classification-updated";
    pub const OPERATION_PROGRESS: &str = "operation:progress";
    pub const OPERATION_COMPLETED: &str = "operation:completed";
    pub const OPERATION_FAILED: &str = "operation:failed";
    pub const PROVIDER_STATUS_CHANGED: &str = "provider:status-changed";
    pub const HOMEWORK_VERSION_CREATED: &str = "homework:version-created";
    pub const WORKFLOW_STAGE_CHANGED: &str = "workflow:stage-changed";
    pub const REVIEW_PROGRESS: &str = "review:progress";
    pub const REVIEW_COMPLETED: &str = "review:completed";
    pub const REVIEW_FAILED: &str = "review:failed";
    pub const SUBMISSION_CONFIRMED: &str = "submission:confirmed";
    pub const INBOX_UPDATED: &str = "inbox:updated";
    /// Этап 6: состояние watcher загрузок и список кандидатов изменились.
    pub const DOWNLOADS_WATCHER_UPDATED: &str = "downloads:watcher-updated";
    /// Этап 6: фаза и прогресс импорта экспорта ЦУ.
    pub const CU_IMPORT_PROGRESS: &str = "cu-import:progress";
    /// Этап 7: прогресс пакетного извлечения текста.
    pub const EXTRACTION_PROGRESS: &str = "extraction:progress";
    /// Этап 7: результаты извлечения изменились (анализ, перестроение, очистка cache).
    pub const EXTRACTION_UPDATED: &str = "extraction:updated";
    /// Этап 8: агрегат ДЗ изменился (новая ревизия) — клиент перечитывает `get_workflow`.
    pub const HOMEWORK_UPDATED: &str = "homework:updated";
    /// Этап 8: создан черновик промта (первичного или исправлений).
    pub const HOMEWORK_REVISION_PROMPT_CREATED: &str = "homework:revision-prompt-created";
    /// Этап 8: готовность к сдаче пересчитана после мутации.
    pub const HOMEWORK_READINESS_UPDATED: &str = "homework:readiness-updated";

    /// Этап 9: прогресс задания передачи провайдеру (фаза, время, объём вывода).
    pub const AI_JOB_PROGRESS: &str = "ai-job:progress";
    /// Этап 9: задание изменилось (ответ получен, ошибка, отмена) — перечитать.
    pub const AI_JOB_UPDATED: &str = "ai-job:updated";

    /// Служебные события native intake (не входят в `BridgeEvents`, слушает только Tauri adapter).
    pub const INTAKE_NATIVE_DROP: &str = "intake:native-drop";
    pub const INTAKE_DRAG_STATE: &str = "intake:drag-state";

    pub const ALL: [&str; 22] = [
        INBOX_FILE_ADDED,
        INBOX_CLASSIFICATION_UPDATED,
        OPERATION_PROGRESS,
        OPERATION_COMPLETED,
        OPERATION_FAILED,
        PROVIDER_STATUS_CHANGED,
        HOMEWORK_VERSION_CREATED,
        WORKFLOW_STAGE_CHANGED,
        REVIEW_PROGRESS,
        REVIEW_COMPLETED,
        REVIEW_FAILED,
        SUBMISSION_CONFIRMED,
        INBOX_UPDATED,
        DOWNLOADS_WATCHER_UPDATED,
        CU_IMPORT_PROGRESS,
        EXTRACTION_PROGRESS,
        EXTRACTION_UPDATED,
        HOMEWORK_UPDATED,
        HOMEWORK_REVISION_PROMPT_CREATED,
        HOMEWORK_READINESS_UPDATED,
        AI_JOB_PROGRESS,
        AI_JOB_UPDATED,
    ];
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsWatcherUpdatedPayload {
    pub watcher: crate::lms::DownloadsWatcherDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CuImportProgressPayload {
    pub operation_id: String,
    pub phase: crate::lms::ExportPhase,
    pub completed: usize,
    pub total: usize,
    pub entry_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionProgressPayload {
    pub operation_id: String,
    pub completed: usize,
    pub total: usize,
    pub material: Option<crate::extraction::MaterialRef>,
    pub name: Option<String>,
    pub outcome: Option<crate::extraction::BatchItemOutcome>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionUpdateReason {
    Analyzed,
    Cleared,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionUpdatedPayload {
    pub reason: ExtractionUpdateReason,
    pub materials: Vec<crate::extraction::MaterialRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxFileAddedPayload {
    pub files: Vec<InboxFileDto>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxUpdateReason {
    Import,
}

/// Список «Входящих» изменился; клиент перечитывает `get_inbox_files`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxUpdatedPayload {
    pub reason: InboxUpdateReason,
    pub file_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeDropPayload {
    pub items: Vec<crate::intake::DroppedItemDto>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DragState {
    Enter,
    Leave,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragStatePayload {
    pub state: DragState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxClassificationUpdatedPayload {
    pub file_id: String,
    pub classification: ClassificationDto,
    pub status: InboxStatus,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgressPayload {
    pub operation_id: String,
    pub kind: OperationKind,
    pub progress: f64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationCompletedPayload {
    pub operation: OperationDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationFailedPayload {
    pub operation: OperationDto,
    pub error: AppError,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusChangedPayload {
    pub status: crate::workflow::dto::ProviderStatusDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkVersionCreatedPayload {
    pub homework_id: String,
    pub version: HomeworkVersionDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStageChangedPayload {
    pub homework_id: String,
    pub from: HomeworkWorkflowStage,
    pub to: HomeworkWorkflowStage,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewProgressPayload {
    pub run_id: String,
    pub homework_id: String,
    pub progress: f64,
    pub step: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCompletedPayload {
    pub run: ReviewRunDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFailedPayload {
    pub run: ReviewRunDto,
    pub error: AppError,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionConfirmedPayload {
    pub receipt: SubmissionReceiptDto,
}

pub fn emit_bridge_event<R, T>(
    app: &AppHandle<R>,
    event: &'static str,
    payload: T,
) -> Result<(), AppError>
where
    R: Runtime,
    T: Serialize + Clone,
{
    app.emit(event, payload).map_err(|_| emit_failure())
}

fn emit_failure() -> AppError {
    AppError::new(ErrorCode::Unknown, "Не удалось отправить Tauri event")
}

#[derive(Default)]
pub struct EventRegistry;

impl EventRegistry {
    pub fn registered_services(&self) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        domain::OperationStatus,
        dto::{OperationDto, OperationKind},
        error::{AppError, ErrorCode},
    };

    use super::{
        emit_failure, names, OperationCompletedPayload, OperationFailedPayload,
        OperationProgressPayload,
    };

    fn operation() -> OperationDto {
        OperationDto {
            id: "operation-1".to_owned(),
            kind: OperationKind::Route,
            title: "Раскладка".to_owned(),
            detail: None,
            status: OperationStatus::Completed,
            started_at: "2026-10-21T10:00:00Z".to_owned(),
            finished_at: Some("2026-10-21T10:01:00Z".to_owned()),
            undoable: true,
            paths: vec!["Предметы/ОБА/Неделя 07".to_owned()],
            backup_path: Some("Резервные копии/operation-1".to_owned()),
            homework_id: Some("homework-7".to_owned()),
        }
    }

    #[test]
    fn event_names_match_frontend_contract_exactly() {
        assert_eq!(
            names::ALL,
            [
                "inbox:file-added",
                "inbox:classification-updated",
                "operation:progress",
                "operation:completed",
                "operation:failed",
                "provider:status-changed",
                "homework:version-created",
                "workflow:stage-changed",
                "review:progress",
                "review:completed",
                "review:failed",
                "submission:confirmed",
                "inbox:updated",
                "downloads:watcher-updated",
                "cu-import:progress",
                "extraction:progress",
                "extraction:updated",
                "homework:updated",
                "homework:revision-prompt-created",
                "homework:readiness-updated",
                "ai-job:progress",
                "ai-job:updated",
            ]
        );
        assert_eq!(names::INTAKE_NATIVE_DROP, "intake:native-drop");
        assert_eq!(names::INTAKE_DRAG_STATE, "intake:drag-state");
    }

    #[test]
    fn inbox_updated_payload_is_camel_case() {
        let value = serde_json::to_value(super::InboxUpdatedPayload {
            reason: super::InboxUpdateReason::Import,
            file_ids: vec!["f1".to_owned()],
        })
        .unwrap();
        assert_eq!(value, json!({ "reason": "import", "fileIds": ["f1"] }));
    }

    #[test]
    fn payloads_serialize_in_camel_case() {
        let value = serde_json::to_value(OperationProgressPayload {
            operation_id: "operation-1".to_owned(),
            kind: OperationKind::Route,
            progress: 0.5,
            message: "Копирование".to_owned(),
        })
        .unwrap();
        assert_eq!(value["operationId"], "operation-1");
        assert_eq!(value["kind"], "route");
        assert_eq!(value["progress"], 0.5);
        assert!(value.get("operation_id").is_none());
    }

    #[test]
    fn nested_app_error_keeps_structured_code_and_message() {
        let value = serde_json::to_value(OperationFailedPayload {
            operation: operation(),
            error: AppError::new(ErrorCode::Database, "SQLite недоступна"),
        })
        .unwrap();
        assert_eq!(
            value["error"],
            json!({ "code": "database", "message": "SQLite недоступна" })
        );
    }

    #[test]
    fn operation_event_reuses_existing_dto_shape() {
        let operation = operation();
        let direct = serde_json::to_value(&operation).unwrap();
        let wrapped = serde_json::to_value(OperationCompletedPayload { operation }).unwrap();
        assert_eq!(wrapped["operation"], direct);
        assert_eq!(wrapped["operation"]["startedAt"], "2026-10-21T10:00:00Z");
    }

    #[test]
    fn test_payloads_contain_only_relative_paths() {
        let value = serde_json::to_value(OperationCompletedPayload {
            operation: operation(),
        })
        .unwrap();
        let serialized = value.to_string();
        assert!(!serialized.contains("/Users/"));
        assert!(!serialized.contains("file://"));
    }

    #[test]
    fn emit_failure_is_a_structured_app_error() {
        let error = emit_failure();
        assert_eq!(error.code, ErrorCode::Unknown);
        assert_eq!(error.message, "Не удалось отправить Tauri event");
    }
}
