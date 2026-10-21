use crate::{
    domain::OperationStatus,
    dto::{OperationDto, OperationKind},
    error::BackendError,
};
use rusqlite::{params, Row};

use super::{parse_json, require_relative_path, ReadRepository};

impl ReadRepository<'_> {
    pub fn operation(&self, operation_id: &str) -> Result<OperationDto, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, kind, title, detail, status, started_at, completed_at, undoable,
                            paths_json, backup_path, homework_id
                     FROM operations WHERE id = ?1",
                    params![operation_id],
                    hydrate_operation,
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        BackendError::not_found("operation not found")
                    }
                    other => other.into(),
                })
        })
    }

    pub fn activity_log(&self) -> Result<Vec<OperationDto>, BackendError> {
        self.database.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, kind, title, detail, status, started_at, completed_at, undoable,
                        paths_json, backup_path, homework_id
                 FROM operations ORDER BY started_at DESC, id DESC",
            )?;
            let mut rows = statement.query([])?;
            let mut output = Vec::new();
            while let Some(row) = rows.next()? {
                output.push(hydrate_operation(row)?);
            }
            Ok(output)
        })
    }
}

fn hydrate_operation(row: &Row<'_>) -> Result<OperationDto, rusqlite::Error> {
    let id: String = row.get(0)?;
    let raw_kind: String = row.get(1)?;
    let raw_status: String = row.get(4)?;
    let paths_json: String = row.get(8)?;
    let paths: Vec<String> = parse_json(&paths_json, "operations.paths_json")
        .map_err(|error| conversion_error(8, error.message))?;
    for path in &paths {
        require_relative_path(path, "operations.paths_json")
            .map_err(|error| conversion_error(8, error.message))?;
    }
    let backup_path: Option<String> = row.get(9)?;
    if let Some(path) = &backup_path {
        require_relative_path(path, "operations.backup_path")
            .map_err(|error| conversion_error(9, error.message))?;
    }
    Ok(OperationDto {
        id,
        kind: OperationKind::try_from_db(&raw_kind)
            .map_err(|message| conversion_error(1, message))?,
        title: row.get(2)?,
        detail: row.get(3)?,
        status: OperationStatus::try_from_db(&raw_status)
            .map_err(|message| conversion_error(4, message))?,
        started_at: row.get(5)?,
        finished_at: row.get(6)?,
        undoable: row.get::<_, i64>(7)? == 1,
        paths,
        backup_path,
        homework_id: row.get(10)?,
    })
}

fn conversion_error(index: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}
