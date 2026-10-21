use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use rustix::fs::{renameat_with, RenameFlags, CWD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, BackendError};

use super::{
    hashing::{sha256_file, Sha256Digest},
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

struct TempGuard {
    path: PathBuf,
    published: bool,
}

struct FinalGuard {
    path: PathBuf,
    verified: bool,
}

impl Drop for FinalGuard {
    fn drop(&mut self) {
        if !self.verified {
            let _cleanup_result = fs::remove_file(&self.path);
        }
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if !self.published {
            let _cleanup_result = fs::remove_file(&self.path);
        }
    }
}

pub fn atomic_copy(
    source: &SourceFile,
    destination: &VerifiedDestination,
) -> Result<CopyReceipt, AppError> {
    let expected_hash = sha256_file(source.path())?;
    atomic_copy_expected(source, destination, &expected_hash)
}

pub(crate) fn atomic_copy_expected(
    source: &SourceFile,
    destination: &VerifiedDestination,
    expected_hash: &Sha256Digest,
) -> Result<CopyReceipt, AppError> {
    let expected_size = source.revalidate()?;
    destination.revalidate_parents()?;
    reject_existing_destination(&destination.absolute())?;
    let parent = destination.ensure_parent()?;
    destination.revalidate_parents()?;
    reject_existing_destination(&destination.absolute())?;

    let mut source_file = File::open(source.path())
        .map_err(|_| io_error("Не удалось открыть исходный файл для чтения"))?;
    if !source_file
        .metadata()
        .map_err(|_| io_error("Не удалось проверить открытый исходный файл"))?
        .is_file()
    {
        return Err(blocked("Источник больше не является обычным файлом"));
    }

    let temp_path = parent.join(format!(".kontur-{}.tmp", Uuid::new_v4()));
    let mut guard = TempGuard {
        path: temp_path.clone(),
        published: false,
    };
    let mut temp = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|_| io_error("Не удалось создать временный файл"))?;

    let copied = copy_stream(&mut source_file, &mut temp)?;
    temp.flush()
        .map_err(|_| io_error("Не удалось записать временный файл"))?;
    temp.sync_all()
        .map_err(|_| io_error("Не удалось синхронизировать временный файл"))?;
    drop(temp);

    if copied != expected_size {
        return Err(io_error("Размер временной копии не совпадает с источником"));
    }
    let copied_hash = sha256_file(&temp_path)?;
    if &copied_hash != expected_hash {
        return Err(io_error(
            "SHA-256 временной копии не совпадает с источником",
        ));
    }

    destination.revalidate_parents()?;
    reject_existing_destination(&destination.absolute())?;
    let final_path = destination.absolute();
    renameat_with(CWD, &temp_path, CWD, &final_path, RenameFlags::NOREPLACE).map_err(|_| {
        if fs::symlink_metadata(&final_path).is_ok() {
            BackendError::conflict("Целевой файл уже существует")
        } else {
            io_error("Не удалось атомарно опубликовать целевой файл")
        }
    })?;
    let mut final_guard = FinalGuard {
        path: final_path.clone(),
        verified: false,
    };
    guard.published = true;

    destination.revalidate_parents()?;
    let final_metadata = fs::symlink_metadata(&final_path)
        .map_err(|_| io_error("Не удалось проверить созданный файл"))?;
    if final_metadata.file_type().is_symlink()
        || !final_metadata.is_file()
        || final_metadata.len() != expected_size
        || sha256_file(&final_path)? != copied_hash
    {
        return Err(io_error("Проверка созданного файла не пройдена"));
    }
    File::open(&parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| io_error("Не удалось синхронизировать целевой каталог"))?;
    final_guard.verified = true;
    Ok(CopyReceipt {
        destination: destination.relative().clone(),
        sha256: copied_hash,
        size_bytes: copied,
    })
}

fn copy_stream(source: &mut File, destination: &mut File) -> Result<u64, AppError> {
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|_| io_error("Ошибка чтения исходного файла"))?;
        if read == 0 {
            return Ok(total);
        }
        destination
            .write_all(&buffer[..read])
            .map_err(|_| io_error("Ошибка записи временного файла"))?;
        total += read as u64;
    }
}

fn reject_existing_destination(path: &Path) -> Result<(), AppError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(BackendError::conflict("Целевой файл уже существует")),
        Err(_) => Err(io_error("Не удалось проверить целевой файл")),
    }
}
