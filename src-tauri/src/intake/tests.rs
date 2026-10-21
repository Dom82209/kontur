//! Сценарии импорта во «Входящие». Все файловые операции выполняются только в `TempDir`.

use std::{
    cell::RefCell,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use tempfile::TempDir;
use uuid::Uuid;

use super::{
    choose_root, pick_files_for_import, register_native_paths, FilePicker, ImportItemInput,
    ImportItemReport, ImportOrigin, ImportOutcome, ImportProgress, InboxImporter, IntakeGrants,
    RootKind, RootState, RootsService, INBOX_RELATIVE,
};
use crate::{
    db::Database,
    domain::OperationStatus,
    dto::{FileKind, OperationKind},
    error::ErrorCode,
    file_service::sha256_file,
};

struct Fixture {
    _tmp: TempDir,
    root: PathBuf,
    db: Database,
    vault: PathBuf,
    external: PathBuf,
    home: PathBuf,
    grants: IntakeGrants,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let vault = root.join("vault");
        let external = root.join("external");
        let home = root.join("home");
        for dir in [&vault, &external, &home] {
            fs::create_dir_all(dir).unwrap();
        }
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        let fixture = Self {
            _tmp: tmp,
            root,
            db,
            vault,
            external,
            home,
            grants: IntakeGrants::new(),
        };
        fixture
            .roots_service()
            .set_root(RootKind::Vault, &fixture.vault)
            .unwrap();
        fixture
    }

    fn roots_service(&self) -> RootsService<'_> {
        RootsService::new(&self.db, Some(self.home.clone()))
    }

    fn inbox(&self) -> PathBuf {
        self.vault.join(INBOX_RELATIVE)
    }

    fn external_file(&self, relative: &str, content: &[u8]) -> PathBuf {
        let path = self.external.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn import_with_id(
        &self,
        operation_id: &str,
        paths: &[&Path],
    ) -> (ImportOutcome, Vec<ImportProgress>) {
        self.grants
            .grant(paths.iter().map(|path| path.to_path_buf()));
        self.import_ungranted(operation_id, paths).unwrap()
    }

    fn import(&self, paths: &[&Path]) -> (ImportOutcome, Vec<ImportProgress>) {
        self.import_with_id(&Uuid::new_v4().to_string(), paths)
    }

    fn import_ungranted(
        &self,
        operation_id: &str,
        paths: &[&Path],
    ) -> Result<(ImportOutcome, Vec<ImportProgress>), crate::error::AppError> {
        let roots = self.roots_service().open_import_roots()?;
        let items: Vec<ImportItemInput> = paths
            .iter()
            .map(|path| ImportItemInput {
                source_path: path.to_string_lossy().into_owned(),
                origin: ImportOrigin::Drop,
            })
            .collect();
        let mut events = Vec::new();
        let outcome = InboxImporter::new(&self.db, &roots, &self.grants).run(
            operation_id,
            &items,
            &mut |event| events.push(event),
        )?;
        Ok((outcome, events))
    }

    fn inbox_entries(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.inbox()) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn inbox_rows(&self) -> i64 {
        self.db
            .with_connection(|connection| {
                Ok(connection
                    .query_row("SELECT COUNT(*) FROM inbox_files", [], |row| row.get(0))?)
            })
            .unwrap()
    }

    fn all_text_in_database(&self) -> String {
        self.db
            .with_connection(|connection| {
                let mut out = String::new();
                for table in [
                    "app_settings",
                    "inbox_files",
                    "operations",
                    "operation_results",
                ] {
                    let mut statement = connection.prepare(&format!("SELECT * FROM {table}"))?;
                    let columns = statement.column_count();
                    let mut rows = statement.query([])?;
                    while let Some(row) = rows.next()? {
                        for index in 0..columns {
                            if let Ok(Some(value)) = row.get::<_, Option<String>>(index) {
                                out.push_str(&value);
                                out.push('\n');
                            }
                        }
                    }
                }
                Ok(out)
            })
            .unwrap()
    }
}

