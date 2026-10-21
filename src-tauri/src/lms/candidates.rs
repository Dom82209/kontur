//! Кандидаты на импорт: файлы, найденные watcher в папке загрузок или выбранные
//! кнопкой «Импортировать экспорт ЦУ».
//!
//! Реестр живёт только в памяти процесса. WebView получает непрозрачный id,
//! имя и размер — абсолютный путь наружу не передаётся. Для файла из загрузок
//! хранится только имя внутри папки и identity inode: при импорте файл
//! открывается заново fd-relative и сверяется с ней.

use std::{path::PathBuf, sync::Mutex};

use serde::Serialize;
use uuid::Uuid;

use crate::{
    error::{AppError, BackendError},
    file_service::FileIdentity,
};

const MAX_CANDIDATES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateOrigin {
    /// Найден watcher папки загрузок.
    Watcher,
    /// Выбран пользователем кнопкой «Импортировать экспорт ЦУ».
    Manual,
}

/// Вид кандидата по имени: ZIP-экспорт, PDF-экспорт или обычный файл.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Zip,
    Pdf,
    File,
}

impl CandidateKind {
    pub fn of(name: &str) -> Self {
        let lower = name.to_lowercase();
        if lower.ends_with(".zip") {
            Self::Zip
        } else if lower.ends_with(".pdf") {
            Self::Pdf
        } else {
            Self::File
        }
    }
}

#[derive(Debug, Clone)]
pub enum CandidateSource {
    /// Файл верхнего уровня папки загрузок: inode, размер и mtime на момент
    /// стабилизации. Любое расхождение при открытии — файл подменён.
    Downloads {
        identity: FileIdentity,
        size_bytes: u64,
        modified_ns: i128,
    },
    /// Путь из системного диалога; дополнительно разрешён в `IntakeGrants`.
    External { path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub detected_at: String,
    pub origin: CandidateOrigin,
    pub kind: CandidateKind,
    pub source: CandidateSource,
    /// SHA-256, если уже вычислен (watcher хэширует при стабилизации).
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadCandidateDto {
    pub id: String,
    pub name: String,
    pub size_bytes: u64,
    pub detected_at: String,
    pub origin: CandidateOrigin,
    pub kind: CandidateKind,
}

impl From<&Candidate> for DownloadCandidateDto {
    fn from(candidate: &Candidate) -> Self {
        Self {
            id: candidate.id.clone(),
            name: candidate.name.clone(),
            size_bytes: candidate.size_bytes,
            detected_at: candidate.detected_at.clone(),
            origin: candidate.origin,
            kind: candidate.kind,
        }
    }
}

#[derive(Debug, Default)]
pub struct CandidateRegistry {
    entries: Mutex<Vec<Candidate>>,
}

impl CandidateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Регистрирует кандидата и возвращает его id. Старейшие вытесняются при
    /// переполнении. Повторное обнаружение того же файла watcher не дублирует.
    pub fn register(
        &self,
        name: String,
        size_bytes: u64,
        origin: CandidateOrigin,
        source: CandidateSource,
        sha256: Option<String>,
    ) -> Result<Candidate, AppError> {
        let mut entries = self.lock()?;
        if let CandidateSource::Downloads { identity, .. } = &source {
            if let Some(existing) = entries.iter().find(|candidate| {
                candidate.name == name
                    && matches!(&candidate.source, CandidateSource::Downloads { identity: known, .. } if known == identity)
            }) {
                return Ok(existing.clone());
            }
        }
        let candidate = Candidate {
            id: Uuid::new_v4().to_string(),
            kind: CandidateKind::of(&name),
            name,
            size_bytes,
            detected_at: now_iso(),
            origin,
            source,
            sha256,
        };
        entries.push(candidate.clone());
        if entries.len() > MAX_CANDIDATES {
            let overflow = entries.len() - MAX_CANDIDATES;
            entries.drain(..overflow);
        }
        Ok(candidate)
    }

    pub fn get(&self, id: &str) -> Result<Candidate, AppError> {
        self.lock()?
            .iter()
            .find(|candidate| candidate.id == id)
            .cloned()
            .ok_or_else(|| {
                BackendError::not_found(
                    "Кандидат на импорт не найден — обновите список или выберите файл заново",
                )
            })
    }

    pub fn remove(&self, id: &str) -> Result<Option<Candidate>, AppError> {
        let mut entries = self.lock()?;
        let position = entries.iter().position(|candidate| candidate.id == id);
        Ok(position.map(|index| entries.remove(index)))
    }

    /// Убирает кандидатов watcher, чьих файлов больше нет в папке загрузок.
    pub fn retain_downloads(&self, present: &[(String, FileIdentity)]) -> Result<bool, AppError> {
        let mut entries = self.lock()?;
        let before = entries.len();
        entries.retain(|candidate| match &candidate.source {
            CandidateSource::Downloads { identity, .. } => present
                .iter()
                .any(|(name, known)| *name == candidate.name && known == identity),
            CandidateSource::External { .. } => true,
        });
        Ok(entries.len() != before)
    }

    /// Убирает всех кандидатов watcher (watcher выключен).
    pub fn clear_watcher(&self) -> Result<(), AppError> {
        self.lock()?
            .retain(|candidate| candidate.origin != CandidateOrigin::Watcher);
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<DownloadCandidateDto>, AppError> {
        let entries = self.lock()?;
        let mut list: Vec<DownloadCandidateDto> =
            entries.iter().map(DownloadCandidateDto::from).collect();
        list.reverse();
        Ok(list)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<Candidate>>, AppError> {
        self.entries
            .lock()
            .map_err(|_| BackendError::database("Реестр кандидатов импорта повреждён"))
    }
}

pub(crate) fn now_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
