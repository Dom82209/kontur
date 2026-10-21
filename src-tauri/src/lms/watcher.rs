//! Opt-in наблюдение за папкой загрузок.
//!
//! Выключено по умолчанию и включается только явным действием в Настройках.
//! Watcher **ничего не импортирует сам**: он находит завершённые загрузки
//! подходящих типов и показывает их как кандидатов; импорт начинается только
//! после предпросмотра и подтверждения пользователя.
//!
//! Наблюдение — периодический опрос верхнего уровня папки через открытый fd
//! (без рекурсии и без следования symlink). Файл становится кандидатом, когда:
//!
//! - имя не временное (`.crdownload`, `.part`, `.download`, `.tmp`, скрытые, `~$…`)
//!   и рядом нет одноимённого временного файла (Firefox держит пустой файл,
//!   пока докачивает `.part`);
//! - расширение из списка учебных форматов;
//! - размер больше нуля и не больше лимита;
//! - размер, mtime и inode не менялись `stable_polls` опросов подряд и не меньше
//!   `quiet_period` (debounce окончания загрузки);
//! - этот файл (по inode/размеру/mtime) ещё не предлагался в текущем сеансе, а его
//!   SHA-256 не встречался во «Входящих» и в `download_seen`.
//!
//! Файлы, уже лежавшие в папке при включении, считаются базовой линией и не
//! предлагаются (для них есть кнопка «Импортировать экспорт ЦУ»).

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    db::Database,
    error::{AppError, BackendError},
    file_service::{hash_source, DownloadEntry, DownloadsRoot, FileIdentity},
};

use super::{
    candidates::{
        now_iso, CandidateOrigin, CandidateRegistry, CandidateSource, DownloadCandidateDto,
    },
    store,
};

pub const POLL_INTERVAL: Duration = Duration::from_millis(1500);
/// Файлы больше этого размера watcher не хэширует и не предлагает.
pub const MAX_CANDIDATE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const TEMP_SUFFIXES: &[&str] = &[
    ".crdownload",
    ".part",
    ".partial",
    ".download",
    ".tmp",
    ".temp",
    ".opdownload",
    ".aria2",
    ".!ut",
    ".filepart",
];

const ALLOWED_EXTENSIONS: &[&str] = &[
    "zip", "pdf", "doc", "docx", "xls", "xlsx", "csv", "tsv", "ppt", "pptx", "ipynb", "txt", "md",
    "json", "odt", "ods", "odp", "rtf", "tex", "png", "jpg", "jpeg", "py", "r", "sql", "parquet",
    "html",
];

/// Временный файл браузера или скрытый файл — никогда не кандидат.
pub fn is_temporary_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.starts_with('.')
        || name.starts_with("~$")
        || lower.starts_with("unconfirmed ")
        || TEMP_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

/// Имя подходит для кандидата: не временное и учебного формата.
pub fn is_candidate_name(name: &str) -> bool {
    if is_temporary_name(name) {
        return false;
    }
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_lowercase())
        .unwrap_or_default();
    ALLOWED_EXTENSIONS.contains(&extension.as_str())
}

