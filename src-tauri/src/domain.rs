use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subject {
    pub id: String,
    pub name: String,
    pub short_name: String,
    #[serde(skip)]
    pub folder_path: String,
}

#[derive(Debug, Clone)]
pub struct NewSubject<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub short_name: &'a str,
    pub folder_path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Week {
    pub id: String,
    pub subject_id: String,
    pub number: i64,
    pub starts_at: String,
    pub ends_at: String,
    pub topic: Option<String>,
    #[serde(skip)]
    pub folder_path: String,
}

#[derive(Debug, Clone)]
pub struct NewWeek<'a> {
    pub id: &'a str,
    pub subject_id: &'a str,
    pub number: i64,
    pub starts_at: &'a str,
    pub ends_at: &'a str,
    pub topic: Option<&'a str>,
    pub folder_path: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeworkStage {
    Task,
    InProgress,
    Ready,
    Submitted,
}

impl HomeworkStage {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::InProgress => "in_progress",
            Self::Ready => "ready_to_submit",
            Self::Submitted => "submitted",
        }
    }

    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "task" => Ok(Self::Task),
            "in_progress" => Ok(Self::InProgress),
            "ready_to_submit" => Ok(Self::Ready),
            "submitted" => Ok(Self::Submitted),
            _ => Err(format!("unknown database homework stage: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeworkWorkflowStage {
    MaterialsCollected,
    CodexAnalysis,
    PromptReady,
    ClaudeSolving,
    CodexReview,
    NeedsFixes,
    TeacherReview,
    ReadyToSubmit,
    Submitted,
}

impl HomeworkWorkflowStage {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::MaterialsCollected => "materials",
            Self::CodexAnalysis => "analysis",
            Self::PromptReady => "prompt",
            Self::ClaudeSolving => "solution",
            Self::CodexReview => "review",
            Self::NeedsFixes => "revision",
            Self::TeacherReview => "teacher",
            Self::ReadyToSubmit => "ready_to_submit",
            Self::Submitted => "submitted",
        }
    }

    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "materials" => Ok(Self::MaterialsCollected),
            "analysis" => Ok(Self::CodexAnalysis),
            "prompt" => Ok(Self::PromptReady),
            "solution" => Ok(Self::ClaudeSolving),
            "review" => Ok(Self::CodexReview),
            "revision" => Ok(Self::NeedsFixes),
            "teacher" => Ok(Self::TeacherReview),
            "ready_to_submit" => Ok(Self::ReadyToSubmit),
            "submitted" => Ok(Self::Submitted),
            _ => Err(format!("unknown database workflow stage: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Homework {
    pub id: String,
    #[serde(skip)]
    pub week_id: String,
    pub title: String,
    pub deadline: Option<String>,
    pub stage: HomeworkStage,
    pub workflow_stage: HomeworkWorkflowStage,
    pub final_version_id: Option<String>,
    pub final_file_name: Option<String>,
    pub submitted_at: Option<String>,
    pub folder_path: String,
    pub assignment_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewHomework<'a> {
    pub id: &'a str,
    pub week_id: &'a str,
    pub title: &'a str,
    pub deadline: &'a str,
    pub stage: HomeworkStage,
    pub workflow_stage: HomeworkWorkflowStage,
    pub folder_path: &'a str,
    pub assignment_url: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionAuthor {
    Codex,
    Claude,
    User,
}

impl VersionAuthor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::User => "user",
        }
    }

    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "codex" => Ok(Self::Codex),
            "claude" => Ok(Self::Claude),
            "user" => Ok(Self::User),
            _ => Err(format!("unknown version author: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkVersion {
    pub id: String,
    pub homework_id: String,
    pub number: i64,
    pub label: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub created_at: String,
    pub source_file_id: String,
    pub note: Option<String>,
    pub sections: Value,
    pub author: VersionAuthor,
    pub prompt_id: Option<String>,
    pub source_bundle_id: Option<String>,
    pub files: Value,
    pub frozen: bool,
    #[serde(skip)]
    pub file_path: String,
}

#[derive(Debug, Clone)]
pub struct NewHomeworkVersion<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub number: i64,
    pub label: &'a str,
    pub file_name: &'a str,
    pub file_path: &'a str,
    pub size_bytes: i64,
    pub source_file_id: &'a str,
    pub note: Option<&'a str>,
    pub sections: &'a Value,
    pub author: VersionAuthor,
    pub prompt_id: Option<&'a str>,
    pub source_bundle_id: Option<&'a str>,
    pub files: &'a Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxFile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub size_bytes: i64,
    pub added_at: String,
    pub origin: String,
    pub status: String,
    #[serde(skip)]
    pub relative_path: String,
    pub classification: Option<Value>,
    pub duplicate: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub size_bytes: i64,
    pub modified_at: String,
    pub relative_path: String,
}

#[derive(Debug, Clone)]
pub struct NewInboxFile<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub size_bytes: i64,
    pub added_at: &'a str,
    pub origin: &'a str,
    pub status: &'a str,
    pub relative_path: &'a str,
    pub classification: Option<&'a Value>,
    pub duplicate: Option<&'a Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBundle {
    pub id: String,
    pub homework_id: String,
    pub purpose: String,
    pub version_id: Option<String>,
    pub items: Vec<SourceBundleItem>,
    pub complete: bool,
    pub missing: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBundleItem {
    pub id: String,
    pub file_id: String,
    pub source_kind: String,
    pub required: bool,
    pub included: bool,
}

