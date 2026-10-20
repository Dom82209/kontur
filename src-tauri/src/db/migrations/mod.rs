use rusqlite::Connection;

use crate::error::BackendError;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

struct Migration {
    version: u32,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("v1.sql"),
}];

pub fn schema_version(connection: &Connection) -> Result<u32, BackendError> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version)
}

pub fn migrate(connection: &mut Connection) -> Result<u32, BackendError> {
    let current = schema_version(connection)?;
    if current > CURRENT_SCHEMA_VERSION {
        return Err(BackendError::conflict(format!(
            "database schema version {current} is newer than supported version {CURRENT_SCHEMA_VERSION}"
        )));
    }

    for migration in MIGRATIONS.iter().filter(|item| item.version > current) {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.pragma_update(None, "user_version", migration.version)?;
        transaction.commit()?;
    }

    schema_version(connection)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{migrate, schema_version, CURRENT_SCHEMA_VERSION};
    use crate::error::ErrorCode;

    #[test]
    fn migrates_empty_database() {
        let mut connection = Connection::open_in_memory().unwrap();
        assert_eq!(migrate(&mut connection).unwrap(), CURRENT_SCHEMA_VERSION);
        let expected = [
            "app_settings",
            "files",
            "homework_versions",
            "homeworks",
            "operations",
            "review_runs",
            "revision_prompts",
            "source_bundle_items",
            "source_bundles",
            "subjects",
            "submission_packages",
            "submission_receipts",
            "teacher_comments",
            "teacher_requirements",
            "weeks",
        ];
        for table in expected {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing table: {table}");
        }
    }

    #[test]
    fn migration_is_idempotent() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        migrate(&mut connection).unwrap();
        assert_eq!(schema_version(&connection).unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn rejects_newer_schema() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        let error = migrate(&mut connection).unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
    }
}
