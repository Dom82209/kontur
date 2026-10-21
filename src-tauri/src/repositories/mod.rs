mod files;
mod homeworks;
mod inbox;
mod mutations;
mod operations;
mod settings;
mod subjects;
mod weeks;
pub(crate) mod writes;

use std::path::{Component, Path};

use serde::de::DeserializeOwned;

use crate::{db::Database, error::BackendError};

pub use mutations::Repositories;

pub struct ReadRepository<'a> {
    database: &'a Database,
}

impl<'a> ReadRepository<'a> {
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    pub fn schema_version(&self) -> Result<u32, BackendError> {
        self.database.schema_version()
    }
}

fn parse_json<T: DeserializeOwned>(raw: &str, field: &str) -> Result<T, BackendError> {
    serde_json::from_str(raw)
        .map_err(|error| BackendError::database(format!("invalid JSON in {field}: {error}")))
}

fn require_relative_path(value: &str, field: &str) -> Result<(), BackendError> {
    let path = Path::new(value);
    let valid = !value.trim().is_empty()
        && !path.is_absolute()
        && !value.starts_with('\\')
        && !value.contains(':')
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    if valid {
        Ok(())
    } else {
        Err(BackendError::database(format!(
            "invalid persistent path in {field}"
        )))
    }
}

fn invalid_enum(field: &str, message: String) -> BackendError {
    BackendError::database(format!("invalid enum in {field}: {message}"))
}
