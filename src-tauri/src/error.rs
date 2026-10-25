use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    Conflict,
    Duplicate,
    ConfirmationRequired,
    IoError,
    NotUndoable,
    Immutable,
    Blocked,
    Cancelled,
    Database,
    /// Этап 9: провайдер (официальный CLI) недоступен или не подключён.
    ProviderUnavailable,
    /// Этап 9: официальный CLI требует входа — выполняется в самом CLI.
    ReauthRequired,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendError {
    pub code: ErrorCode,
    pub message: String,
}

pub type AppError = BackendError;

impl BackendError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn immutable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Immutable, message)
    }

    pub fn database(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Database, message)
    }

    pub fn blocked(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Blocked, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::IoError, message)
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for BackendError {}

impl From<rusqlite::Error> for BackendError {
    fn from(error: rusqlite::Error) -> Self {
        Self::database(error.to_string())
    }
}

impl From<std::io::Error> for BackendError {
    fn from(error: std::io::Error) -> Self {
        Self::new(ErrorCode::IoError, error.to_string())
    }
}
