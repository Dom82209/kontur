use crate::{
    dto::{ClassificationDto, DuplicateInfoDto, FileKind, InboxFileDto, InboxOrigin, InboxStatus},
    error::BackendError,
};

use super::{invalid_enum, parse_json, require_relative_path, ReadRepository};

impl ReadRepository<'_> {
    pub fn inbox_files(&self) -> Result<Vec<InboxFileDto>, BackendError> {
        self.database.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT id, name, kind, size_bytes, added_at, origin, status, relative_path,
                        classification_json, duplicate_json
                 FROM inbox_files ORDER BY added_at DESC, id DESC",
            )?;
            let mut rows = statement.query([])?;
            let mut output = Vec::new();
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let raw_kind: String = row.get(2)?;
                let raw_origin: String = row.get(5)?;
                let raw_status: String = row.get(6)?;
                let relative_path: String = row.get(7)?;
                let classification_json: Option<String> = row.get(8)?;
                let duplicate_json: Option<String> = row.get(9)?;
                require_relative_path(&relative_path, "inbox_files.relative_path")?;
                let classification = classification_json
                    .as_deref()
                    .map(|raw| parse_json::<ClassificationDto>(raw, "classification_json"))
                    .transpose()?;
                if classification
                    .as_ref()
                    .is_some_and(|value| !(0.0..=1.0).contains(&value.confidence))
                {
                    return Err(BackendError::database(format!(
                        "inbox file {id} has invalid classification confidence"
                    )));
                }
                let duplicate = duplicate_json
                    .as_deref()
                    .map(|raw| parse_json::<DuplicateInfoDto>(raw, "duplicate_json"))
                    .transpose()?;
                if let Some(value) = &duplicate {
                    require_relative_path(&value.existing_path, "duplicate.existingPath")?;
                }
                output.push(InboxFileDto {
                    id,
                    name: row.get(1)?,
                    kind: FileKind::try_from_db(&raw_kind)
                        .map_err(|message| invalid_enum("inbox_files.kind", message))?,
                    size_bytes: row.get(3)?,
                    added_at: row.get(4)?,
                    origin: InboxOrigin::try_from_db(&raw_origin)
                        .map_err(|message| invalid_enum("inbox_files.origin", message))?,
                    status: InboxStatus::try_from_db(&raw_status)
                        .map_err(|message| invalid_enum("inbox_files.status", message))?,
                    classification,
                    duplicate,
                });
            }
            Ok(output)
        })
    }
}
