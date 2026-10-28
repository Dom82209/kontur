//! Только чтение: обход дерева vault для adoption (production workflow, этап 2).
//!
//! Обход выполняется fd-relative: каждый каталог открывается `openat(O_DIRECTORY |
//! O_NOFOLLOW)` относительно уже открытого родителя, тип записи определяется
//! `statat(AT_SYMLINK_NOFOLLOW)`. Symlink, специальные файлы и имена не в UTF-8 не
//! открываются, а попадают в отчёт как пропущенные. Ничего не создаётся и не
//! изменяется; файлы открываются только `O_RDONLY | O_NOFOLLOW` для SHA-256.

use std::{ffi::OsStr, os::fd::OwnedFd};

use rustix::fs::{self as rfs, AtFlags, FileType};

use crate::error::{AppError, BackendError};

use super::{
    fd::{self, Identity},
    paths::io_error,
    FileIdentity, PersistentPath, Sha256Digest, VaultRoot,
};

/// Тип записи дерева. Открываются только `File` и `Directory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannedKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedEntry {
    /// Путь относительно корня vault, компоненты через `/`.
    pub relative: String,
    pub kind: ScannedKind,
    pub size_bytes: u64,
    pub modified_ns: i128,
    pub identity: FileIdentity,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ScanLimits {
    pub max_entries: usize,
    pub max_depth: usize,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_entries: 50_000,
            max_depth: 12,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScanOutcome {
    pub entries: Vec<ScannedEntry>,
    /// Имена не в UTF-8 (путь не может быть сохранён безопасно).
    pub non_utf8: Vec<String>,
    /// Каталоги, которые не удалось открыть (подмена во время обхода, права).
    pub unreadable: Vec<String>,
    pub truncated: bool,
}

/// Снимок содержимого файла, прочитанного без следования symlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashedFile {
    pub sha256: Sha256Digest,
    pub size_bytes: u64,
    pub identity: FileIdentity,
}

impl VaultRoot {
    /// Перечисляет дерево под `start` (или весь vault). Скрытые записи (`.obsidian`,
    /// `.trash`, `.DS_Store`, marker «Контура») не перечисляются и не открываются.
    pub fn scan_tree(
        &self,
        start: Option<&PersistentPath>,
        limits: ScanLimits,
    ) -> Result<ScanOutcome, AppError> {
        let root = self.controlled().fd();
        let (directory, prefix) = match start {
            None => (root.clone(), String::new()),
            Some(relative) => match fd::walk_directory(root, relative)? {
                Some(directory) => (directory, relative.as_str().to_owned()),
                None => return Ok(ScanOutcome::default()),
            },
        };
        let mut outcome = ScanOutcome::default();
        walk(&directory, &prefix, 0, limits, &mut outcome)?;
        outcome
            .entries
            .sort_by(|left, right| left.relative.cmp(&right.relative));
        Ok(outcome)
    }

    /// Потоковый SHA-256 файла внутри vault (`O_RDONLY | O_NOFOLLOW`, без symlink в
    /// промежуточных компонентах). Отсутствие или не-обычный файл — ошибка.
    pub fn hash_file(&self, relative: &PersistentPath) -> Result<HashedFile, AppError> {
        let parent = self
            .controlled()
            .open_existing_parent(relative)?
            .ok_or_else(|| BackendError::not_found("Файл не найден в хранилище"))?;
        match fd::open_leaf(&parent, relative.file_name())? {
            fd::Leaf::Missing => Err(BackendError::not_found("Файл не найден в хранилище")),
            fd::Leaf::NotRegular => Err(BackendError::blocked(
                "Путь в хранилище не является обычным файлом",
            )),
            fd::Leaf::File(file, stat) => {
                let (sha256, size_bytes) = fd::hash_open_file(&file)?;
                if size_bytes != fd::stat_size(&stat) {
                    return Err(io_error("Файл изменился во время чтения"));
                }
                self.controlled().verify_parent(relative, &parent)?;
                Ok(HashedFile {
                    sha256,
                    size_bytes,
                    identity: FileIdentity::of(Identity::of(&stat)),
                })
            }
        }
    }
}

fn walk(
    directory: &OwnedFd,
    prefix: &str,
    depth: usize,
    limits: ScanLimits,
    outcome: &mut ScanOutcome,
) -> Result<(), AppError> {
    let reader = rfs::Dir::read_from(directory)
        .map_err(|_| io_error("Не удалось прочитать папку хранилища"))?;
    let mut names = Vec::new();
    for entry in reader {
        let entry = entry.map_err(|_| io_error("Не удалось прочитать папку хранилища"))?;
        let raw = entry.file_name().to_bytes();
        if raw == b"." || raw == b".." || raw.first() == Some(&b'.') {
            continue;
        }
        match std::str::from_utf8(raw) {
            Ok(name) => names.push(name.to_owned()),
            Err(_) => outcome
                .non_utf8
                .push(join(prefix, &String::from_utf8_lossy(raw))),
        }
    }
    names.sort();
    for name in names {
        if outcome.entries.len() >= limits.max_entries {
            outcome.truncated = true;
            return Ok(());
        }
        let relative = join(prefix, &name);
        let Ok(stat) = rfs::statat(directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW) else {
            outcome.unreadable.push(relative);
            continue;
        };
        let kind = match fd::file_type(&stat) {
            FileType::RegularFile => ScannedKind::File,
            FileType::Directory => ScannedKind::Directory,
            FileType::Symlink => ScannedKind::Symlink,
            _ => ScannedKind::Other,
        };
        outcome.entries.push(ScannedEntry {
            relative: relative.clone(),
            kind,
            size_bytes: if kind == ScannedKind::File {
                fd::stat_size(&stat)
            } else {
                0
            },
            modified_ns: fd::stat_mtime_ns(&stat),
            identity: FileIdentity::of(Identity::of(&stat)),
            depth,
        });
        if kind != ScannedKind::Directory {
            continue;
        }
        if depth + 1 >= limits.max_depth {
            outcome.truncated = true;
            continue;
        }
        // Повторное открытие с O_NOFOLLOW: если каталог подменили symlink — отказ.
        match fd::open_child_directory(directory, OsStr::new(&name)) {
            Ok(Some(child)) => {
                if Identity::of(&rfs::fstat(&child).map_err(|_| io_error("fstat"))?)
                    != Identity::of(&stat)
                {
                    outcome.unreadable.push(relative);
                    continue;
                }
                walk(&child, &relative, depth + 1, limits, outcome)?;
                if outcome.truncated && outcome.entries.len() >= limits.max_entries {
                    return Ok(());
                }
            }
            Ok(None) | Err(_) => outcome.unreadable.push(relative),
        }
    }
    Ok(())
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}
