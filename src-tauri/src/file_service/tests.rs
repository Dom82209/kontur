use std::{fs, path::Path};

use serde_json::json;
use tempfile::TempDir;

use crate::{db::migrations::CURRENT_SCHEMA_VERSION, error::ErrorCode};

use super::{
    atomic_copy, backup::create_backup, hashing::Sha256Digest, plan_copy, sha256_file,
    undo::validate_undo, BackupRoot, BackupUndoEntry, CopyDecision, CreatedFileUndoEntry,
    PersistentPath, SourceFile, UndoContext, UndoManifest, VaultRoot, VerifiedDestination,
};

struct Fixture {
    _temp: TempDir,
    vault: VaultRoot,
    backup: BackupRoot,
    vault_path: std::path::PathBuf,
    backup_path: std::path::PathBuf,
    source_path: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        let backup_path = temp.path().join("backups");
        let sources = temp.path().join("sources");
        fs::create_dir_all(&vault_path).unwrap();
        fs::create_dir_all(&backup_path).unwrap();
        fs::create_dir_all(&sources).unwrap();
        let source_path = sources.join("source.pdf");
        fs::write(&source_path, b"source contents").unwrap();
        let vault = VaultRoot::new(&vault_path, None).unwrap();
        let backup = BackupRoot::new(&backup_path, None).unwrap();
        Self {
            _temp: temp,
            vault,
            backup,
            vault_path,
            backup_path,
            source_path,
        }
    }

    fn source(&self) -> SourceFile {
        SourceFile::new(&self.source_path).unwrap()
    }

    fn destination(&self, path: &str) -> VerifiedDestination {
        VerifiedDestination::in_vault(&self.vault, PersistentPath::new(path).unwrap()).unwrap()
    }
}

#[test]
fn accepts_absolute_safe_vault_root() {
    let fixture = Fixture::new();
    assert!(fixture.vault_path.is_absolute());
}

