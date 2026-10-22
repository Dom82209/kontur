//! Флаги отмены запущенных пакетов извлечения (только в памяти процесса).

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use crate::error::{AppError, BackendError};

#[derive(Default)]
pub struct ExtractionJobs {
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl ExtractionJobs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Регистрирует пакет; повторный запуск с тем же id, пока первый идёт, — `conflict`.
    pub fn register(&self, operation_id: &str) -> Result<Arc<AtomicBool>, AppError> {
        let mut running = self
            .running
            .lock()
            .map_err(|_| BackendError::database("Реестр извлечения повреждён"))?;
        if running.contains_key(operation_id) {
            return Err(BackendError::conflict(
                "Извлечение с этим идентификатором уже выполняется",
            ));
        }
        let flag = Arc::new(AtomicBool::new(false));
        running.insert(operation_id.to_owned(), Arc::clone(&flag));
        Ok(flag)
    }

    /// Запрашивает остановку. `false` — такого пакета нет (уже завершён).
    pub fn cancel(&self, operation_id: &str) -> bool {
        self.running
            .lock()
            .ok()
            .and_then(|running| running.get(operation_id).cloned())
            .map(|flag| flag.store(true, Ordering::Relaxed))
            .is_some()
    }

    pub fn finish(&self, operation_id: &str) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(operation_id);
        }
    }

    pub fn is_running(&self, operation_id: &str) -> bool {
        self.running
            .lock()
            .map(|running| running.contains_key(operation_id))
            .unwrap_or(false)
    }
}
