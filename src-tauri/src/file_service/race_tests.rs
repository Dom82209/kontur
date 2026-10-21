//! Детерминированные TOCTOU/symlink-race тесты.
//!
//! Подмена выполняется синхронно из test hook в точно известной точке между
//! системными вызовами (или через `Barrier` из другого потока). Sleep не используется.
//! Все файлы находятся внутри `TempDir`; каталог `outside` моделирует цель атаки и
//! после каждого теста обязан остаться побайтно неизменным.

#![cfg(unix)]

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Barrier},
};

use tempfile::TempDir;

use crate::error::ErrorCode;

use super::{
    atomic_copy,
    atomic_copy::atomic_copy_expected,
    backup::create_backup,
    hashing::Sha256Digest,
    hooks::{install, HookPoint},
    plan_copy, sha256_file,
    undo::validate_undo,
    BackupRoot, BackupUndoEntry, CopyDecision, CreatedFileUndoEntry, PersistentPath, SourceFile,
    UndoContext, UndoManifest, VaultRoot, VerifiedDestination,
};

const SOURCE_CONTENT: &[u8] = b"source contents";

struct Race {
    temp: TempDir,
    vault_path: PathBuf,
    backup_path: PathBuf,
    outside: PathBuf,
    source_path: PathBuf,
    vault: VaultRoot,
    backup: BackupRoot,
}

impl Race {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let vault_path = temp.path().join("vault");
        let backup_path = temp.path().join("backups");
        let outside = temp.path().join("outside");
        let sources = temp.path().join("sources");
        for dir in [&vault_path, &backup_path, &outside, &sources] {
            fs::create_dir_all(dir).unwrap();
        }
        let source_path = sources.join("Лекция.pdf");
        fs::write(&source_path, SOURCE_CONTENT).unwrap();
        let vault = VaultRoot::new(&vault_path, None).unwrap();
        let backup = BackupRoot::new(&backup_path, None).unwrap();
        Self {
            temp,
            vault_path,
            backup_path,
            outside,
            source_path,
            vault,
            backup,
        }
    }

    fn source(&self) -> SourceFile {
        SourceFile::new(&self.source_path).unwrap()
    }

    fn destination(&self, path: &str) -> VerifiedDestination {
        VerifiedDestination::in_vault(&self.vault, PersistentPath::new(path).unwrap()).unwrap()
    }

    fn outside_snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        snapshot(&self.outside)
    }

    fn sources_snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        snapshot(self.source_path.parent().unwrap())
    }
}

