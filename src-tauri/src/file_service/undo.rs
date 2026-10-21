use serde::{Deserialize, Serialize};

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
