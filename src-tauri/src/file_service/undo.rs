use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use uuid::Uuid;

use crate::error::{AppError, BackendError, ErrorCode};

use super::{
    fd::{self, Leaf},
    hooks::{self, HookPoint},
    BackupRoot, PersistentPath, Sha256Digest, VaultRoot, VerifiedDestination,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedFileUndoEntry {
    pub path: PersistentPath,
    pub expected_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupUndoEntry {
    pub original_path: PersistentPath,
    pub backup_path: PersistentPath,
    pub expected_backup_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoManifest {
    pub operation_id: String,
    pub created_files: Vec<CreatedFileUndoEntry>,
    pub backups: Vec<BackupUndoEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoContext {
    pub undoable: bool,
    pub already_undone: bool,
    pub submitted_homework: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedUndo {
    manifest: UndoManifest,
}

pub struct StagedUndoFile {
    parent: super::fd::DirFd,
    original: OsString,
    tombstone: OsString,
    identity: super::fd::Identity,
    committed: bool,
}

impl StagedUndoFile {
    pub fn commit(mut self) -> Result<(), AppError> {
        if !fd::unlink_if_same(&self.parent, &self.tombstone, self.identity) {
            return Err(not_undoable("Временный файл undo был изменён"));
        }
        fd::sync_directory(&self.parent)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for StagedUndoFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fd::rename_noreplace(&self.parent, &self.tombstone, &self.original);
            let _ = fd::sync_directory(&self.parent);
        }
    }
}

/// Применяет уже проверенный manifest. Удаляет только созданный операцией inode,
/// хэш которого всё ещё совпадает. Восстановление backup выполняется отдельной
/// операцией: смешивать удаление и замену файлов без журнала нельзя.
pub fn apply_created_files(validated: &ValidatedUndo, vault: &VaultRoot) -> Result<(), AppError> {
    if !validated.manifest.backups.is_empty() {
        return Err(not_undoable(
            "Undo с восстановлением резервной копии требует отдельного подтверждения",
        ));
    }
    for entry in validated.manifest.created_files.iter().rev() {
        let destination = VerifiedDestination::in_vault(vault, entry.path.clone())?;
        let parent = destination
            .open_existing_parent()?
            .ok_or_else(|| not_undoable("Файл для undo не найден"))?;
        let (identity, hash) = match fd::open_leaf(&parent, destination.file_name())? {
            Leaf::File(file, stat) => {
                let (hash, _) = fd::hash_open_file(&file)?;
                (fd::Identity::of(&stat), hash)
            }
            _ => return Err(not_undoable("Файл для undo не найден")),
        };
        destination.verify_parent(&parent)?;
        if hash != entry.expected_sha256 {
            return Err(not_undoable("Созданный операцией файл был изменён"));
        }
        if !fd::unlink_if_same(&parent, destination.file_name(), identity) {
            return Err(not_undoable("Файл изменился во время undo"));
        }
        fd::sync_directory(&parent)?;
    }
    Ok(())
}

/// Обратимо убирает созданные файлы под служебные имена в тех же каталогах.
/// Drop возвращает исходные имена при rollback; `commit` окончательно удаляет.
pub fn stage_created_files(
    validated: &ValidatedUndo,
    vault: &VaultRoot,
) -> Result<Vec<StagedUndoFile>, AppError> {
    if !validated.manifest.backups.is_empty() {
        return Err(not_undoable("Undo содержит резервные копии"));
    }
    let mut staged = Vec::new();
    for entry in validated.manifest.created_files.iter().rev() {
        let destination = VerifiedDestination::in_vault(vault, entry.path.clone())?;
        let parent = destination
            .open_existing_parent()?
            .ok_or_else(|| not_undoable("Файл для undo не найден"))?;
        let (identity, hash) = match fd::open_leaf(&parent, destination.file_name())? {
            Leaf::File(file, stat) => (fd::Identity::of(&stat), fd::hash_open_file(&file)?.0),
            _ => return Err(not_undoable("Файл для undo не найден")),
        };
        destination.verify_parent(&parent)?;
        if hash != entry.expected_sha256 {
            return Err(not_undoable("Созданный операцией файл был изменён"));
        }
        let tombstone = OsString::from(format!(".kontur-undo-{}.tmp", Uuid::new_v4()));
        fd::rename_noreplace(&parent, destination.file_name(), &tombstone)?;
        staged.push(StagedUndoFile {
            parent,
            original: destination.file_name().to_owned(),
            tombstone,
            identity,
            committed: false,
        });
    }
    Ok(staged)
}

impl ValidatedUndo {
    pub fn manifest(&self) -> &UndoManifest {
        &self.manifest
    }
}

pub fn validate_undo(
    manifest: &UndoManifest,
    context: UndoContext,
    vault: &VaultRoot,
    backup_root: &BackupRoot,
) -> Result<ValidatedUndo, AppError> {
    validate_created_entries(manifest, context, vault)?;
    for entry in &manifest.backups {
        let original = VerifiedDestination::in_vault(vault, entry.original_path.clone())?;
        if let Some(parent) = original.open_existing_parent()? {
            hooks::fire(|| HookPoint::UndoParentOpened);
            original.verify_parent(&parent)?;
        }
        let backup = VerifiedDestination::in_backup(backup_root, entry.backup_path.clone())?;
        verify_file_hash(
            &backup,
            &entry.expected_backup_sha256,
            "Резервная копия отсутствует или была изменена",
        )?;
    }
    Ok(ValidatedUndo {
        manifest: manifest.clone(),
    })
}

/// Проверка undo для операций, которые только создавали файлы и не имеют backup.
pub fn validate_created_undo(
    manifest: &UndoManifest,
    context: UndoContext,
    vault: &VaultRoot,
) -> Result<ValidatedUndo, AppError> {
    if !manifest.backups.is_empty() {
        return Err(not_undoable(
            "Undo manifest требует проверки резервной копии",
        ));
    }
    validate_created_entries(manifest, context, vault)?;
    Ok(ValidatedUndo {
        manifest: manifest.clone(),
    })
}

fn validate_created_entries(
    manifest: &UndoManifest,
    context: UndoContext,
    vault: &VaultRoot,
) -> Result<(), AppError> {
    if !context.undoable || context.already_undone || context.submitted_homework {
        return Err(not_undoable("Операцию нельзя отменить"));
    }
    if manifest.operation_id.is_empty() {
        return Err(not_undoable(
            "Undo manifest не содержит идентификатор операции",
        ));
    }
    for entry in &manifest.created_files {
        let destination = VerifiedDestination::in_vault(vault, entry.path.clone())?;
        verify_file_hash(
            &destination,
            &entry.expected_sha256,
            "Созданный операцией файл был изменён",
        )?;
    }
    Ok(())
}

/// Открывает файл fd-relative (O_NOFOLLOW в каждом компоненте), хэширует открытый inode
/// и подтверждает, что родительский каталог не был подменён во время проверки.
fn verify_file_hash(
    destination: &VerifiedDestination,
    expected: &Sha256Digest,
    changed_message: &str,
) -> Result<(), AppError> {
    let parent = destination
        .open_existing_parent()?
        .ok_or_else(|| not_undoable("Файл для undo не найден"))?;
    hooks::fire(|| HookPoint::UndoParentOpened);
    let file = match fd::open_leaf(&parent, destination.file_name())? {
        Leaf::Missing => return Err(not_undoable("Файл для undo не найден")),
        Leaf::NotRegular => return Err(not_undoable("Путь undo не является обычным файлом")),
        Leaf::File(file, _) => file,
    };
    let (hash, _) = fd::hash_open_file(&file)?;
    destination.verify_parent(&parent)?;
    if &hash != expected {
        return Err(not_undoable(changed_message));
    }
    Ok(())
}

fn not_undoable(message: &str) -> AppError {
    BackendError::new(ErrorCode::NotUndoable, message)
}
