//! Только чтение файла внутри хранилища для извлечения текста (этап 7).
//!
//! Файл открывается fd-relative (`O_RDONLY | O_NOFOLLOW`, без symlink в
//! компонентах), содержимое и SHA-256 читаются из одного inode через `pread`.
//! Оригинал никогда не открывается на запись, не перемещается и не меняется.

use std::os::unix::fs::FileExt;

use sha2::{Digest, Sha256};

use crate::error::AppError;

use super::{
    paths::{io_error, SourceFile},
    PersistentPath, Sha256Digest, VaultRoot, VerifiedDestination,
};

/// Снимок содержимого. `bytes = None`, если файл больше лимита чтения в память
/// (тогда SHA-256 посчитан потоково).
#[derive(Debug, Clone)]
pub struct ReadSnapshot {
    pub size_bytes: u64,
    pub sha256: Sha256Digest,
    pub bytes: Option<Vec<u8>>,
}

pub fn read_vault_file(
    vault: &VaultRoot,
    relative: PersistentPath,
    max_bytes: u64,
) -> Result<ReadSnapshot, AppError> {
    let destination = VerifiedDestination::in_vault(vault, relative)?;
    let source = SourceFile::open_in(&destination)?;
    let size = source.revalidate()?;
    if size > max_bytes {
        let sha256 = super::hash_source(&source)?;
        return Ok(ReadSnapshot {
            size_bytes: size,
            sha256,
            bytes: None,
        });
    }
    let length = usize::try_from(size).map_err(|_| io_error("Файл слишком большой"))?;
    let mut bytes = vec![0_u8; length];
    let mut filled = 0_usize;
    while filled < length {
        let read = source
            .file()
            .read_at(&mut bytes[filled..], filled as u64)
            .map_err(|_| io_error("Не удалось прочитать файл"))?;
        if read == 0 {
            return Err(io_error("Файл изменился во время чтения"));
        }
        filled += read;
    }
    // Файл не должен вырасти во время чтения: иначе снимок неполный.
    let mut probe = [0_u8; 1];
    if source
        .file()
        .read_at(&mut probe, size)
        .map_err(|_| io_error("Не удалось прочитать файл"))?
        != 0
        || source.revalidate()? != size
    {
        return Err(io_error("Файл изменился во время чтения"));
    }
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(ReadSnapshot {
        size_bytes: size,
        sha256: Sha256Digest::from_hasher(hasher),
        bytes: Some(bytes),
    })
}
