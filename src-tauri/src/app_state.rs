use std::path::{Path, PathBuf};

use crate::{db::Database, events::EventRegistry};

pub struct AppState {
    app_data_dir: PathBuf,
    database: Database,
    events: EventRegistry,
}

impl AppState {
    pub fn new(app_data_dir: PathBuf, database: Database) -> Self {
        Self {
            app_data_dir,
            database,
            events: EventRegistry,
        }
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    pub fn events(&self) -> &EventRegistry {
        &self.events
    }
}