fn single(outcome: &ImportOutcome) -> &ImportItemReport {
    assert_eq!(outcome.report.items.len(), 1);
    &outcome.report.items[0]
}

fn failed_code(outcome: &ImportOutcome) -> ErrorCode {
    match single(outcome) {
        ImportItemReport::Failed { error, .. } => error.code,
        other => panic!("expected failure, got {other:?}"),
    }
}

struct FakePicker {
    files: Option<Vec<PathBuf>>,
    folder: Option<PathBuf>,
    calls: RefCell<u32>,
}

impl FilePicker for FakePicker {
    fn pick_files(&self, _start: Option<&Path>) -> Option<Vec<PathBuf>> {
        *self.calls.borrow_mut() += 1;
        self.files.clone()
    }

    fn pick_folder(&self, _title: &str, _start: Option<&Path>) -> Option<PathBuf> {
        *self.calls.borrow_mut() += 1;
        self.folder.clone()
    }
}

// ───────────── Валидный импорт ─────────────

#[test]
fn valid_import_copies_atomically_records_hash_and_reports_progress() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Лекция 07.pdf", b"%PDF-1.7 lecture");
    let (outcome, progress) = fixture.import(&[&source]);

    let ImportItemReport::Imported {
        file,
        renamed_from,
        recovered,
        name,
    } = single(&outcome)
    else {
        panic!("expected import");
    };
    assert_eq!(name, "Лекция 07.pdf");
    assert_eq!(file.name, "Лекция 07.pdf");
    assert_eq!(file.kind, FileKind::Pdf);
    assert_eq!(file.size_bytes, 16);
    assert!(renamed_from.is_none());
    assert!(!recovered);
    assert!(file.duplicate.is_none());

    let copied = fixture.inbox().join("Лекция 07.pdf");
    assert_eq!(fs::read(&copied).unwrap(), b"%PDF-1.7 lecture");
    assert_eq!(
        fs::read(&source).unwrap(),
        b"%PDF-1.7 lecture",
        "source untouched"
    );
    assert_eq!(
        fixture.inbox_entries(),
        vec!["Лекция 07.pdf"],
        "no temp files left"
    );

    let (sha, mime, origin): (String, String, String) = fixture
        .db
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT sha256, mime_type, origin FROM inbox_files",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
        .unwrap();
    assert_eq!(sha, sha256_file(&source).unwrap().as_str());
    assert_eq!(mime, "application/pdf");
    assert_eq!(origin, "drop");

    assert_eq!(outcome.operation.kind, OperationKind::Import);
    assert_eq!(outcome.operation.status, OperationStatus::Completed);
    assert!(!outcome.operation.undoable);
    assert_eq!(
        outcome.operation.paths,
        vec![format!("{INBOX_RELATIVE}/Лекция 07.pdf")]
    );
    assert!(outcome.failure.is_none());
    assert_eq!(outcome.added_files().len(), 1);

    let fractions: Vec<f64> = progress.iter().map(|event| event.progress).collect();
    assert_eq!(fractions, vec![0.0, 1.0]);
    assert!(progress
        .iter()
        .all(|event| event.operation_id == outcome.report.operation_id));

    let stored = super::stored_import_report(&fixture.db, &outcome.report.operation_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.imported, 1);
    assert!(!stored.replayed);
}

#[test]
fn no_source_or_absolute_paths_are_persisted() {
    let fixture = Fixture::new();
    let source = fixture.external_file("private/Конспект.md", b"# notes");
    let missing = fixture.external.join("gone.pdf");
    fixture.import(&[&source, &missing]);
    let text = fixture.all_text_in_database();
    assert!(!text.contains(fixture.external.to_str().unwrap()));
    // Единственный абсолютный путь — выбранный корень хранилища.
    let absolute: Vec<&str> = text
        .lines()
        .filter(|line| line.contains(fixture.root.to_str().unwrap()))
        .collect();
    assert_eq!(
        absolute.len(),
        1,
        "only managed_roots holds a root path: {absolute:?}"
    );
    assert!(absolute[0].contains("vaultPath"));
}

