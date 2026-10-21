//! fd-relative примитивы файлового сервиса.
//!
//! Все операции внутри контролируемого корня выполняются относительно уже
//! открытых directory file descriptors. Каждый компонент открывается отдельно
//! через `openat(O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)`, поэтому symlink в любом
//! промежуточном компоненте отклоняется ядром, а последующая подмена пути не
//! перенаправляет уже начатую операцию в другой каталог.

use std::{
    ffi::OsStr,
    fs::File,
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::FileExt,
    },
    path::{Component, Path},
    sync::Arc,
};

use rustix::{
    fs::{self as rfs, AtFlags, FileType, Mode, OFlags, RenameFlags, Stat},
    io::Errno,
};
use sha2::{Digest, Sha256};

use crate::error::{AppError, BackendError};

use super::{
    hashing::Sha256Digest,
    paths::{blocked, io_error},
    PersistentPath,
};

const BUFFER_SIZE: usize = 64 * 1024;

/// Открытый каталог. `Arc` позволяет guard-ам удерживать тот же inode до cleanup.
pub(crate) type DirFd = Arc<OwnedFd>;

/// Идентичность inode: устройство + номер inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity {
    dev: u64,
    ino: u64,
}

impl Identity {
    // На macOS `st_dev` — i32, на Linux — u64; приведение нужно для единого сравнения.
    #[allow(clippy::unnecessary_cast)]
    pub(crate) fn of(stat: &Stat) -> Self {
        Self {
            dev: stat.st_dev as u64,
            ino: stat.st_ino as u64,
        }
    }
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

fn read_file_flags() -> OFlags {
    // NONBLOCK не даёт зависнуть на FIFO до проверки типа через fstat.
    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY
}

fn new_file_mode() -> Mode {
    Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH
}

fn new_directory_mode() -> Mode {
    Mode::RWXU | Mode::RGRP | Mode::XGRP | Mode::ROTH | Mode::XOTH
}

fn is_symlink_rejection(errno: Errno) -> bool {
    // ELOOP — O_NOFOLLOW встретил symlink; ENOTDIR — компонент не каталог.
    errno == Errno::LOOP || errno == Errno::NOTDIR || errno == Errno::MLINK
}

pub(crate) fn file_type(stat: &Stat) -> FileType {
    FileType::from_raw_mode(stat.st_mode)
}

pub(crate) fn stat_size(stat: &Stat) -> u64 {
    u64::try_from(stat.st_size).unwrap_or(0)
}

/// Открывает уже канонизированный абсолютный корень без следования symlink в последнем компоненте.
pub(crate) fn open_root_directory(path: &Path) -> Result<DirFd, AppError> {
    let fd = rfs::open(path, directory_flags(), Mode::empty()).map_err(|errno| {
        if is_symlink_rejection(errno) {
            blocked("Корень должен быть обычным каталогом")
        } else {
            blocked("Корень недоступен")
        }
    })?;
    ensure_directory(&fd)?;
    Ok(Arc::new(fd))
}

fn ensure_directory(fd: &OwnedFd) -> Result<(), AppError> {
    let stat = rfs::fstat(fd).map_err(|_| io_error("Не удалось проверить открытый каталог"))?;
    if file_type(&stat) != FileType::Directory {
        return Err(blocked("Компонент пути не является каталогом"));
    }
    Ok(())
}

pub(crate) fn fd_identity(fd: impl AsFd) -> Result<Identity, AppError> {
    rfs::fstat(fd)
        .map(|stat| Identity::of(&stat))
        .map_err(|_| io_error("Не удалось проверить открытый дескриптор"))
}

/// Открывает дочерний каталог строго внутри `dir`. `None` — компонент отсутствует.
pub(crate) fn open_child_directory(
    dir: &OwnedFd,
    name: &OsStr,
) -> Result<Option<OwnedFd>, AppError> {
    match rfs::openat(dir, name, directory_flags(), Mode::empty()) {
        Ok(fd) => {
            ensure_directory(&fd)?;
            Ok(Some(fd))
        }
        Err(Errno::NOENT) => Ok(None),
        Err(errno) if is_symlink_rejection(errno) => Err(blocked(
            "Компонент пути является symlink или не является каталогом",
        )),
        Err(_) => Err(io_error("Не удалось открыть каталог")),
    }
}

/// Открывает или создаёт (`mkdirat`) дочерний каталог внутри `dir`.
fn open_or_create_child_directory(dir: &OwnedFd, name: &OsStr) -> Result<OwnedFd, AppError> {
    if let Some(fd) = open_child_directory(dir, name)? {
        return Ok(fd);
    }
    match rfs::mkdirat(dir, name, new_directory_mode()) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(_) => return Err(io_error("Не удалось создать целевую папку")),
    }
    // Повторное открытие с O_NOFOLLOW: если конкурент успел положить symlink — отказ.
    open_child_directory(dir, name)?.ok_or_else(|| io_error("Созданная папка исчезла"))
}

