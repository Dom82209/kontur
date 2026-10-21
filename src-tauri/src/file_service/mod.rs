mod atomic_copy;
mod backup;
mod fd;
mod hashing;
mod hooks;
mod paths;
mod planning;
mod undo;

pub use atomic_copy::{atomic_copy, CopyReceipt};
pub use backup::{create_backup, BackupManifest};
#[cfg(test)]
pub(crate) use hashing::sha256_file;
pub use hashing::{hash_source, Sha256Digest};
pub use paths::{
    BackupRoot, InboxRoot, PersistentPath, SourceFile, VaultRoot, VerifiedDestination,
};
pub use planning::{plan_copy, CopyDecision};
pub use undo::{
    validate_undo, BackupUndoEntry, CreatedFileUndoEntry, UndoContext, UndoManifest, ValidatedUndo,
};

#[cfg(test)]
mod race_tests;
#[cfg(test)]
mod tests;
