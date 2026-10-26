use crate::{
    dto::{
        ClassificationDto, DuplicateInfoDto, FileKind, InboxFileDto, InboxOrigin, InboxStatus,
        ProvenanceDto,
    },
    error::BackendError,
};
use rusqlite::{params, Row};

use super::{parse_json, require_relative_path, ReadRepository};

impl ReadRepository<'_> {
    pub fn inbox_file(&self, file_id: &str) -> Result<InboxFileDto, BackendError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT id, name, kind, size_bytes, added_at, origin, status, relative_path,
                            classification_json, duplicate_json, provenance_json
                     FROM inbox_files WHERE id = ?1",
                    params![file_id],
                    hydrate_inbox_file,
                )
                .map_err(|error| match error {
                    rusqlite::Error::QueryReturnedNoRows => {
                        BackendError::not_found("inbox file not found")
                    }
                    other => other.into(),
                })
        })
    }

    pub fn inbox_files(&self) -> Result<Vec<InboxFileDto>, BackendError> {
        self.database.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, kind, size_bytes, added_at, origin, status, relative_path,
                        classification_json, duplicate_json, provenance_json
                 FROM inbox_files
                 WHERE status <> 'routed'
                 ORDER BY added_at DESC, id DESC",
            )?;
            let mut rows = statement.query([])?;
            let mut output = Vec::new();
            while let Some(row) = rows.next()? {
                output.push(hydrate_inbox_file(row)?);
            }
            Ok(output)
        })
    }
}

fn hydrate_inbox_file(row: &Row<'_>) -> Result<InboxFileDto, rusqlite::Error> {
    let id: String = row.get(0)?;
    let raw_kind: String = row.get(2)?;
    let raw_origin: String = row.get(5)?;
    let raw_status: String = row.get(6)?;
    let relative_path: String = row.get(7)?;
    let classification_json: Option<String> = row.get(8)?;
    let duplicate_json: Option<String> = row.get(9)?;
    let provenance_json: Option<String> = row.get(10)?;
    require_relative_path(&relative_path, "inbox_files.relative_path")
        .map_err(|error| conversion_error(7, error.message))?;
    let classification = classification_json
        .as_deref()
        .map(|raw| parse_json::<ClassificationDto>(raw, "classification_json"))
        .transpose()
        .map_err(|error| conversion_error(8, error.message))?;
    if classification
        .as_ref()
        .is_some_and(|value| !(0.0..=1.0).contains(&value.confidence))
    {
        return Err(conversion_error(
            8,
            format!("inbox file {id} has invalid classification confidence"),
        ));
    }
    let duplicate = duplicate_json
        .as_deref()
        .map(|raw| parse_json::<DuplicateInfoDto>(raw, "duplicate_json"))
        .transpose()
        .map_err(|error| conversion_error(9, error.message))?;
    if let Some(value) = &duplicate {
        require_relative_path(&value.existing_path, "duplicate.existingPath")
            .map_err(|error| conversion_error(9, error.message))?;
    }
    let provenance = provenance_json
        .as_deref()
        .map(|raw| parse_json::<ProvenanceDto>(raw, "provenance_json"))
        .transpose()
        .map_err(|error| conversion_error(10, error.message))?;
    Ok(InboxFileDto {
        id,
        name: row.get(1)?,
        kind: FileKind::try_from_db(&raw_kind).map_err(|message| conversion_error(2, message))?,
        size_bytes: row.get(3)?,
        added_at: row.get(4)?,
        origin: InboxOrigin::try_from_db(&raw_origin)
            .map_err(|message| conversion_error(5, message))?,
        status: InboxStatus::try_from_db(&raw_status)
            .map_err(|message| conversion_error(6, message))?,
        classification,
        duplicate,
        provenance,
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