/// Перемещает каталог в `<name>-moved` и кладёт на его место symlink на `target`.
fn swap_with_symlink(dir: &Path, target: &Path) -> PathBuf {
    let moved = dir.with_file_name(format!(
        "{}-moved",
        dir.file_name().unwrap().to_string_lossy()
    ));
    fs::rename(dir, &moved).unwrap();
    symlink(target, dir).unwrap();
    moved
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut output = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let file_type = entry.file_type().unwrap();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if file_type.is_dir() {
                output.insert(relative, b"<dir>".to_vec());
                stack.push(path);
            } else if file_type.is_symlink() {
                output.insert(relative, b"<symlink>".to_vec());
            } else {
                output.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    output
}

fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Хук, срабатывающий один раз в заданной точке.
fn once_at(point: HookPoint, action: impl FnOnce() + 'static) -> impl FnMut(&HookPoint) {
    let mut action = Some(action);
    move |current: &HookPoint| {
        if *current == point {
            if let Some(action) = action.take() {
                action();
            }
        }
    }
}

fn undo_context() -> UndoContext {
    UndoContext {
        undoable: true,
        already_undone: false,
        submitted_homework: false,
    }
}

#[test]
fn successful_copy_reaches_all_write_hook_points_in_order() {
    let race = Race::new();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let recorder = Rc::clone(&seen);
    let _hook = install(move |point| recorder.borrow_mut().push(point.clone()));
    atomic_copy(&race.source(), &race.destination("dest/Лекция.pdf")).unwrap();
    assert_eq!(
        *seen.borrow(),
        vec![
            HookPoint::BeforeTempCreate,
            HookPoint::BeforeRename,
            HookPoint::BeforeFinalVerify
        ]
    );
}

#[test]
fn parent_swapped_before_temp_creation_is_blocked_without_outside_write() {
    let race = Race::new();
    let outside_before = race.outside_snapshot();
    let dest = race.vault_path.join("dest");
    let outside = race.outside.clone();
    let moved = dest.with_file_name("dest-moved");
    let _hook = install(once_at(HookPoint::BeforeTempCreate, move || {
        swap_with_symlink(&dest, &outside);
    }));

    let error = atomic_copy(&race.source(), &race.destination("dest/Лекция.pdf")).unwrap_err();

    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
    assert!(entry_names(&moved).is_empty());
}

#[test]
fn parent_swapped_before_rename_is_blocked_and_temp_is_removed_via_fd() {
    let race = Race::new();
    let outside_before = race.outside_snapshot();
    let dest = race.vault_path.join("dest");
    let outside = race.outside.clone();
    let moved = dest.with_file_name("dest-moved");
    let _hook = install(once_at(HookPoint::BeforeRename, move || {
        swap_with_symlink(&dest, &outside);
    }));

    let error = atomic_copy(&race.source(), &race.destination("dest/Лекция.pdf")).unwrap_err();

    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
    assert!(
        entry_names(&moved).is_empty(),
        "temp остался в исходном каталоге"
    );
}

#[test]
fn parent_swapped_before_final_verification_rolls_back_inside_original_directory() {
    let race = Race::new();
    fs::write(race.outside.join("Лекция.pdf"), b"victim").unwrap();
    let outside_before = race.outside_snapshot();
    let dest = race.vault_path.join("dest");
    let outside = race.outside.clone();
    let moved = dest.with_file_name("dest-moved");
    let _hook = install(once_at(HookPoint::BeforeFinalVerify, move || {
        swap_with_symlink(&dest, &outside);
    }));

    let error = atomic_copy(&race.source(), &race.destination("dest/Лекция.pdf")).unwrap_err();

    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
    assert!(
        entry_names(&moved).is_empty(),
        "неподтверждённый final должен быть удалён из исходного inode каталога"
    );
}

#[test]
fn cleanup_after_parent_swap_never_touches_outside_victim() {
    let race = Race::new();
    let dest = race.vault_path.join("dest");
    let outside = race.outside.clone();
    let moved = dest.with_file_name("dest-moved");
    let victim_name = Rc::new(RefCell::new(None));
    let recorded = Rc::clone(&victim_name);
    let _hook = install(move |point| {
        if let HookPoint::BeforeCleanup { file_name } = point {
            if recorded.borrow().is_none() {
                swap_with_symlink(&dest, &outside);
                // Файл с тем же именем, что и temp, теперь «виден» по старому пути.
                fs::write(outside.join(file_name), b"victim").unwrap();
                *recorded.borrow_mut() = Some(file_name.clone());
            }
        }
    });
    let wrong = Sha256Digest::parse("0".repeat(64)).unwrap();

    let error = atomic_copy_expected(&race.source(), &race.destination("dest/Лекция.pdf"), &wrong)
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::IoError);
    let name = victim_name.borrow().clone().expect("cleanup hook reached");
    assert_eq!(fs::read(race.outside.join(&name)).unwrap(), b"victim");
    assert!(
        entry_names(&moved).is_empty(),
        "temp удалён через fd каталога"
    );
}