#[test]
fn rejects_dot_as_production_vault() {
    assert_eq!(
        VaultRoot::new(Path::new("."), None).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn rejects_filesystem_root_without_touching_user_data() {
    assert_eq!(
        VaultRoot::new(Path::new("/"), None).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn rejects_configured_home_directory() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    assert_eq!(
        VaultRoot::new(&home, Some(&home)).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn rejects_relative_source() {
    assert_eq!(
        SourceFile::new(Path::new("file.pdf")).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn missing_source_is_not_found_without_exposing_path() {
    let temp = tempfile::tempdir().unwrap();
    let error = SourceFile::new(&temp.path().join("missing.pdf")).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(!error
        .message
        .contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn rejects_source_directory() {
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        SourceFile::new(temp.path()).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[cfg(unix)]
#[test]
fn rejects_source_symlink() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let link = fixture.source_path.with_file_name("link.pdf");
    symlink(&fixture.source_path, &link).unwrap();
    assert_eq!(SourceFile::new(&link).unwrap_err().code, ErrorCode::Blocked);
}

#[test]
fn rejects_parent_traversal() {
    assert_eq!(
        PersistentPath::new("Предметы/../secret").unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn rejects_absolute_persistent_path() {
    assert_eq!(
        PersistentPath::new("/tmp/file").unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn rejects_file_url_persistent_path() {
    assert_eq!(
        PersistentPath::new("file:///tmp/file").unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[cfg(unix)]
#[test]
fn rejects_destination_symlink_escape() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = fixture._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, fixture.vault_path.join("escape")).unwrap();
    let error = VerifiedDestination::in_vault(
        &fixture.vault,
        PersistentPath::new("escape/file.pdf").unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
}

#[test]
fn hashes_empty_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("empty");
    fs::write(&file, []).unwrap();
    assert_eq!(
        sha256_file(&file).unwrap().as_str(),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn hashes_known_content() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("known");
    fs::write(&file, b"abc").unwrap();
    assert_eq!(
        sha256_file(&file).unwrap().as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn hashes_large_file_streamingly() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("large");
    let content = vec![0x5a; 3 * 1024 * 1024 + 17];
    fs::write(&file, &content).unwrap();
    let digest = sha256_file(&file).unwrap();
    assert_eq!(digest.as_str().len(), 64);
    assert_eq!(digest, sha256_file(&file).unwrap());
}

#[test]
fn plans_copy_for_missing_destination() {
    let fixture = Fixture::new();
    assert_eq!(
        plan_copy(&fixture.source(), &fixture.destination("Предметы/file.pdf")).unwrap(),
        CopyDecision::Copy
    );
}

#[test]
fn plans_same_content_without_duplicate() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("Предметы")).unwrap();
    fs::copy(
        &fixture.source_path,
        fixture.vault_path.join("Предметы/file.pdf"),
    )
    .unwrap();
    assert_eq!(
        plan_copy(&fixture.source(), &fixture.destination("Предметы/file.pdf")).unwrap(),
        CopyDecision::SameContent
    );
}

#[test]
fn plans_name_conflict_for_different_content() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("Предметы")).unwrap();
    fs::write(fixture.vault_path.join("Предметы/file.pdf"), b"other").unwrap();
    assert_eq!(
        plan_copy(&fixture.source(), &fixture.destination("Предметы/file.pdf")).unwrap(),
        CopyDecision::NameConflict {
            suggested_name: "file (2).pdf".to_owned()
        }
    );
}

#[test]
fn suggested_name_supports_cyrillic() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("Предметы")).unwrap();
    fs::write(fixture.vault_path.join("Предметы/Лекция.pdf"), b"other").unwrap();
    assert_eq!(
        plan_copy(
            &fixture.source(),
            &fixture.destination("Предметы/Лекция.pdf")
        )
        .unwrap(),
        CopyDecision::NameConflict {
            suggested_name: "Лекция (2).pdf".to_owned()
        }
    );
}

#[test]
fn suggested_name_preserves_extension() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("work")).unwrap();
    fs::write(fixture.vault_path.join("work/archive.tar.gz"), b"other").unwrap();
    let CopyDecision::NameConflict { suggested_name } = plan_copy(
        &fixture.source(),
        &fixture.destination("work/archive.tar.gz"),
    )
    .unwrap() else {
        panic!("expected conflict");
    };
    assert_eq!(suggested_name, "archive.tar (2).gz");
}

#[test]
fn suggested_name_skips_existing_suffixes() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("work")).unwrap();
    for name in ["file.pdf", "file (2).pdf", "file (3).pdf"] {
        fs::write(fixture.vault_path.join("work").join(name), b"other").unwrap();
    }
    let CopyDecision::NameConflict { suggested_name } =
        plan_copy(&fixture.source(), &fixture.destination("work/file.pdf")).unwrap()
    else {
        panic!("expected conflict");
    };
    assert_eq!(suggested_name, "file (4).pdf");
}

#[test]
fn atomic_copy_creates_exact_copy_and_receipt() {
    let fixture = Fixture::new();
    let destination = fixture.destination("Предметы/00 Входящие/source.pdf");
    let receipt = atomic_copy(&fixture.source(), &destination).unwrap();
    assert_eq!(
        fs::read(fixture.vault_path.join(receipt.destination.as_str())).unwrap(),
        b"source contents"
    );
    assert_eq!(receipt.size_bytes, 15);
    assert_eq!(receipt.sha256, sha256_file(&fixture.source_path).unwrap());
}

#[test]
fn atomic_copy_does_not_modify_source() {
    let fixture = Fixture::new();
    let before = fs::read(&fixture.source_path).unwrap();
    atomic_copy(&fixture.source(), &fixture.destination("dest/file.pdf")).unwrap();
    assert_eq!(fs::read(&fixture.source_path).unwrap(), before);
}

#[test]
fn atomic_copy_never_overwrites_destination() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("dest")).unwrap();
    let target = fixture.vault_path.join("dest/file.pdf");
    fs::write(&target, b"keep me").unwrap();
    let error = atomic_copy(&fixture.source(), &fixture.destination("dest/file.pdf")).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(fs::read(target).unwrap(), b"keep me");
}

#[test]
fn failed_copy_leaves_no_final_or_temporary_file() {
    let fixture = Fixture::new();
    let destination = fixture.destination("dest/file.pdf");
    let wrong = Sha256Digest::parse("0".repeat(64)).unwrap();
    let error = super::atomic_copy::atomic_copy_expected(&fixture.source(), &destination, &wrong)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::IoError);
    assert!(!fixture.vault_path.join("dest/file.pdf").exists());
    let names: Vec<_> = fs::read_dir(fixture.vault_path.join("dest"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert!(names.is_empty());
}

#[test]
fn receipt_verifies_size_and_hash() {
    let fixture = Fixture::new();
    let receipt = atomic_copy(&fixture.source(), &fixture.destination("dest/file.pdf")).unwrap();
    assert_eq!(
        receipt.size_bytes,
        fs::metadata(&fixture.source_path).unwrap().len()
    );
    assert_eq!(receipt.sha256, sha256_file(&fixture.source_path).unwrap());
}

#[test]
fn backup_is_created_before_original_change() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("work")).unwrap();
    let original = fixture.vault_path.join("work/file.txt");
    fs::write(&original, b"before").unwrap();
    let manifest = create_backup(
        "operation-1",
        &fixture.vault,
        PersistentPath::new("work/file.txt").unwrap(),
        &fixture.backup,
    )
    .unwrap();
    fs::write(&original, b"after").unwrap();
    assert_eq!(
        fs::read(fixture.backup_path.join(manifest.backup_path.as_str())).unwrap(),
        b"before"
    );
}

#[test]
fn backup_manifest_contains_only_relative_paths() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("work")).unwrap();
    fs::write(fixture.vault_path.join("work/file.txt"), b"before").unwrap();
    let manifest = create_backup(
        "operation-1",
        &fixture.vault,
        PersistentPath::new("work/file.txt").unwrap(),
        &fixture.backup,
    )
    .unwrap();
    let value = serde_json::to_value(manifest).unwrap();
    let encoded = value.to_string();
    assert_eq!(value["sourcePath"], "work/file.txt");
    assert!(!encoded.contains(fixture._temp.path().to_string_lossy().as_ref()));
}

