use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, BackendError, ErrorCode};

use super::{
    sha256_file, BackupRoot, PersistentPath, Sha256Digest, VaultRoot, VerifiedDestination,
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
        return Err(BackendError::new(
            ErrorCode::NotUndoable,
            "Операцию нельзя отменить",
        ));
    }
    if manifest.operation_id.is_empty() {
        return Err(BackendError::new(
            ErrorCode::NotUndoable,
            "Undo manifest не содержит идентификатор операции",
        ));
    }
    for entry in &manifest.created_files {
        let destination = VerifiedDestination::in_vault(vault, entry.path.clone())?;
        validate_regular_file(&destination)?;
        if sha256_file(&destination.absolute())? != entry.expected_sha256 {
            return Err(BackendError::new(
                ErrorCode::NotUndoable,
                "Созданный операцией файл был изменён",
            ));
        }
    }
    for entry in &manifest.backups {
        let original = VerifiedDestination::in_vault(vault, entry.original_path.clone())?;
        original.revalidate_parents()?;
        let backup = VerifiedDestination::in_backup(backup_root, entry.backup_path.clone())?;
        validate_regular_file(&backup)?;
        if sha256_file(&backup.absolute())? != entry.expected_backup_sha256 {
            return Err(BackendError::new(
                ErrorCode::NotUndoable,
                "Резервная копия отсутствует или была изменена",
            ));
        }
    }
    Ok(ValidatedUndo {
        manifest: manifest.clone(),
    })
}

fn validate_regular_file(destination: &VerifiedDestination) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(destination.absolute())
        .map_err(|_| BackendError::new(ErrorCode::NotUndoable, "Файл для undo не найден"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(BackendError::new(
            ErrorCode::NotUndoable,
            "Путь undo не является обычным файлом",
        ));
    }
    Ok(())
}
