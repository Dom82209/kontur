use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use crate::{
    db::Database,
    error::{AppError, BackendError},
    events::EventRegistry,
    intake::IntakeGrants,
    lms::{CandidateRegistry, WatcherService},
};

pub struct AppState {
    app_data_dir: PathBuf,
    home_dir: Option<PathBuf>,
    database: Database,
    events: EventRegistry,
    grants: IntakeGrants,
    import_lock: Mutex<()>,
    candidates: CandidateRegistry,
    watcher: WatcherService,
}

impl AppState {
    pub fn new(app_data_dir: PathBuf, home_dir: Option<PathBuf>, database: Database) -> Self {
        Self {
            app_data_dir,
            home_dir,
            database,
            events: EventRegistry,
            grants: IntakeGrants::new(),
            import_lock: Mutex::new(()),
            candidates: CandidateRegistry::new(),
            watcher: WatcherService::new(),
        }
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    pub fn home_dir(&self) -> Option<PathBuf> {
        self.home_dir.clone()
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    pub fn events(&self) -> &EventRegistry {
        &self.events
    }

    /// Пути, полученные из native picker/drop; живут только в памяти процесса.
    pub fn grants(&self) -> &IntakeGrants {
        &self.grants
    }

    /// Кандидаты на импорт (watcher загрузок и выбранные экспорты ЦУ), только в памяти.
    pub fn candidates(&self) -> &CandidateRegistry {
        &self.candidates
    }

    pub fn watcher(&self) -> &WatcherService {
        &self.watcher
    }

    /// Каталог для временной распаковки экспорта (внутри данных приложения).
    pub fn scratch_dir(&self) -> PathBuf {
        self.app_data_dir.join("tmp")
    }

    /// Импорты выполняются последовательно: повтор команды с тем же operation id
    /// дождётся первого вызова и вернёт сохранённый итог.
    pub fn lock_imports(&self) -> Result<MutexGuard<'_, ()>, AppError> {
        self.import_lock
            .lock()
            .map_err(|_| BackendError::database("Блокировка импорта повреждена"))
    }
}