#[test]
fn backup_never_overwrites_existing_backup() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("work")).unwrap();
    fs::write(fixture.vault_path.join("work/file.txt"), b"before").unwrap();
    let path = PersistentPath::new("work/file.txt").unwrap();
    create_backup("operation-1", &fixture.vault, path.clone(), &fixture.backup).unwrap();
    assert_eq!(
        create_backup("operation-1", &fixture.vault, path, &fixture.backup)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn undo_manifest_serializes_in_camel_case() {
    let manifest = UndoManifest {
        operation_id: "operation-1".to_owned(),
        created_files: vec![CreatedFileUndoEntry {
            path: PersistentPath::new("work/new.txt").unwrap(),
            expected_sha256: Sha256Digest::parse("a".repeat(64)).unwrap(),
        }],
        backups: vec![],
    };
    let value = serde_json::to_value(manifest).unwrap();
    assert_eq!(value["operationId"], "operation-1");
    assert!(value.get("createdFiles").is_some());
    assert!(value["createdFiles"][0].get("expectedSha256").is_some());
}

#[test]
fn undo_rejects_changed_created_file() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("work")).unwrap();
    let file = fixture.vault_path.join("work/new.txt");
    fs::write(&file, b"created").unwrap();
    let original_hash = sha256_file(&file).unwrap();
    fs::write(&file, b"changed").unwrap();
    let manifest = undo_manifest_with_created(original_hash);
    assert_eq!(
        validate_undo(&manifest, undo_context(), &fixture.vault, &fixture.backup)
            .unwrap_err()
            .code,
        ErrorCode::NotUndoable
    );
}

#[test]
fn repeated_undo_is_rejected() {
    let fixture = Fixture::new();
    let mut context = undo_context();
    context.already_undone = true;
    assert_eq!(
        validate_undo(
            &empty_undo_manifest(),
            context,
            &fixture.vault,
            &fixture.backup
        )
        .unwrap_err()
        .code,
        ErrorCode::NotUndoable
    );
}

#[test]
fn submitted_homework_blocks_undo() {
    let fixture = Fixture::new();
    let mut context = undo_context();
    context.submitted_homework = true;
    assert_eq!(
        validate_undo(
            &empty_undo_manifest(),
            context,
            &fixture.vault,
            &fixture.backup
        )
        .unwrap_err()
        .code,
        ErrorCode::NotUndoable
    );
}

#[test]
fn undo_validates_backup_hash_and_relative_roots() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.vault_path.join("work")).unwrap();
    fs::write(fixture.vault_path.join("work/file.txt"), b"original").unwrap();
    let backup = create_backup(
        "operation-2",
        &fixture.vault,
        PersistentPath::new("work/file.txt").unwrap(),
        &fixture.backup,
    )
    .unwrap();
    let manifest = UndoManifest {
        operation_id: "operation-2".to_owned(),
        created_files: vec![],
        backups: vec![BackupUndoEntry {
            original_path: backup.source_path,
            backup_path: backup.backup_path,
            expected_backup_sha256: backup.sha256,
        }],
    };
    assert_eq!(
        validate_undo(&manifest, undo_context(), &fixture.vault, &fixture.backup)
            .unwrap()
            .manifest(),
        &manifest
    );
}

#[test]
fn persistent_path_deserialization_revalidates_input() {
    let result = serde_json::from_value::<PersistentPath>(json!("../escape"));
    assert!(result.is_err());
}

#[test]
fn schema_version_remains_two() {
    assert_eq!(CURRENT_SCHEMA_VERSION, 2);
}

fn undo_context() -> UndoContext {
    UndoContext {
        undoable: true,
        already_undone: false,
        submitted_homework: false,
    }
}

fn empty_undo_manifest() -> UndoManifest {
    UndoManifest {
        operation_id: "operation-1".to_owned(),
        created_files: vec![],
        backups: vec![],
    }
}

fn undo_manifest_with_created(hash: Sha256Digest) -> UndoManifest {
    UndoManifest {
        operation_id: "operation-1".to_owned(),
        created_files: vec![CreatedFileUndoEntry {
            path: PersistentPath::new("work/new.txt").unwrap(),
            expected_sha256: hash,
        }],
        backups: vec![],
    }
}
