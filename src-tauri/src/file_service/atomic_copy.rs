use std::{
    ffi::{OsStr, OsString},
    fs::File,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, BackendError};

use super::{
    fd::{self, DirFd, Identity},
    hashing::Sha256Digest,
    hooks::{self, HookPoint},
    paths::{blocked, io_error},
    SourceFile, VerifiedDestination,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyReceipt {
    pub destination: super::PersistentPath,
    pub sha256: Sha256Digest,
    pub size_bytes: u64,
}

/// Файл, созданный этой операцией внутри подтверждённого каталога.
///
/// Guard удерживает fd каталога и identity созданного inode. Cleanup выполняется
/// `unlinkat(dir_fd, name)` и только если имя всё ещё указывает на тот же regular inode;
/// абсолютный путь повторно не разрешается.
struct CreatedEntry {
    dir: DirFd,
    name: OsString,
    identity: Identity,
    keep: bool,
}

impl CreatedEntry {
    fn new(dir: &DirFd, name: OsString, identity: Identity) -> Self {
        Self {
            dir: DirFd::clone(dir),
            name,
            identity,
            keep: false,
        }
    }
}

impl Drop for CreatedEntry {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        hooks::fire(|| HookPoint::BeforeCleanup {
            file_name: self.name.clone(),
        });
        let _removed = fd::unlink_if_same(&self.dir, &self.name, self.identity);
    }
}

/// Temp-файл до публикации.
struct TempGuard(CreatedEntry);

/// Опубликованный файл до успешной финальной проверки.
struct FinalGuard(CreatedEntry);

pub fn atomic_copy(
    source: &SourceFile,
    destination: &VerifiedDestination,
) -> Result<CopyReceipt, AppError> {
    copy_inner(source, destination, None)
}

pub(crate) fn atomic_copy_expected(
    source: &SourceFile,
    destination: &VerifiedDestination,
    expected_hash: &Sha256Digest,
) -> Result<CopyReceipt, AppError> {
    copy_inner(source, destination, Some(expected_hash))
}

fn copy_inner(
    source: &SourceFile,
    destination: &VerifiedDestination,
    expected_hash: Option<&Sha256Digest>,
) -> Result<CopyReceipt, AppError> {
    let size_before = source.revalidate()?;
    let parent = destination.ensure_parent()?;
    let final_name = destination.file_name().to_owned();
    hooks::fire(|| HookPoint::BeforeTempCreate);

    destination.verify_parent(&parent)?;
    reject_existing(&parent, &final_name)?;

    let temp_name = OsString::from(format!(".kontur-{}.tmp", Uuid::new_v4()));
    let mut temp = fd::create_exclusive(&parent, &temp_name)?;
    let temp_identity = fd::fd_identity(&temp)?;
    let mut temp_guard = TempGuard(CreatedEntry::new(&parent, temp_name.clone(), temp_identity));

    let (copied_hash, copied) = fd::copy_and_hash(source.file(), &mut temp)?;
    temp.sync_all()
        .map_err(|_| io_error("Не удалось синхронизировать временный файл"))?;

    let size_after = source.revalidate()?;
    if copied != size_before || copied != size_after {
        return Err(io_error("Размер временной копии не совпадает с источником"));
    }
    if expected_hash.is_some_and(|expected| expected != &copied_hash) {
        return Err(io_error(
            "SHA-256 временной копии не совпадает с источником",
        ));
    }
    verify_open_file(&temp, &copied_hash, copied)?;

    hooks::fire(|| HookPoint::BeforeRename);
    destination.verify_parent(&parent)?;
    reject_existing(&parent, &final_name)?;
    fd::rename_noreplace(&parent, &temp_name, &final_name)?;
    temp_guard.0.keep = true;
    let mut final_guard = FinalGuard(CreatedEntry::new(
        &parent,
        final_name.clone(),
        temp_identity,
    ));

    hooks::fire(|| HookPoint::BeforeFinalVerify);
    destination.verify_parent(&parent)?;
    verify_published(&parent, &final_name, temp_identity, copied)?;
    verify_open_file(&temp, &copied_hash, copied)?;
    fd::sync_directory(&parent)?;
    final_guard.0.keep = true;

    Ok(CopyReceipt {
        destination: destination.relative().clone(),
        sha256: copied_hash,
        size_bytes: copied,
    })
}

fn reject_existing(parent: &DirFd, name: &OsStr) -> Result<(), AppError> {
    if fd::entry_exists(parent, name)? {
        return Err(BackendError::conflict("Целевой файл уже существует"));
    }
    Ok(())
}

/// Повторно хэширует содержимое через уже открытый дескриптор созданного inode.
fn verify_open_file(file: &File, expected: &Sha256Digest, size: u64) -> Result<(), AppError> {
    let (hash, read) = fd::hash_open_file(file)?;
    if &hash != expected || read != size {
        return Err(io_error("Проверка созданного файла не пройдена"));
    }
    Ok(())
}

/// Проверяет, что опубликованное имя в подтверждённом каталоге — тот же regular inode.
fn verify_published(
    parent: &DirFd,
    name: &OsStr,
    identity: Identity,
    size: u64,
) -> Result<(), AppError> {
    let stat = fd::stat_leaf(parent, name)?.ok_or_else(|| io_error("Опубликованный файл исчез"))?;
    if fd::file_type(&stat) != rustix::fs::FileType::RegularFile || Identity::of(&stat) != identity
    {
        return Err(blocked("Опубликованный файл был подменён"));
    }
    if fd::stat_size(&stat) != size {
        return Err(io_error("Проверка созданного файла не пройдена"));
    }
    Ok(())
}
