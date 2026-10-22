mod atomic_copy;
mod backup;
mod fd;
mod hashing;
mod hooks;
mod paths;
mod planning;
mod read;
mod undo;

pub use atomic_copy::{
    atomic_copy, atomic_copy_committed, inspect_stored_file, CommittedCopy, CopyReceipt,
    StoredFileState,
};
pub use backup::{create_backup, BackupManifest};
#[cfg(test)]
pub(crate) use hashing::sha256_file;
pub use hashing::{hash_source, Sha256Digest};
#[cfg(test)]
pub(crate) use hooks::{install as install_test_hook, HookPoint as TestHookPoint};
pub use paths::{
    BackupRoot, DownloadEntry, DownloadsRoot, FileIdentity, InboxRoot, PersistentPath, ScratchRoot,
    SourceFile, VaultRoot, VerifiedDestination,
};
pub use planning::{plan_copy, CopyDecision};
pub use read::{read_vault_file, ReadSnapshot};
pub use undo::{
    validate_undo, BackupUndoEntry, CreatedFileUndoEntry, UndoContext, UndoManifest, ValidatedUndo,
};

#[cfg(test)]
mod race_tests;
#[cfg(test)]
mod tests;