#[test]
fn cleanup_only_removes_inode_created_by_this_operation() {
    let race = Race::new();
    let dest = race.vault_path.join("dest");
    let replaced = Rc::new(RefCell::new(None));
    let recorded = Rc::clone(&replaced);
    let dest_for_hook = dest.clone();
    let _hook = install(move |point| {
        if let HookPoint::BeforeCleanup { file_name } = point {
            if recorded.borrow().is_none() {
                // Конкурент атомарно заменяет temp другим файлом под тем же именем.
                let foreign = dest_for_hook.join("foreign");
                fs::write(&foreign, b"foreign").unwrap();
                fs::rename(&foreign, dest_for_hook.join(file_name)).unwrap();
                *recorded.borrow_mut() = Some(file_name.clone());
            }
        }
    });
    let wrong = Sha256Digest::parse("0".repeat(64)).unwrap();

    atomic_copy_expected(&race.source(), &race.destination("dest/Лекция.pdf"), &wrong).unwrap_err();

    let name = replaced.borrow().clone().expect("cleanup hook reached");
    assert_eq!(fs::read(dest.join(name)).unwrap(), b"foreign");
}

#[test]
fn planning_detects_parent_swap_and_never_trusts_outside_content() {
    let race = Race::new();
    fs::create_dir(race.vault_path.join("dest")).unwrap();
    // Снаружи лежит файл с тем же содержимым: path-based planning ответил бы SameContent.
    fs::write(race.outside.join("Лекция.pdf"), SOURCE_CONTENT).unwrap();
    let outside_before = race.outside_snapshot();
    let destination = race.destination("dest/Лекция.pdf");
    let dest = race.vault_path.join("dest");
    let outside = race.outside.clone();
    let _hook = install(once_at(HookPoint::PlanningParentOpened, move || {
        swap_with_symlink(&dest, &outside);
    }));

    let error = plan_copy(&race.source(), &destination).unwrap_err();

    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
}

#[test]
fn planning_rejects_symlink_leaf_with_matching_content() {
    let race = Race::new();
    fs::create_dir(race.vault_path.join("dest")).unwrap();
    fs::write(race.outside.join("same.pdf"), SOURCE_CONTENT).unwrap();
    symlink(
        race.outside.join("same.pdf"),
        race.vault_path.join("dest/Лекция.pdf"),
    )
    .unwrap();
    assert_eq!(
        plan_copy(&race.source(), &race.destination("dest/Лекция.pdf"))
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn planning_keeps_conflict_numbering_and_cyrillic_extension() {
    let race = Race::new();
    let dir = race.vault_path.join("Предметы");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("Лекция.pdf"), b"other").unwrap();
    fs::write(dir.join("Лекция (2).pdf"), b"other").unwrap();
    assert_eq!(
        plan_copy(&race.source(), &race.destination("Предметы/Лекция.pdf")).unwrap(),
        CopyDecision::NameConflict {
            suggested_name: "Лекция (3).pdf".to_owned()
        }
    );
    assert_eq!(
        entry_names(&dir),
        vec!["Лекция (2).pdf".to_owned(), "Лекция.pdf".to_owned()],
        "planning остаётся read-only"
    );
}

#[test]
fn same_content_plan_does_not_create_copy() {
    let race = Race::new();
    let dir = race.vault_path.join("dest");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("Лекция.pdf"), SOURCE_CONTENT).unwrap();
    assert_eq!(
        plan_copy(&race.source(), &race.destination("dest/Лекция.pdf")).unwrap(),
        CopyDecision::SameContent
    );
    assert_eq!(entry_names(&dir), vec!["Лекция.pdf".to_owned()]);
}

#[test]
fn intermediate_symlink_created_after_verification_is_rejected_by_copy() {
    let race = Race::new();
    fs::create_dir_all(race.vault_path.join("a/b")).unwrap();
    let destination = race.destination("a/b/c/Лекция.pdf");
    swap_with_symlink(&race.vault_path.join("a/b"), &race.outside);
    let outside_before = race.outside_snapshot();

    let error = atomic_copy(&race.source(), &destination).unwrap_err();

    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
}

