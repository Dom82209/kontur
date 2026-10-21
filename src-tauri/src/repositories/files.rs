use std::collections::HashMap;

use rusqlite::params;

use crate::{
    dto::{FileKind, FileRefDto},
    error::BackendError,
};

use super::{invalid_enum, require_relative_path, ReadRepository};

#[derive(Debug)]
pub(super) struct OwnedFile {
    pub owner_id: String,
    pub category: String,
    pub file: FileRefDto,
}

impl ReadRepository<'_> {
    pub(super) fn week_files(
        &self,
        subject_id: Option<&str>,
    ) -> Result<Vec<OwnedFile>, BackendError> {
        self.database.with_connection(|connection| {
            let sql = if subject_id.is_some() {
                "SELECT f.week_id, f.category, f.id, f.name, f.kind, f.size_bytes,
                        coalesce(f.modified_at, f.updated_at, f.created_at), f.relative_path
                 FROM files f JOIN weeks w ON w.id = f.week_id
                 WHERE w.subject_id = ?1
                 ORDER BY w.number, f.relative_path, f.id"
            } else {
                "SELECT f.week_id, f.category, f.id, f.name, f.kind, f.size_bytes,
                        coalesce(f.modified_at, f.updated_at, f.created_at), f.relative_path
                 FROM files f WHERE f.week_id IS NOT NULL
                 ORDER BY f.week_id, f.relative_path, f.id"
            };
            let mut statement = connection.prepare(sql)?;
            let mut rows = if let Some(id) = subject_id {
                statement.query(params![id])?
            } else {
                statement.query([])?
            };
            let mut output = Vec::new();
            while let Some(row) = rows.next()? {
                output.push(OwnedFile {
                    owner_id: row.get(0)?,
                    category: row.get(1)?,
                    file: hydrate_file(
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    )?,
                });
            }
            Ok(output)
        })
    }

    pub(super) fn task_files(
        &self,
        homework_id: Option<&str>,
    ) -> Result<HashMap<String, Vec<FileRefDto>>, BackendError> {
        self.database.with_connection(|connection| {
            let sql = if homework_id.is_some() {
                "SELECT homework_id, id, name, kind, size_bytes,
                        coalesce(modified_at, updated_at, created_at), relative_path
                 FROM files
                 WHERE homework_id = ?1 AND category IN ('homework_task', 'homework_data')
                 ORDER BY relative_path, id"
            } else {
                "SELECT homework_id, id, name, kind, size_bytes,
                        coalesce(modified_at, updated_at, created_at), relative_path
                 FROM files
                 WHERE homework_id IS NOT NULL AND category IN ('homework_task', 'homework_data')
                 ORDER BY homework_id, relative_path, id"
            };
            let mut statement = connection.prepare(sql)?;
            let mut rows = if let Some(id) = homework_id {
                statement.query(params![id])?
            } else {
                statement.query([])?
            };
            let mut output: HashMap<String, Vec<FileRefDto>> = HashMap::new();
            while let Some(row) = rows.next()? {
                let homework_id: String = row.get(0)?;
                let file = hydrate_file(
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                )?;
                output.entry(homework_id).or_default().push(file);
            }
            Ok(output)
        })
    }
}

pub(super) fn hydrate_file(
    id: String,
    name: String,
    raw_kind: String,
    size_bytes: i64,
    modified_at: String,
    relative_path: String,
) -> Result<FileRefDto, BackendError> {
    require_relative_path(&relative_path, "files.relative_path")?;
    let kind =
        FileKind::try_from_db(&raw_kind).map_err(|message| invalid_enum("files.kind", message))?;
    Ok(FileRefDto {
        id,
        name,
        kind,
        size_bytes,
        modified_at,
        relative_path,
    })
}

pub(super) fn group_week_files(
    files: Vec<OwnedFile>,
) -> HashMap<String, Vec<(String, FileRefDto)>> {
    let mut grouped: HashMap<String, Vec<(String, FileRefDto)>> = HashMap::new();
    for owned in files {
        grouped
            .entry(owned.owner_id)
            .or_default()
            .push((owned.category, owned.file));
    }
    grouped
}