// ───────────── Дедупликация и конфликты ─────────────

#[test]
fn duplicate_content_is_detected_and_not_copied_twice() {
    let fixture = Fixture::new();
    let first = fixture.external_file("a/Семинар.pdf", b"same bytes");
    let second = fixture.external_file("b/Семинар копия.pdf", b"same bytes");
    let (first_outcome, _) = fixture.import(&[&first]);
    let (outcome, _) = fixture.import(&[&second]);

    let ImportItemReport::Imported { file: original, .. } = single(&first_outcome) else {
        panic!();
    };
    let ImportItemReport::Duplicate { file, restored, .. } = single(&outcome) else {
        panic!("expected duplicate");
    };
    assert_eq!(file.id, original.id);
    assert!(!restored);
    assert_eq!(fixture.inbox_entries(), vec!["Семинар.pdf"]);
    assert_eq!(fixture.inbox_rows(), 1);
    assert_eq!(outcome.report.duplicates, 1);
    assert!(outcome.added_files().is_empty());
    assert_eq!(outcome.touched_file_ids(), vec![original.id.clone()]);
    assert_eq!(outcome.operation.status, OperationStatus::Completed);
}

#[test]
fn same_name_with_different_content_gets_numbered_name_without_overwrite() {
    let fixture = Fixture::new();
    let first = fixture.external_file("a/Лекция.pdf", b"version A");
    let second = fixture.external_file("b/Лекция.pdf", b"version B");
    let third = fixture.external_file("c/Лекция.pdf", b"version C");
    fixture.import(&[&first]);
    let (outcome, _) = fixture.import(&[&second, &third]);

    let names: Vec<(String, Option<String>)> = outcome
        .report
        .items
        .iter()
        .map(|item| match item {
            ImportItemReport::Imported {
                file, renamed_from, ..
            } => (file.name.clone(), renamed_from.clone()),
            other => panic!("expected import, got {other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        vec![
            ("Лекция (2).pdf".to_owned(), Some("Лекция.pdf".to_owned())),
            ("Лекция (3).pdf".to_owned(), Some("Лекция.pdf".to_owned())),
        ]
    );
    assert_eq!(
        fs::read(fixture.inbox().join("Лекция.pdf")).unwrap(),
        b"version A"
    );
    assert_eq!(
        fs::read(fixture.inbox().join("Лекция (2).pdf")).unwrap(),
        b"version B"
    );
    assert_eq!(
        fs::read(fixture.inbox().join("Лекция (3).pdf")).unwrap(),
        b"version C"
    );
    let ImportItemReport::Imported { file, .. } = &outcome.report.items[0] else {
        panic!();
    };
    let duplicate = file.duplicate.as_ref().unwrap();
    assert!(!duplicate.same_content);
    assert_eq!(
        duplicate.existing_path,
        format!("{INBOX_RELATIVE}/Лекция.pdf")
    );
    assert_eq!(duplicate.suggested_name, "Лекция (2).pdf");
}

#[test]
fn name_taken_by_foreign_file_on_disk_is_never_overwritten() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.inbox()).unwrap();
    fs::write(fixture.inbox().join("План.docx"), b"user file").unwrap();
    let source = fixture.external_file("План.docx", b"imported");
    let (outcome, _) = fixture.import(&[&source]);
    let ImportItemReport::Imported { file, .. } = single(&outcome) else {
        panic!();
    };
    assert_eq!(file.name, "План (2).docx");
    assert_eq!(
        fs::read(fixture.inbox().join("План.docx")).unwrap(),
        b"user file"
    );
}

// ───────────── Недоступный или небезопасный source ─────────────