fn normal_components(relative: &PersistentPath) -> Result<Vec<&OsStr>, AppError> {
    relative
        .as_path()
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name),
            _ => Err(blocked("небезопасный компонент целевого пути")),
        })
        .collect()
}

/// Обходит все компоненты, кроме последнего, и возвращает fd родительского каталога.
/// При `create = false` отсутствие компонента даёт `Ok(None)`.
pub(crate) fn walk_parent(
    root: &DirFd,
    relative: &PersistentPath,
    create: bool,
) -> Result<Option<DirFd>, AppError> {
    let components = normal_components(relative)?;
    let parents = &components[..components.len().saturating_sub(1)];
    walk(root, parents, create)
}

/// Обходит все компоненты относительного пути как каталоги, без создания.
pub(crate) fn walk_directory(
    root: &DirFd,
    relative: &PersistentPath,
) -> Result<Option<DirFd>, AppError> {
    let components = normal_components(relative)?;
    walk(root, &components, false)
}

fn walk(root: &DirFd, components: &[&OsStr], create: bool) -> Result<Option<DirFd>, AppError> {
    let mut current: DirFd = Arc::clone(root);
    for name in components {
        let next = if create {
            open_or_create_child_directory(&current, name)?
        } else {
            match open_child_directory(&current, name)? {
                Some(fd) => fd,
                None => return Ok(None),
            }
        };
        current = Arc::new(next);
    }
    Ok(Some(current))
}

/// Повторно обходит путь от корня и требует, чтобы родитель был тем же inode,
/// что и уже открытый `parent`. Обнаруживает подмену/перемещение каталога.
pub(crate) fn verify_parent_identity(
    root: &DirFd,
    relative: &PersistentPath,
    parent: &DirFd,
) -> Result<(), AppError> {
    let expected = fd_identity(parent.as_ref())?;
    let current = walk_parent(root, relative, false)
        .map_err(|_| blocked("Целевая папка была подменена во время операции"))?
        .ok_or_else(|| blocked("Целевая папка исчезла во время операции"))?;
    if fd_identity(current.as_ref())? != expected {
        return Err(blocked("Целевая папка была подменена во время операции"));
    }
    Ok(())
}

/// Состояние имени внутри каталога, проверенное через `statat(AT_SYMLINK_NOFOLLOW)`.
pub(crate) fn entry_exists(dir: &OwnedFd, name: &OsStr) -> Result<bool, AppError> {
    match rfs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(_) => Err(io_error("Не удалось проверить целевой файл")),
    }
}

pub(crate) enum Leaf {
    Missing,
    NotRegular,
    File(File, Stat),
}

/// Открывает конечный файл внутри `dir` только на чтение с O_NOFOLLOW.
pub(crate) fn open_leaf(dir: &OwnedFd, name: &OsStr) -> Result<Leaf, AppError> {
    let fd = match rfs::openat(dir, name, read_file_flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(Leaf::Missing),
        Err(errno) if is_symlink_rejection(errno) || errno == Errno::NXIO => {
            return Ok(Leaf::NotRegular)
        }
        Err(_) => return Err(io_error("Не удалось открыть файл")),
    };
    let stat = rfs::fstat(&fd).map_err(|_| io_error("Не удалось проверить открытый файл"))?;
    if file_type(&stat) != FileType::RegularFile {
        return Ok(Leaf::NotRegular);
    }
    Ok(Leaf::File(File::from(fd), stat))
}

