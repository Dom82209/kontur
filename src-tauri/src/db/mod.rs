pub mod migrations;

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{Connection, Transaction};

use crate::error::BackendError;

#[derive(Debug, Clone)]
enum DatabaseLocation {
    File(PathBuf),
    Memory,
}

pub struct Database {
    connection: Mutex<Connection>,
    location: DatabaseLocation,
}

impl Database {
    pub fn open_file(path: impl AsRef<Path>) -> Result<Self, BackendError> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        configure_connection(&connection, true)?;
        Ok(Self {
            connection: Mutex::new(connection),
            location: DatabaseLocation::File(path),
        })
    }

    pub fn open_in_memory() -> Result<Self, BackendError> {
        let connection = Connection::open_in_memory()?;
        configure_connection(&connection, false)?;
        Ok(Self {
            connection: Mutex::new(connection),
            location: DatabaseLocation::Memory,
        })
    }

    pub fn migrate(&self) -> Result<u32, BackendError> {
        let mut connection = self.lock()?;
        migrations::migrate(&mut connection)
    }

    pub fn schema_version(&self) -> Result<u32, BackendError> {
        let connection = self.lock()?;
        migrations::schema_version(&connection)
    }

    pub fn ping(&self) -> Result<(), BackendError> {
        let connection = self.lock()?;
        connection.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn exists(&self) -> bool {
        match &self.location {
            DatabaseLocation::File(path) => path.is_file(),
            DatabaseLocation::Memory => true,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.location {
            DatabaseLocation::File(path) => Some(path),
            DatabaseLocation::Memory => None,
        }
    }

    pub fn with_connection<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let connection = self.lock()?;
        action(&connection)
    }

    pub fn with_transaction<T>(
        &self,
        action: impl FnOnce(&Transaction<'_>) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let result = action(&transaction)?;
        transaction.commit()?;
        Ok(result)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, BackendError> {
        self.connection
            .lock()
            .map_err(|_| BackendError::database("database connection lock is poisoned"))
    }
}

fn configure_connection(connection: &Connection, production: bool) -> Result<(), BackendError> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    if production {
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
    }
    Ok(())
}