#[test]
fn vanished_source_fails_without_side_effects() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Задание.pdf", b"task");
    fixture.grants.grant([source.clone()]);
    fs::remove_file(&source).unwrap();
    let (outcome, _) = fixture
        .import_ungranted(&Uuid::new_v4().to_string(), &[&source])
        .unwrap();
    assert_eq!(failed_code(&outcome), ErrorCode::NotFound);
    assert_eq!(outcome.operation.status, OperationStatus::Failed);
    assert_eq!(outcome.failure.as_ref().unwrap().code, ErrorCode::NotFound);
    assert!(fixture.inbox_entries().is_empty());
    assert_eq!(fixture.inbox_rows(), 0);
}

#[test]
fn symlink_source_is_blocked() {
    let fixture = Fixture::new();
    let target = fixture.external_file("real.pdf", b"secret");
    let link = fixture.external.join("link.pdf");
    symlink(&target, &link).unwrap();
    let (outcome, _) = fixture.import(&[&link]);
    assert_eq!(failed_code(&outcome), ErrorCode::Blocked);
    assert!(fixture.inbox_entries().is_empty());
}

#[test]
fn symlink_parent_of_source_is_blocked() {
    let fixture = Fixture::new();
    let real_dir = fixture.root.join("elsewhere");
    fs::create_dir_all(&real_dir).unwrap();
    fs::write(real_dir.join("id_rsa"), b"private key").unwrap();
    let linked_dir = fixture.external.join("Загрузки");
    symlink(&real_dir, &linked_dir).unwrap();
    let source = linked_dir.join("id_rsa");
    let (outcome, _) = fixture.import(&[&source]);
    assert_eq!(failed_code(&outcome), ErrorCode::Blocked);
    assert_eq!(fixture.inbox_rows(), 0);
    assert!(fixture.inbox_entries().is_empty());
}

#[test]
fn directory_and_relative_sources_are_rejected() {
    let fixture = Fixture::new();
    let directory = fixture.external.join("Папка");
    fs::create_dir_all(&directory).unwrap();
    let (outcome, _) = fixture.import(&[&directory]);
    assert_eq!(failed_code(&outcome), ErrorCode::Blocked);
    let (outcome, _) = fixture.import(&[Path::new("relative.pdf")]);
    assert_eq!(failed_code(&outcome), ErrorCode::Blocked);
}

#[test]
fn path_not_granted_by_picker_or_drop_is_rejected() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Отчёт.xlsx", b"sheet");
    let (outcome, _) = fixture
        .import_ungranted(&Uuid::new_v4().to_string(), &[&source])
        .unwrap();
    assert_eq!(failed_code(&outcome), ErrorCode::Blocked);
    assert!(fixture.inbox_entries().is_empty());
}

#[test]
fn symlinked_inbox_component_is_blocked_and_outside_is_untouched() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside");
    fs::create_dir_all(outside.join("00 Входящие")).unwrap();
    symlink(&outside, fixture.vault.join("Предметы")).unwrap();
    let source = fixture.external_file("a.pdf", b"a");
    fixture.grants.grant([source.clone()]);
    let error = fixture
        .import_ungranted(&Uuid::new_v4().to_string(), &[&source])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(fs::read_dir(outside.join("00 Входящие"))
        .unwrap()
        .next()
        .is_none());
}

// ───────────── Picker ─────────────

#[test]
fn cancelled_picker_returns_no_items_and_grants_nothing() {
    let fixture = Fixture::new();
    let picker = FakePicker {
        files: None,
        folder: None,
        calls: RefCell::new(0),
    };
    let items = pick_files_for_import(&picker, &fixture.grants, None);
    assert!(items.is_empty());
    assert_eq!(fixture.grants.granted_count(), 0);
    assert_eq!(*picker.calls.borrow(), 1);

    let before = fixture.roots_service().config().unwrap();
    let selection = choose_root(&fixture.roots_service(), &picker, RootKind::Backup).unwrap();
    assert!(selection.cancelled);
    assert_eq!(fixture.roots_service().config().unwrap(), before);
}

