use std::{ffi::OsString, io::Write};
use uuid::Uuid;

use super::{
    fd::{self, Identity, Leaf},
    PersistentPath, VaultRoot, VerifiedDestination,
};
use crate::error::{AppError, BackendError};

const START: &str = "<!-- kontur:start -->";
const END: &str = "<!-- kontur:end -->";
const MAX_NOTE: u64 = 4 * 1024 * 1024;

/// Запись управляемого markdown-блока. Пользовательский текст вне маркеров сохраняется.
/// Объект можно компенсировать, пока связанная транзакция SQLite не завершена.
pub struct ManagedMarkdownWrite {
    destination: VerifiedDestination,
    previous: Option<Vec<u8>>,
    identity: Identity,
}
impl ManagedMarkdownWrite {
    pub fn compensate(self) -> bool {
        match self.previous {
            Some(bytes) => write_bytes(&self.destination, &bytes).is_ok(),
            None => self
                .destination
                .open_existing_parent()
                .ok()
                .flatten()
                .is_some_and(|parent| {
                    fd::unlink_if_same(&parent, self.destination.file_name(), self.identity)
                }),
        }
    }
}

pub fn write_managed_markdown(
    vault: &VaultRoot,
    path: PersistentPath,
    managed: &str,
) -> Result<ManagedMarkdownWrite, AppError> {
    let destination = VerifiedDestination::in_vault(vault, path)?;
    let previous = read_existing(&destination)?;
    let old = previous
        .as_deref()
        .map(|b| {
            std::str::from_utf8(b)
                .map_err(|_| BackendError::blocked("Существующая заметка не является UTF-8"))
        })
        .transpose()?;
    let block = format!("{START}\n{}\n{END}", managed.trim());
    let next = match old {
        None => format!("{block}\n"),
        Some(text) => replace_block(text, &block)?,
    };
    let identity = write_bytes(&destination, next.as_bytes())?;
    Ok(ManagedMarkdownWrite {
        destination,
        previous,
        identity,
    })
}
fn replace_block(text: &str, block: &str) -> Result<String, AppError> {
    match (text.find(START), text.find(END)) {
        (None, None) => Ok(format!(
            "{}{}{}\n",
            text,
            if text.ends_with('\n') { "" } else { "\n" },
            block
        )),
        (Some(a), Some(b)) if b >= a => {
            let end = b + END.len();
            Ok(format!("{}{}{}", &text[..a], block, &text[end..]))
        }
        _ => Err(BackendError::blocked(
            "Маркеры управляемого блока заметки повреждены",
        )),
    }
}
fn read_existing(dest: &VerifiedDestination) -> Result<Option<Vec<u8>>, AppError> {
    let Some(parent) = dest.open_existing_parent()? else {
        return Ok(None);
    };
    let bytes = match fd::open_leaf(&parent, dest.file_name())? {
        Leaf::Missing => None,
        Leaf::NotRegular => {
            return Err(BackendError::blocked(
                "Путь заметки не является обычным файлом",
            ))
        }
        Leaf::File(mut file, stat) => {
            if fd::stat_size(&stat) > MAX_NOTE {
                return Err(BackendError::blocked(
                    "Заметка слишком большая для безопасного обновления",
                ));
            }
            use std::io::Read;
            let mut b = Vec::new();
            file.read_to_end(&mut b)
                .map_err(|_| BackendError::io("Не удалось прочитать заметку"))?;
            Some(b)
        }
    };
    dest.verify_parent(&parent)?;
    Ok(bytes)
}
fn write_bytes(dest: &VerifiedDestination, bytes: &[u8]) -> Result<Identity, AppError> {
    let parent = dest.ensure_parent()?;
    dest.verify_parent(&parent)?;
    let temp = OsString::from(format!(".kontur-note-{}.tmp", Uuid::new_v4()));
    let mut file = fd::create_exclusive(&parent, &temp)?;
    file.write_all(bytes)
        .map_err(|_| BackendError::io("Не удалось записать заметку"))?;
    file.sync_all()
        .map_err(|_| BackendError::io("Не удалось синхронизировать заметку"))?;
    let identity = fd::fd_identity(&file)?;
    match fd::open_leaf(&parent, dest.file_name())? {
        Leaf::Missing => fd::rename_noreplace(&parent, &temp, dest.file_name())?,
        Leaf::File(_, _) => fd::rename_replace(&parent, &temp, dest.file_name())?,
        Leaf::NotRegular => return Err(BackendError::blocked("Путь заметки небезопасен")),
    };
    dest.verify_parent(&parent)?;
    fd::sync_directory(&parent)?;
    Ok(identity)
}
