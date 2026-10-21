use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, BackendError};

use super::{
    atomic_copy::atomic_copy_expected, hash_source, plan_copy, BackupRoot, CopyDecision,
    PersistentPath, SourceFile, VaultRoot, VerifiedDestination,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub operation_id: String,
    pub source_path: PersistentPath,
    pub backup_path: PersistentPath,
    pub sha256: super::Sha256Digest,
    pub size_bytes: u64,
    pub created_at: String,
}

pub fn create_backup(
    operation_id: &str,
    vault: &VaultRoot,
    source_path: PersistentPath,
    backup_root: &BackupRoot,
) -> Result<BackupManifest, AppError> {
    validate_operation_id(operation_id)?;
    let source_destination = VerifiedDestination::in_vault(vault, source_path.clone())?;
    let source = SourceFile::open_in(&source_destination)?;
    let backup_path = PersistentPath::new(format!("{operation_id}/{}", source_path.as_str()))?;
    let backup_destination = VerifiedDestination::in_backup(backup_root, backup_path.clone())?;
    match plan_copy(&source, &backup_destination)? {
        CopyDecision::Copy => {}
        CopyDecision::SameContent | CopyDecision::NameConflict { .. } => {
            return Err(BackendError::conflict(
                "Резервная копия для операции уже существует",
            ))
        }
    }
    // Хэш и копия относятся к одному открытому inode; изменение содержимого между
    // ними обнаруживается сравнением с ожидаемым SHA-256.
    let expected = hash_source(&source)?;
    let receipt = atomic_copy_expected(&source, &backup_destination, &expected)?;
    let created_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| super::paths::io_error("Не удалось сформировать время резервной копии"))?;
    Ok(BackupManifest {
        operation_id: operation_id.to_owned(),
        source_path,
        backup_path,
        sha256: receipt.sha256,
        size_bytes: receipt.size_bytes,
        created_at,
    })
}

fn validate_operation_id(value: &str) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(super::paths::blocked("Некорректный идентификатор операции"));
    }
    Ok(())
}