#[test]
fn picker_selection_registers_only_regular_files() {
    let fixture = Fixture::new();
    let file = fixture.external_file("Данные.csv", b"a,b\n1,2\n");
    let directory = fixture.external.join("Папка");
    fs::create_dir_all(&directory).unwrap();
    let picker = FakePicker {
        files: Some(vec![file.clone(), directory.clone()]),
        folder: None,
        calls: RefCell::new(0),
    };
    let items = pick_files_for_import(&picker, &fixture.grants, None);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Данные.csv");
    assert_eq!(items[0].size_bytes, 8);
    assert_eq!(items[0].source_path, file.to_string_lossy());
    assert!(fixture.grants.is_granted(&file));
    assert!(!fixture.grants.is_granted(&directory));
}

#[test]
fn native_drop_paths_are_registered_like_picker_paths() {
    let fixture = Fixture::new();
    let file = fixture.external_file("drop.txt", b"x");
    let items = register_native_paths(
        &fixture.grants,
        vec![file.clone(), PathBuf::from("rel.txt")],
    );
    assert_eq!(items.len(), 1);
    assert!(fixture.grants.is_granted(&file));
}

// ───────────── Ошибки до и после filesystem commit ─────────────

#[test]
fn database_failure_after_copy_removes_created_copy() {
    let fixture = Fixture::new();
    fixture
        .db
        .with_connection(|connection| {
            connection.execute_batch(
                "CREATE TRIGGER test_fail_inbox_insert BEFORE INSERT ON inbox_files
                 BEGIN SELECT RAISE(ABORT, 'simulated database failure'); END;",
            )?;
            Ok(())
        })
        .unwrap();
    let source = fixture.external_file("Лекция.pdf", b"content");
    let (outcome, _) = fixture.import(&[&source]);
    let ImportItemReport::Failed { error, .. } = single(&outcome) else {
        panic!();
    };
    assert_eq!(error.code, ErrorCode::Database);
    assert!(error.message.contains("копия удалена"), "{}", error.message);
    assert!(
        fixture.inbox_entries().is_empty(),
        "compensation removed the copy"
    );
    assert_eq!(fixture.inbox_rows(), 0);
    assert_eq!(outcome.operation.status, OperationStatus::Failed);
}

#[test]
fn copy_failure_before_database_write_leaves_no_record_or_file() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Большой.pdf", b"0123456789abcdef");
    let shrink = source.clone();
    let _hook = crate::file_service::install_test_hook(move |point| {
        if point == &crate::file_service::TestHookPoint::BeforeTempCreate {
            fs::write(&shrink, b"0123").unwrap();
        }
    });
    let (outcome, _) = fixture.import(&[&source]);
    assert_eq!(failed_code(&outcome), ErrorCode::IoError);
    assert!(fixture.inbox_entries().is_empty());
    assert_eq!(fixture.inbox_rows(), 0);
}

#[test]
fn orphan_file_from_interrupted_import_is_adopted_without_copy() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.inbox()).unwrap();
    fs::write(fixture.inbox().join("Лекция.pdf"), b"same").unwrap();
    let source = fixture.external_file("Лекция.pdf", b"same");
    let (outcome, _) = fixture.import(&[&source]);
    let ImportItemReport::Imported {
        recovered, file, ..
    } = single(&outcome)
    else {
        panic!();
    };
    assert!(recovered);
    assert_eq!(file.name, "Лекция.pdf");
    assert_eq!(fixture.inbox_entries(), vec!["Лекция.pdf"]);
    assert_eq!(fixture.inbox_rows(), 1);
}