#[derive(Debug, Clone, Copy)]
pub struct ScannerConfig {
    pub quiet_period: Duration,
    pub stable_polls: u32,
    pub max_bytes: u64,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            quiet_period: Duration::from_secs(3),
            stable_polls: 2,
            max_bytes: MAX_CANDIDATE_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Signature {
    identity: FileIdentity,
    size: u64,
    modified_ns: i128,
}

impl Signature {
    fn of(entry: &DownloadEntry) -> Self {
        Self {
            identity: entry.identity,
            size: entry.size_bytes,
            modified_ns: entry.modified_ns,
        }
    }
}

#[derive(Debug, Clone)]
struct Observation {
    signature: Signature,
    last_change: Instant,
    stable_polls: u32,
}

/// Чистая логика стабилизации: без файловой системы и часов, только наблюдения.
#[derive(Debug, Default)]
pub struct DownloadScanner {
    config: ScannerConfig,
    observations: HashMap<String, Observation>,
    offered: HashSet<Signature>,
    baseline_taken: bool,
}

impl DownloadScanner {
    pub fn new(config: ScannerConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// Принимает список файлов верхнего уровня и возвращает те, что только что
    /// стали готовыми кандидатами. Каждый файл возвращается не больше одного раза.
    pub fn scan(&mut self, entries: &[DownloadEntry], now: Instant) -> Vec<DownloadEntry> {
        let in_progress: HashSet<String> = entries
            .iter()
            .filter_map(|entry| {
                let lower = entry.name.to_lowercase();
                TEMP_SUFFIXES
                    .iter()
                    .find_map(|suffix| lower.strip_suffix(suffix).map(|base| base.to_owned()))
            })
            .collect();
        let present: HashSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        self.observations
            .retain(|name, _| present.contains(name.as_str()));

        let mut ready = Vec::new();
        for entry in entries {
            if !is_candidate_name(&entry.name) {
                continue;
            }
            let signature = Signature::of(entry);
            if !self.baseline_taken {
                self.offered.insert(signature);
                self.observations.insert(
                    entry.name.clone(),
                    Observation {
                        signature,
                        last_change: now,
                        stable_polls: 0,
                    },
                );
                continue;
            }
            let observation = self
                .observations
                .entry(entry.name.clone())
                .or_insert(Observation {
                    signature,
                    last_change: now,
                    stable_polls: 0,
                });
            if observation.signature == signature {
                observation.stable_polls = observation.stable_polls.saturating_add(1);
            } else {
                observation.signature = signature;
                observation.last_change = now;
                observation.stable_polls = 0;
            }
            let quiet = now.saturating_duration_since(observation.last_change);
            let downloading = in_progress.contains(&entry.name.to_lowercase());
            if observation.stable_polls >= self.config.stable_polls
                && quiet >= self.config.quiet_period
                && !downloading
                && entry.size_bytes > 0
                && entry.size_bytes <= self.config.max_bytes
                && self.offered.insert(signature)
            {
                ready.push(entry.clone());
            }
        }
        self.baseline_taken = true;
        ready
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatcherState {
    /// Выключен пользователем (по умолчанию).
    Disabled,
    /// Включён, но папка загрузок не выбрана или недоступна.
    NoFolder,
    Watching,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsWatcherDto {
    pub enabled: bool,
    pub state: WatcherState,
    pub message: Option<String>,
    pub last_scan_at: Option<String>,
    pub candidates: Vec<DownloadCandidateDto>,
}

/// Один шаг наблюдения, независимый от потока и Tauri (тестируется на TempDir).
pub struct WatcherCore {
    scanner: DownloadScanner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickOutcome {
    /// Список кандидатов изменился (нужно событие для UI).
    pub changed: bool,
}

impl WatcherCore {
    pub fn new(config: ScannerConfig) -> Self {
        Self {
            scanner: DownloadScanner::new(config),
        }
    }

    pub fn tick(
        &mut self,
        root: &DownloadsRoot,
        database: &Database,
        registry: &CandidateRegistry,
        now: Instant,
    ) -> Result<TickOutcome, AppError> {
        let entries = root.list_files()?;
        let present: Vec<(String, FileIdentity)> = entries
            .iter()
            .map(|entry| (entry.name.clone(), entry.identity))
            .collect();
        let mut changed = registry.retain_downloads(&present)?;
        for entry in self.scanner.scan(&entries, now) {
            // Хэш читается из того же открытого inode, что будет импортирован.
            let Ok(source) = root.open_file(&entry.name, entry.identity) else {
                continue;
            };
            let Ok(sha256) = hash_source(&source) else {
                continue;
            };
            if store::is_known_content(database, sha256.as_str())? {
                continue;
            }
            registry.register(
                entry.name.clone(),
                entry.size_bytes,
                CandidateOrigin::Watcher,
                CandidateSource::Downloads {
                    identity: entry.identity,
                    size_bytes: entry.size_bytes,
                    modified_ns: entry.modified_ns,
                },
                Some(sha256.as_str().to_owned()),
            )?;
            changed = true;
        }
        Ok(TickOutcome { changed })
    }
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct RuntimeStatus {
    state: WatcherState,
    message: Option<String>,
    last_scan_at: Option<String>,
}

/// Управление фоновым потоком watcher. Поток один; повторное включение
/// перезапускает его с новой базовой линией.
pub struct WatcherService {
    running: Mutex<Option<Running>>,
    status: Arc<Mutex<RuntimeStatus>>,
}

impl Default for WatcherService {
    fn default() -> Self {
        Self {
            running: Mutex::new(None),
            status: Arc::new(Mutex::new(RuntimeStatus {
                state: WatcherState::Disabled,
                message: None,
                last_scan_at: None,
            })),
        }
    }
}

/// Шаг наблюдения с доступом к БД, реестру и домашнему каталогу.
pub type WatcherTask<'a> =
    dyn FnMut(&Database, &CandidateRegistry, Option<&Path>) -> Result<TickOutcome, AppError> + 'a;

/// Что нужно фоновому потоку на каждом шаге.
pub trait WatcherHost: Send + 'static {
    /// Выполнить шаг с доступом к БД и реестру. `None` — приложение завершается.
    fn with_state(&self, task: &mut WatcherTask<'_>) -> Option<Result<TickOutcome, AppError>>;
    /// Сообщить UI об изменении.
    fn notify(&self);
}

impl WatcherService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_running(&self) -> bool {
        self.running
            .lock()
            .map(|running| running.is_some())
            .unwrap_or(false)
    }

    /// Запускает поток. Уже запущенный поток останавливается и заменяется.
    pub fn start(
        &self,
        host: Box<dyn WatcherHost>,
        downloads: PathBuf,
        config: ScannerConfig,
    ) -> Result<(), AppError> {
        self.stop()?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let status = Arc::clone(&self.status);
        self.set_status(WatcherState::Watching, None)?;
        let handle = std::thread::Builder::new()
            .name("kontur-downloads-watcher".to_owned())
            .spawn(move || run_loop(host, downloads, config, thread_stop, status))
            .map_err(|_| BackendError::io("Не удалось запустить наблюдение за загрузками"))?;
        *self.lock_running()? = Some(Running { stop, handle });
        Ok(())
    }

    pub fn stop(&self) -> Result<(), AppError> {
        let running = self.lock_running()?.take();
        if let Some(running) = running {
            running.stop.store(true, Ordering::SeqCst);
            let _ = running.handle.join();
        }
        self.set_status(WatcherState::Disabled, None)
    }

    pub fn mark(&self, state: WatcherState, message: Option<String>) -> Result<(), AppError> {
        self.set_status(state, message)
    }

    pub fn snapshot(
        &self,
        enabled: bool,
        registry: &CandidateRegistry,
    ) -> Result<DownloadsWatcherDto, AppError> {
        let status = self
            .status
            .lock()
            .map_err(|_| BackendError::database("Состояние watcher повреждено"))?
            .clone();
        let state = if enabled {
            status.state
        } else {
            WatcherState::Disabled
        };
        Ok(DownloadsWatcherDto {
            enabled,
            state,
            message: if enabled { status.message } else { None },
            last_scan_at: status.last_scan_at,
            candidates: registry.list()?,
        })
    }

    fn set_status(&self, state: WatcherState, message: Option<String>) -> Result<(), AppError> {
        let mut status = self
            .status
            .lock()
            .map_err(|_| BackendError::database("Состояние watcher повреждено"))?;
        status.state = state;
        status.message = message;
        Ok(())
    }

    fn lock_running(&self) -> Result<std::sync::MutexGuard<'_, Option<Running>>, AppError> {
        self.running
            .lock()
            .map_err(|_| BackendError::database("Состояние watcher повреждено"))
    }
}

impl Drop for WatcherService {
    fn drop(&mut self) {
        if let Ok(mut running) = self.running.lock() {
            if let Some(running) = running.take() {
                running.stop.store(true, Ordering::SeqCst);
                let _ = running.handle.join();
            }
        }
    }
}

fn run_loop(
    host: Box<dyn WatcherHost>,
    downloads: PathBuf,
    config: ScannerConfig,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<RuntimeStatus>>,
) {
    let mut core = WatcherCore::new(config);
    let mut last_state: Option<(WatcherState, Option<String>)> = None;
    while !stop.load(Ordering::SeqCst) {
        let result = host.with_state(&mut |database, registry, home| {
            let root = DownloadsRoot::new(&downloads, home)?;
            core.tick(&root, database, registry, Instant::now())
        });
        let Some(result) = result else { break };
        let (state, message, changed) = match result {
            Ok(outcome) => (WatcherState::Watching, None, outcome.changed),
            Err(error) => (WatcherState::Error, Some(error.message), false),
        };
        let state_changed = last_state.as_ref() != Some(&(state, message.clone()));
        if let Ok(mut current) = status.lock() {
            current.state = state;
            current.message = message.clone();
            current.last_scan_at = Some(now_iso());
        }
        if changed || state_changed {
            host.notify();
        }
        last_state = Some((state, message));
        let mut waited = Duration::ZERO;
        while waited < POLL_INTERVAL && !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(100));
            waited += Duration::from_millis(100);
        }
    }
}