#[test]
fn intermediate_regular_file_is_rejected() {
    let race = Race::new();
    fs::write(race.vault_path.join("a"), b"not a dir").unwrap();
    assert_eq!(
        VerifiedDestination::in_vault(&race.vault, PersistentPath::new("a/b.pdf").unwrap())
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn undo_detects_parent_swap_for_created_file() {
    let race = Race::new();
    let work = race.vault_path.join("work");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("new.txt"), b"changed").unwrap();
    // Снаружи лежит файл с ожидаемым содержимым.
    fs::write(race.outside.join("new.txt"), b"created").unwrap();
    let expected = sha256_file(&race.outside.join("new.txt")).unwrap();
    let outside_before = race.outside_snapshot();
    let manifest = UndoManifest {
        operation_id: "operation-1".to_owned(),
        created_files: vec![CreatedFileUndoEntry {
            path: PersistentPath::new("work/new.txt").unwrap(),
            expected_sha256: expected,
        }],
        backups: vec![],
    };
    let outside = race.outside.clone();
    let _hook = install(once_at(HookPoint::UndoParentOpened, move || {
        swap_with_symlink(&work, &outside);
    }));

    let error = validate_undo(&manifest, undo_context(), &race.vault, &race.backup).unwrap_err();

    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
}

#[test]
fn undo_detects_backup_directory_swap() {
    let race = Race::new();
    fs::create_dir(race.vault_path.join("work")).unwrap();
    fs::write(race.vault_path.join("work/file.txt"), b"original").unwrap();
    let backup = create_backup(
        "operation-2",
        &race.vault,
        PersistentPath::new("work/file.txt").unwrap(),
        &race.backup,
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
    let backup_work = race.backup_path.join("operation-2/work");
    let fake = race.outside.join("work");
    fs::create_dir(&fake).unwrap();
    fs::write(fake.join("file.txt"), b"original").unwrap();
    let outside_before = race.outside_snapshot();
    let calls = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&calls);
    let _hook = install(move |point| {
        if *point == HookPoint::UndoParentOpened {
            *counter.borrow_mut() += 1;
            // Первый вызов — родитель original, второй — каталог резервной копии.
            if *counter.borrow() == 2 {
                swap_with_symlink(&backup_work, &fake);
            }
        }
    });

    let error = validate_undo(&manifest, undo_context(), &race.vault, &race.backup).unwrap_err();

    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
}

#[test]
fn undo_rejects_symlink_in_intermediate_component() {
    let race = Race::new();
    fs::create_dir(race.outside.join("work")).unwrap();
    fs::write(race.outside.join("work/new.txt"), b"created").unwrap();
    let expected = sha256_file(&race.outside.join("work/new.txt")).unwrap();
    let manifest_path = PersistentPath::new("link/work/new.txt").unwrap();
    let manifest = UndoManifest {
        operation_id: "operation-1".to_owned(),
        created_files: vec![CreatedFileUndoEntry {
            path: manifest_path,
            expected_sha256: expected,
        }],
        backups: vec![],
    };
    symlink(&race.outside, race.vault_path.join("link")).unwrap();
    assert_eq!(
        validate_undo(&manifest, undo_context(), &race.vault, &race.backup)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn undo_rejects_symlink_leaf_as_not_undoable() {
    let race = Race::new();
    fs::create_dir(race.vault_path.join("work")).unwrap();
    fs::write(race.outside.join("new.txt"), b"created").unwrap();
    symlink(
        race.outside.join("new.txt"),
        race.vault_path.join("work/new.txt"),
    )
    .unwrap();
    let manifest = UndoManifest {
        operation_id: "operation-1".to_owned(),
        created_files: vec![CreatedFileUndoEntry {
            path: PersistentPath::new("work/new.txt").unwrap(),
            expected_sha256: sha256_file(&race.outside.join("new.txt")).unwrap(),
        }],
        backups: vec![],
    };
    assert_eq!(
        validate_undo(&manifest, undo_context(), &race.vault, &race.backup)
            .unwrap_err()
            .code,
        ErrorCode::NotUndoable
    );
}

#[test]
fn source_is_bound_to_the_inode_opened_once() {
    let race = Race::new();
    let source = race.source();
    // После открытия путь source указывает на другой inode.
    let replacement = race.source_path.with_file_name("replacement.pdf");
    fs::write(&replacement, b"attacker controlled").unwrap();
    fs::rename(&replacement, &race.source_path).unwrap();

    let receipt = atomic_copy(&source, &race.destination("dest/Лекция.pdf")).unwrap();

    assert_eq!(
        fs::read(race.vault_path.join("dest/Лекция.pdf")).unwrap(),
        SOURCE_CONTENT
    );
    assert_eq!(receipt.size_bytes, SOURCE_CONTENT.len() as u64);
    assert_eq!(
        receipt.sha256,
        sha256_file(&race.vault_path.join("dest/Лекция.pdf")).unwrap()
    );
}

#[test]
fn source_rejects_fifo_without_blocking() {
    let race = Race::new();
    let fifo = race.temp.path().join("pipe");
    let status = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(SourceFile::new(&fifo).unwrap_err().code, ErrorCode::Blocked);
}

#[test]
fn vault_root_swapped_after_open_keeps_operations_in_original_inode() {
    let race = Race::new();
    let source = race.source();
    let destination = race.destination("dest/Лекция.pdf");
    let moved = swap_with_symlink(&race.vault_path, &race.outside);
    let outside_before = race.outside_snapshot();

    atomic_copy(&source, &destination).unwrap();

    assert_eq!(race.outside_snapshot(), outside_before);
    assert_eq!(
        fs::read(moved.join("dest/Лекция.pdf")).unwrap(),
        SOURCE_CONTENT
    );
}

#[test]
fn backup_rejects_symlinked_source_component() {
    let race = Race::new();
    fs::create_dir(race.outside.join("work")).unwrap();
    fs::write(race.outside.join("work/file.txt"), b"secret").unwrap();
    symlink(race.outside.join("work"), race.vault_path.join("work")).unwrap();
    let error = create_backup(
        "operation-3",
        &race.vault,
        PersistentPath::new("work/file.txt").unwrap(),
        &race.backup,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(entry_names(&race.backup_path).is_empty());
}

#[test]
fn overwrite_is_impossible_even_when_destination_appears_before_rename() {
    let race = Race::new();
    let dest = race.vault_path.join("dest");
    let target = dest.join("Лекция.pdf");
    let target_for_hook = target.clone();
    let _hook = install(once_at(HookPoint::BeforeRename, move || {
        fs::write(&target_for_hook, b"concurrent").unwrap();
    }));

    let error = atomic_copy(&race.source(), &race.destination("dest/Лекция.pdf")).unwrap_err();

    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(fs::read(&target).unwrap(), b"concurrent");
    assert_eq!(entry_names(&dest), vec!["Лекция.pdf".to_owned()]);
}

#[test]
fn barrier_synchronized_attacker_thread_cannot_redirect_publish() {
    let race = Race::new();
    let source = race.source();
    let destination = race.destination("dest/Лекция.pdf");
    let dest = race.vault_path.join("dest");
    let outside = race.outside.clone();
    let outside_before = race.outside_snapshot();
    let sources_before = race.sources_snapshot();
    let reached = Arc::new(Barrier::new(2));
    let swapped = Arc::new(Barrier::new(2));

    let worker = {
        let reached = Arc::clone(&reached);
        let swapped = Arc::clone(&swapped);
        std::thread::spawn(move || {
            // Хук thread-local: он установлен в рабочем потоке операции.
            let _hook = install(once_at(HookPoint::BeforeRename, move || {
                reached.wait();
                swapped.wait();
            }));
            atomic_copy(&source, &destination).map(|_| ())
        })
    };

    reached.wait();
    let moved = swap_with_symlink(&dest, &outside);
    swapped.wait();
    let result = worker.join().unwrap();

    assert_eq!(result.unwrap_err().code, ErrorCode::Blocked);
    assert_eq!(race.outside_snapshot(), outside_before);
    assert_eq!(race.sources_snapshot(), sources_before);
    assert!(entry_names(&moved).is_empty());
}
