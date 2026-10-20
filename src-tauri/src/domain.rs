use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subject {
    pub id: String,
    pub name: String,
    pub folder_path: String,
}

#[derive(Debug, Clone)]
pub struct NewSubject<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub folder_path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Week {
    pub id: String,
    pub subject_id: String,
    pub number: i64,
    pub folder_path: String,
}

#[derive(Debug, Clone)]
pub struct NewWeek<'a> {
    pub id: &'a str,
    pub subject_id: &'a str,
    pub number: i64,
    pub folder_path: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HomeworkStage {
    Task,
    InProgress,
    ReadyToSubmit,
    Submitted,
}

impl HomeworkStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::InProgress => "in_progress",
            Self::ReadyToSubmit => "ready_to_submit",
            Self::Submitted => "submitted",
        }
    }
}

impl TryFrom<&str> for HomeworkStage {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "task" => Ok(Self::Task),
            "in_progress" => Ok(Self::InProgress),
            "ready_to_submit" => Ok(Self::ReadyToSubmit),
            "submitted" => Ok(Self::Submitted),
            _ => Err(format!("unknown homework stage: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Homework {
    pub id: String,
    pub week_id: String,
    pub title: String,
    pub stage: HomeworkStage,
    pub workflow_stage: String,
    pub folder_path: String,
}

#[derive(Debug, Clone)]
pub struct NewHomework<'a> {
    pub id: &'a str,
    pub week_id: &'a str,
    pub title: &'a str,
    pub stage: HomeworkStage,
    pub workflow_stage: &'a str,
    pub folder_path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeworkVersion {
    pub id: String,
    pub homework_id: String,
    pub number: i64,
    pub label: String,
    pub file_path: String,
    pub author: String,
    pub frozen: bool,
}

#[derive(Debug, Clone)]
pub struct NewHomeworkVersion<'a> {
    pub id: &'a str,
    pub homework_id: &'a str,
    pub number: i64,
    pub label: &'a str,
    pub file_path: &'a str,
    pub author: &'a str,
}

#[derive(Debug, Clone)]
pub struct NewOperation<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub status: &'a str,
    pub undoable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub undoable: bool,
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
