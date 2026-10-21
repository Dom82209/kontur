use crate::{
    domain::OperationStatus,
    dto::{OperationDto, OperationKind},
    error::BackendError,
};

use super::{invalid_enum, parse_json, require_relative_path, ReadRepository};

impl ReadRepository<'_> {
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
                let id: String = row.get(0)?;
                let raw_kind: String = row.get(1)?;
                let raw_status: String = row.get(4)?;
                let paths_json: String = row.get(8)?;
                let paths: Vec<String> = parse_json(&paths_json, "operations.paths_json")?;
                for path in &paths {
                    require_relative_path(path, "operations.paths_json")?;
                }
                let backup_path: Option<String> = row.get(9)?;
                if let Some(path) = &backup_path {
                    require_relative_path(path, "operations.backup_path")?;
                }
                output.push(OperationDto {
                    id,
                    kind: OperationKind::try_from_db(&raw_kind)
                        .map_err(|message| invalid_enum("operations.kind", message))?,
                    title: row.get(2)?,
                    detail: row.get(3)?,
                    status: OperationStatus::try_from_db(&raw_status)
                        .map_err(|message| invalid_enum("operations.status", message))?,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                    undoable: row.get::<_, i64>(7)? == 1,
                    paths,
                    backup_path,
                    homework_id: row.get(10)?,
                });
            }
            Ok(output)
        })
    }
}