#[test]
fn record_whose_file_disappeared_is_restored_on_reimport() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Семинар.pptx", b"slides");
    fixture.import(&[&source]);
    fs::remove_file(fixture.inbox().join("Семинар.pptx")).unwrap();
    let (outcome, _) = fixture.import(&[&source]);
    let ImportItemReport::Duplicate { restored, .. } = single(&outcome) else {
        panic!();
    };
    assert!(restored);
    assert_eq!(
        fs::read(fixture.inbox().join("Семинар.pptx")).unwrap(),
        b"slides"
    );
    assert_eq!(fixture.inbox_rows(), 1);
}

#[test]
fn record_with_changed_file_on_disk_is_reported_and_not_touched() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Конспект.md", b"original");
    fixture.import(&[&source]);
    fs::write(fixture.inbox().join("Конспект.md"), b"edited by user").unwrap();
    let (outcome, _) = fixture.import(&[&source]);
    assert_eq!(failed_code(&outcome), ErrorCode::Conflict);
    assert_eq!(
        fs::read(fixture.inbox().join("Конспект.md")).unwrap(),
        b"edited by user"
    );
}

// ───────────── Идемпотентность ─────────────

#[test]
fn repeated_command_with_same_operation_id_replays_stored_result() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Задание.pdf", b"task");
    let operation_id = Uuid::new_v4().to_string();
    let (first, _) = fixture.import_with_id(&operation_id, &[&source]);
    let (second, progress) = fixture.import_with_id(&operation_id, &[&source]);
    assert!(second.report.replayed);
    assert!(progress.is_empty(), "replay performs no file work");
    assert_eq!(second.report.items, first.report.items);
    assert_eq!(second.operation.id, first.operation.id);
    assert_eq!(fixture.inbox_rows(), 1);
    assert_eq!(fixture.inbox_entries(), vec!["Задание.pdf"]);
    let operations: i64 = fixture
        .db
        .with_connection(|connection| {
            Ok(connection.query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))?)
        })
        .unwrap();
    assert_eq!(operations, 1);
}

#[test]
fn repeated_import_of_same_file_with_new_id_is_idempotent() {
    let fixture = Fixture::new();
    let source = fixture.external_file("Задание.pdf", b"task");
    fixture.import(&[&source]);
    let (outcome, _) = fixture.import(&[&source]);
    assert!(matches!(
        single(&outcome),
        ImportItemReport::Duplicate { .. }
    ));
    assert_eq!(fixture.inbox_rows(), 1);
    assert_eq!(fixture.inbox_entries(), vec!["Задание.pdf"]);
}

#[test]
fn invalid_operation_id_and_empty_batch_are_rejected_before_any_work() {
    let fixture = Fixture::new();
    let source = fixture.external_file("a.pdf", b"a");
    fixture.grants.grant([source.clone()]);
    let error = fixture
        .import_ungranted("not-a-uuid", &[&source])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    let error = fixture
        .import_ungranted(&Uuid::new_v4().to_string(), &[])
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(fixture.inbox_rows(), 0);
}

#[test]
fn partial_failure_keeps_successful_files_and_completes_operation() {
    let fixture = Fixture::new();
    let good = fixture.external_file("good.txt", b"ok");
    let missing = fixture.external.join("missing.txt");
    let (outcome, progress) = fixture.import(&[&good, &missing]);
    assert_eq!(outcome.report.imported, 1);
    assert_eq!(outcome.report.failed, 1);
    assert!(outcome.failure.is_none());
    assert_eq!(outcome.operation.status, OperationStatus::Completed);
    assert_eq!(progress.len(), 3);
    assert_eq!(progress.last().unwrap().progress, 1.0);
}

// ───────────── Unicode и длинные имена ─────────────