#[derive(Debug, Clone)]
pub struct NewSourceBundle<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub purpose: &'a str,
    pub version_id: Option<&'a str>,
    pub complete: bool,
    pub missing: &'a Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRun {
    pub id: String,
    pub homework_id: String,
    pub kind: String,
    pub role: String,
    pub provider: String,
    pub version_id: Option<String>,
    pub source_bundle_id: Option<String>,
    pub sources_used: Value,
    pub status: String,
    pub progress: i64,
    pub step: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub saved_as_iteration: bool,
    pub result: Option<Value>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPrompt {
    pub id: String,
    pub homework_id: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
    pub created_by: String,
    pub based_on_version_id: Option<String>,
    pub review_run_id: Option<String>,
    pub addresses: Value,
    pub sources: Value,
    pub sent_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeacherComment {
    pub id: String,
    pub homework_id: String,
    pub source: String,
    pub date: String,
    pub question: String,
    pub answer: String,
    pub added_at: String,
    pub attachment_file_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Done,
    NotDone,
    Manual,
    Na,
}

impl RequirementStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::NotDone => "not_done",
            Self::Manual => "manual",
            Self::Na => "na",
        }
    }

    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "done" => Ok(Self::Done),
            "not_done" => Ok(Self::NotDone),
            "manual" => Ok(Self::Manual),
            "na" => Ok(Self::Na),
            _ => Err(format!("unknown requirement status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeacherRequirement {
    pub id: String,
    pub homework_id: String,
    pub text: String,
    pub comment_id: Option<String>,
    pub origin: String,
    pub status: RequirementStatus,
    pub note: String,
    pub checked_version_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionPackage {
    pub id: String,
    pub homework_id: String,
    pub version_id: String,
    pub folder_path: String,
    pub items: Value,
    pub links: Value,
    pub warnings: Value,
    pub checklist: Value,
    pub upload_list: Value,
    pub built_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionReceipt {
    pub id: String,
    pub homework_id: String,
    pub version_id: String,
    pub package_id: String,
    pub submitted_at: String,
    pub confirmed_at: String,
    pub assignment_url: String,
    pub proof: Option<Value>,
    pub snapshot_path: String,
    pub files: Value,
    pub snapshot_hash: String,
}

#[derive(Debug, Clone)]
pub struct NewOperation<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub detail: Option<&'a str>,
    pub status: OperationStatus,
    pub undoable: bool,
    pub paths: &'a Value,
    pub backup_path: Option<&'a str>,
    pub homework_id: Option<&'a str>,
    pub error: Option<&'a Value>,
    pub undo_payload: Option<&'a Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Running,
    Completed,
    Failed,
    Undone,
}

impl OperationStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Undone => "undone",
        }
    }

    pub fn try_from_db(value: &str) -> Result<Self, String> {
        match value {
            "started" | "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" | "cancelled" => Ok(Self::Failed),
            "undone" => Ok(Self::Undone),
            _ => Err(format!("unknown operation status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    pub status: OperationStatus,
    pub undoable: bool,
    pub paths: Value,
    pub backup_path: Option<String>,
    pub homework_id: Option<String>,
    pub error: Option<Value>,
    #[serde(skip)]
    pub undo_payload: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendHealth {
    pub ready: bool,
    pub app_version: String,
    pub schema_version: u32,
    pub platform: String,
    pub database_exists: bool,
}

#[cfg(test)]
mod tests {
    use super::{HomeworkStage, HomeworkWorkflowStage, OperationStatus, RequirementStatus};

    #[test]
    fn all_homework_stages_round_trip_database_values() {
        for stage in [
            HomeworkStage::Task,
            HomeworkStage::InProgress,
            HomeworkStage::Ready,
            HomeworkStage::Submitted,
        ] {
            assert_eq!(HomeworkStage::try_from_db(stage.as_db_str()), Ok(stage));
        }
    }

    #[test]
    fn all_workflow_stages_round_trip_database_values() {
        for stage in [
            HomeworkWorkflowStage::MaterialsCollected,
            HomeworkWorkflowStage::CodexAnalysis,
            HomeworkWorkflowStage::PromptReady,
            HomeworkWorkflowStage::ClaudeSolving,
            HomeworkWorkflowStage::CodexReview,
            HomeworkWorkflowStage::NeedsFixes,
            HomeworkWorkflowStage::TeacherReview,
            HomeworkWorkflowStage::ReadyToSubmit,
            HomeworkWorkflowStage::Submitted,
        ] {
            assert_eq!(
                HomeworkWorkflowStage::try_from_db(stage.as_db_str()),
                Ok(stage)
            );
        }
    }

    #[test]
    fn all_requirement_statuses_round_trip() {
        for status in [
            RequirementStatus::Done,
            RequirementStatus::NotDone,
            RequirementStatus::Manual,
            RequirementStatus::Na,
        ] {
            assert_eq!(RequirementStatus::try_from_db(status.as_str()), Ok(status));
        }
    }

    #[test]
    fn operation_status_maps_legacy_values_exhaustively() {
        assert_eq!(
            OperationStatus::try_from_db("started"),
            Ok(OperationStatus::Running)
        );
        assert_eq!(
            OperationStatus::try_from_db("running"),
            Ok(OperationStatus::Running)
        );
        assert_eq!(
            OperationStatus::try_from_db("completed"),
            Ok(OperationStatus::Completed)
        );
        assert_eq!(
            OperationStatus::try_from_db("failed"),
            Ok(OperationStatus::Failed)
        );
        assert_eq!(
            OperationStatus::try_from_db("cancelled"),
            Ok(OperationStatus::Failed)
        );
        assert_eq!(
            OperationStatus::try_from_db("undone"),
            Ok(OperationStatus::Undone)
        );
    }
}