/// Открывает абсолютный недоверенный source один раз; symlink в последнем компоненте отклоняется.
pub(crate) fn open_source_path(path: &Path) -> Result<(File, Stat), AppError> {
    let fd = rfs::open(path, read_file_flags(), Mode::empty()).map_err(|errno| match errno {
        Errno::NOENT => BackendError::not_found("Исходный файл не найден"),
        Errno::LOOP => blocked("Символические ссылки нельзя использовать как источник"),
        Errno::NXIO => blocked("Источник должен быть обычным файлом"),
        _ => io_error("Не удалось открыть исходный файл"),
    })?;
    let stat = rfs::fstat(&fd).map_err(|_| io_error("Не удалось проверить исходный файл"))?;
    if file_type(&stat) != FileType::RegularFile {
        return Err(blocked("Источник должен быть обычным файлом"));
    }
    Ok((File::from(fd), stat))
}

/// Создаёт новый файл `O_CREAT | O_EXCL | O_NOFOLLOW` внутри `dir`.
pub(crate) fn create_exclusive(dir: &OwnedFd, name: &OsStr) -> Result<File, AppError> {
    let flags = OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    match rfs::openat(dir, name, flags, new_file_mode()) {
        Ok(fd) => Ok(File::from(fd)),
        Err(Errno::EXIST) => Err(BackendError::conflict("Файл уже существует")),
        Err(_) => Err(io_error("Не удалось создать временный файл")),
    }
}

/// `renameat` внутри одного открытого каталога без перезаписи
/// (`RENAME_NOREPLACE` на Linux, `RENAME_EXCL` на macOS).
pub(crate) fn rename_noreplace(dir: &OwnedFd, from: &OsStr, to: &OsStr) -> Result<(), AppError> {
    match rfs::renameat_with(dir, from, dir, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(()),
        Err(Errno::EXIST) => Err(BackendError::conflict("Целевой файл уже существует")),
        Err(_) => Err(io_error("Не удалось атомарно опубликовать целевой файл")),
    }
}

/// Удаляет `name` из `dir` только если это тот же regular inode, что создала операция.
pub(crate) fn unlink_if_same(dir: &OwnedFd, name: &OsStr, identity: Identity) -> bool {
    let Ok(stat) = rfs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) else {
        return false;
    };
    if file_type(&stat) != FileType::RegularFile || Identity::of(&stat) != identity {
        return false;
    }
    rfs::unlinkat(dir, name, AtFlags::empty()).is_ok()
}

/// `statat` конечного имени без следования symlink.
pub(crate) fn stat_leaf(dir: &OwnedFd, name: &OsStr) -> Result<Option<Stat>, AppError> {
    match rfs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => Ok(Some(stat)),
        Err(Errno::NOENT) => Ok(None),
        Err(_) => Err(io_error("Не удалось проверить файл")),
    }
}

pub(crate) fn sync_directory(dir: &OwnedFd) -> Result<(), AppError> {
    rfs::fsync(dir).map_err(|_| io_error("Не удалось синхронизировать целевой каталог"))
}

/// Потоковый SHA-256 открытого файла позиционными чтениями (`pread`) от начала.
pub(crate) fn hash_open_file(file: &File) -> Result<(Sha256Digest, u64), AppError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; BUFFER_SIZE];
    let mut offset = 0_u64;
    loop {
        let read = file
            .read_at(&mut buffer, offset)
            .map_err(|_| io_error("Ошибка чтения при вычислении SHA-256"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        offset += read as u64;
    }
    Ok((Sha256Digest::from_hasher(hasher), offset))
}

/// Копирует открытый source в открытый temp одним проходом, хэшируя ровно скопированные байты.
pub(crate) fn copy_and_hash(
    source: &File,
    destination: &mut File,
) -> Result<(Sha256Digest, u64), AppError> {
    use std::io::Write;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; BUFFER_SIZE];
    let mut offset = 0_u64;
    loop {
        let read = source
            .read_at(&mut buffer, offset)
            .map_err(|_| io_error("Ошибка чтения исходного файла"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        destination
            .write_all(&buffer[..read])
            .map_err(|_| io_error("Ошибка записи временного файла"))?;
        offset += read as u64;
    }
    Ok((Sha256Digest::from_hasher(hasher), offset))
}
