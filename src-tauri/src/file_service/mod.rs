mod atomic_copy;
mod backup;
mod fd;
mod hashing;
mod hooks;
mod managed_markdown;
mod paths;
mod planning;
mod read;
mod scan;
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
pub use managed_markdown::{write_managed_markdown, ManagedMarkdownWrite};
pub use paths::{
    BackupRoot, DownloadEntry, DownloadsRoot, FileIdentity, InboxRoot, PersistentPath, ScratchRoot,
    SourceFile, VaultRoot, VerifiedDestination,
};
pub use planning::{plan_copy, CopyDecision};
pub use read::{read_vault_file, ReadSnapshot};
pub use scan::{HashedFile, ScanLimits, ScanOutcome, ScannedEntry, ScannedKind};
pub use undo::{
    apply_created_files, stage_created_files, validate_created_undo, validate_undo,
    BackupUndoEntry, CreatedFileUndoEntry, StagedUndoFile, UndoContext, UndoManifest,
    ValidatedUndo,
};

#[cfg(test)]
mod race_tests;
#[cfg(test)]
mod tests;