#[test]
fn unicode_and_long_names_are_preserved_safely() {
    let fixture = Fixture::new();
    let emoji = fixture.external_file("ДЗ №3 — «Регрессия» 😀.xlsx", b"one");
    let colon = fixture.external_file("Итог: неделя 7.docx", b"two");
    let long_name = format!("{}.xlsx", "Ж".repeat(120));
    let long = fixture.external_file(&long_name, b"three");
    let (outcome, _) = fixture.import(&[&emoji, &colon, &long]);
    let names: Vec<String> = outcome
        .report
        .items
        .iter()
        .map(|item| match item {
            ImportItemReport::Imported { file, .. } => file.name.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(names[0], "ДЗ №3 — «Регрессия» 😀.xlsx");
    assert_eq!(names[1], "Итог- неделя 7.docx");
    assert!(names[2].len() <= super::naming::MAX_NAME_BYTES);
    assert!(names[2].ends_with(".xlsx"));
    assert!(names[2].starts_with("ЖЖЖ"));
    for name in &names {
        assert!(fixture.inbox().join(name).is_file(), "{name}");
    }

    // Конфликт для длинного имени тоже укладывается в лимит.
    let long_other = fixture.external_file(&format!("other/{long_name}"), b"four");
    let (outcome, _) = fixture.import(&[&long_other]);
    let ImportItemReport::Imported { file, .. } = single(&outcome) else {
        panic!();
    };
    assert!(file.name.ends_with(" (2).xlsx"));
    assert!(file.name.len() <= super::naming::MAX_NAME_BYTES);
}

// ───────────── Управляемые каталоги ─────────────

#[test]
fn roots_are_validated_stored_canonically_and_reported() {
    let fixture = Fixture::new();
    let service = fixture.roots_service();

    let status = service.status().unwrap();
    assert_eq!(status.vault.state, RootState::Ok);
    assert_eq!(status.inbox.state, RootState::Missing);
    assert_eq!(status.backup.state, RootState::NotConfigured);
    assert!(status.import_ready);

    assert_eq!(
        service
            .set_root(RootKind::Backup, &fixture.home)
            .unwrap_err()
            .code,
        ErrorCode::Blocked,
        "home directory is too broad"
    );
    let nested = fixture.vault.join("backup");
    fs::create_dir_all(&nested).unwrap();
    assert_eq!(
        service
            .set_root(RootKind::Backup, &nested)
            .unwrap_err()
            .code,
        ErrorCode::Conflict,
        "backup inside vault is rejected"
    );
    assert_eq!(
        service.set_root(RootKind::Inbox, &nested).unwrap_err().code,
        ErrorCode::Blocked
    );
    let linked = fixture.root.join("vault-link");
    symlink(&fixture.vault, &linked).unwrap();
    assert_eq!(
        service.set_root(RootKind::Vault, &linked).unwrap_err().code,
        ErrorCode::Blocked
    );

    let backup = fixture.root.join("backups");
    fs::create_dir_all(&backup).unwrap();
    let status = service.set_root(RootKind::Backup, &backup).unwrap();
    assert_eq!(status.backup.state, RootState::Ok);
    assert_eq!(
        service.config().unwrap().backup_path.as_deref(),
        Some(backup.to_str().unwrap())
    );

    let status = service.prepare_inbox().unwrap();
    assert_eq!(status.inbox.state, RootState::Ok);
    assert!(fixture.inbox().is_dir());

    fs::rename(&backup, fixture.root.join("moved")).unwrap();
    assert_eq!(service.status().unwrap().backup.state, RootState::Missing);
}

#[test]
fn display_path_abbreviates_home() {
    let fixture = Fixture::new();
    let inside_home = fixture.home.join("Documents").join("Obsidian");
    fs::create_dir_all(&inside_home).unwrap();
    let status = fixture
        .roots_service()
        .set_root(RootKind::Vault, &inside_home)
        .unwrap();
    assert_eq!(
        status.vault.display_path.as_deref(),
        Some("~/Documents/Obsidian")
    );
}

#[test]
fn import_without_configured_vault_is_blocked() {
    let db = Database::open_in_memory().unwrap();
    db.migrate().unwrap();
    let error = RootsService::new(&db, None)
        .open_import_roots()
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert!(error.message.contains("Хранилище"));
}
