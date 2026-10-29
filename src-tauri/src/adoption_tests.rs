use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

use tempfile::TempDir;

use super::{
    stable_id, AdoptionService, AdoptionStatus, CategoryOverride, FileCategory, PlanStatus,
    SkipReason,
};
use crate::{
    db::Database,
    error::ErrorCode,
    workspace::{read_identity, TransitionRequest, WorkspaceCatalog},
};

struct Fixture {
    _temp: TempDir,
    vault: PathBuf,
    database: Database,
}

fn write(vault: &Path, relative: &str, content: &str) {
    let path = vault.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Синтетическая копия структуры настоящего vault (без реальных материалов).
fn populate(vault: &Path) {
    write(vault, "00 Старт.md", "старт");
    write(vault, "Предметы/00 Входящие/README.md", "входящие");
    let oba = "Предметы/Основы бизнес-аналитики";
    write(
        vault,
        &format!("{oba}/00 Основы бизнес-аналитики.md"),
        "обзор",
    );
    write(
        vault,
        &format!("{oba}/00 Общие материалы/Программа курса.pdf"),
        "программа",
    );
    write(
        vault,
        &format!("{oba}/Неделя 02/00 Неделя 02.md"),
        "обзор недели",
    );
    write(
        vault,
        &format!("{oba}/Неделя 02/01 Лекции/README.md"),
        "readme",
    );
    write(
        vault,
        &format!("{oba}/Неделя 02/01 Лекции/Лекция 2.pdf"),
        "лекция 2",
    );
    write(vault, &format!("{oba}/Неделя 02/03 ДЗ/README.md"), "readme");
    write(
        vault,
        &format!("{oba}/Неделя 02/03 ДЗ/Студент_Тестовый_Неделя2_ДЗ.xlsx"),
        "решение 2",
    );
    write(
        vault,
        &format!("{oba}/Неделя 05/02 Семинары/Семинар 5.pdf"),
        "семинар 5",
    );
    write(
        vault,
        &format!("{oba}/Неделя 05/03 ДЗ/Неделя 5_ДЗ.pdf"),
        "условие 5",
    );
    write(
        vault,
        &format!("{oba}/Неделя 05/03 ДЗ/Приложение к ДЗ.xlsx"),
        "данные 5",
    );
    write(
        vault,
        &format!("{oba}/Неделя 05/03 ДЗ/week5.xlsx"),
        "что-то 5",
    );
    write(
        vault,
        &format!("{oba}/Неделя 05/04 Дополнительно/Статья.pdf"),
        "статья",
    );
    let matan = "Предметы/Математический анализ";
    write(
        vault,
        &format!("{matan}/Неделя 00/03 ДЗ/Вводное.pdf"),
        "вводное",
    );
    write(
        vault,
        &format!("{matan}/Неделя 01/01 Лекции/Пределы.pdf"),
        "пределы",
    );
    write(
        vault,
        &format!("{matan}/Неделя 01/03 ДЗ/README.md"),
        "только readme",
    );
    write(vault, "Предметы/.DS_Store", "x");
    write(vault, ".obsidian/app.json", "{}");
}

fn fixture() -> Fixture {
    let temp = TempDir::new().unwrap();
    let app = temp.path().join("app");
    let vault = temp.path().join("vaults").join("ЦУ");
    fs::create_dir_all(&vault).unwrap();
    let vault = fs::canonicalize(vault).unwrap();
    populate(&vault);
    let (catalog, _) = WorkspaceCatalog::bootstrap(&app, None).unwrap();
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&vault))
        .unwrap();
    catalog.confirm_transition(&preview.id).unwrap();
    drop(catalog);
    let (catalog, path) = WorkspaceCatalog::bootstrap(&app, None).unwrap();
    let database = Database::open_file(path).unwrap();
    database.migrate().unwrap();
    catalog.verify_active_database(&database).unwrap();
    Fixture {
        _temp: temp,
        vault,
        database,
    }
}

fn service(fixture: &Fixture) -> AdoptionService<'_> {
    AdoptionService::new(&fixture.database, None)
}

fn count(database: &Database, table: &str) -> i64 {
    database
        .with_connection(|connection| {
            Ok(
                connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap()
}

/// Полный снимок vault: путь → (содержимое, mtime). Включает скрытые файлы.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut output = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let meta = fs::symlink_metadata(entry.path()).unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                output.insert(
                    entry.path(),
                    (fs::read(entry.path()).unwrap(), meta.modified().unwrap()),
                );
            }
        }
    }
    output
}

fn plan_status(database: &Database, id: &str) -> String {
    database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT status FROM vault_adoption_plans WHERE id=?1",
                [id],
                |row| row.get(0),
            )?)
        })
        .unwrap()
}

#[test]
fn preview_is_read_only_and_reports_structure() {
    let fixture = fixture();
    let before = snapshot(&fixture.vault);
    let preview = service(&fixture).preview().unwrap();
    assert_eq!(snapshot(&fixture.vault), before);
    assert_eq!(count(&fixture.database, "subjects"), 0);
    assert_eq!(count(&fixture.database, "files"), 0);

    let summary = &preview.summary;
    assert_eq!(summary.subjects_new, 2);
    // ОБА: недели 02 и 05; матан: неделя 01 (неделя 00 не поддерживается).
    assert_eq!(summary.weeks_new, 3);
    // ДЗ недели 01 матана содержит только README — ДЗ не создаётся.
    assert_eq!(summary.homeworks_new, 2);
    assert_eq!(summary.files_new, 8);
    let reason = |path: &str| {
        preview
            .plan
            .skipped
            .iter()
            .find(|item| item.relative_path.ends_with(path))
            .map(|item| item.reason)
    };
    assert_eq!(
        reason("Неделя 00/03 ДЗ/Вводное.pdf"),
        Some(SkipReason::WeekZero)
    );
    assert_eq!(
        reason("00 Общие материалы/Программа курса.pdf"),
        Some(SkipReason::SubjectLevel)
    );
    assert_eq!(
        reason("01 Лекции/README.md"),
        Some(SkipReason::ObsidianNote)
    );
    assert_eq!(
        reason("00 Входящие/README.md"),
        Some(SkipReason::ServiceFolder)
    );
    let hidden = |path: &str| path.contains(".obsidian") || path.contains(".DS_Store");
    assert!(preview
        .plan
        .skipped
        .iter()
        .all(|item| !hidden(&item.relative_path)));
    assert!(preview
        .plan
        .files
        .iter()
        .all(|file| !hidden(&file.relative_path)));
    let category = |name: &str| {
        preview
            .plan
            .files
            .iter()
            .find(|file| file.name == name)
            .map(|file| (file.category, file.needs_review))
            .unwrap()
    };
    assert_eq!(category("Лекция 2.pdf"), (FileCategory::Lecture, false));
    assert_eq!(category("Семинар 5.pdf"), (FileCategory::Seminar, false));
    assert_eq!(category("Статья.pdf"), (FileCategory::Material, false));
    assert_eq!(
        category("Неделя 5_ДЗ.pdf"),
        (FileCategory::HomeworkTask, false)
    );
    assert_eq!(
        category("Приложение к ДЗ.xlsx"),
        (FileCategory::HomeworkData, false)
    );
    assert_eq!(
        category("Студент_Тестовый_Неделя2_ДЗ.xlsx"),
        (FileCategory::HomeworkDraft, false)
    );
    assert_eq!(category("week5.xlsx"), (FileCategory::HomeworkDraft, true));
    assert_eq!(summary.needs_review, 1);
}

#[test]
fn apply_creates_rows_with_stable_ids_and_relative_paths_and_is_idempotent() {
    let fixture = fixture();
    let before = snapshot(&fixture.vault);
    let preview = service(&fixture).preview().unwrap();
    let report = service(&fixture).apply(&preview.id, &[], false).unwrap();
    assert_eq!(report.status, PlanStatus::Applied);
    assert_eq!(report.created.subjects, 2);
    assert_eq!(report.created.weeks, 3);
    assert_eq!(report.created.homeworks, 2);
    assert_eq!(report.created.files, 8);
    assert_eq!(snapshot(&fixture.vault), before, "apply не трогает vault");
    assert_eq!(plan_status(&fixture.database, &preview.id), "applied");

    let workspace = read_identity(&fixture.database).unwrap().id;
    let path = "Предметы/Основы бизнес-аналитики/Неделя 05/03 ДЗ/Неделя 5_ДЗ.pdf";
    let (id, homework, hash): (String, Option<String>, String) = fixture
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT id, homework_id, content_hash FROM files WHERE relative_path=?1",
                [path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?)
        })
        .unwrap();
    assert_eq!(id, stable_id(&workspace, "file", path));
    assert!(homework.is_some());
    assert_eq!(hash.len(), 64);
    let absolute: i64 = fixture
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT COUNT(*) FROM files WHERE relative_path LIKE '/%'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(absolute, 0);
    // Неизвестный срок и даты недели не выдумываются.
    let (deadline, starts): (Option<String>, String) = fixture
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT h.deadline, w.starts_at FROM homeworks h JOIN weeks w ON w.id=h.week_id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(deadline, None);
    assert_eq!(starts, "");

    let second = service(&fixture).preview().unwrap();
    assert_eq!(second.summary.files_new, 0);
    assert_eq!(second.summary.files_existing, 8);
    assert_eq!(second.summary.subjects_existing, 2);
    let again = service(&fixture).apply(&second.id, &[], false).unwrap();
    assert_eq!(again.created.files, 0);
    assert_eq!(again.already_present, 2 + 3 + 2 + 8);
    assert_eq!(count(&fixture.database, "files"), 8);
    assert_eq!(
        count(&fixture.database, "vault_adoption_items"),
        2 + 3 + 2 + 8
    );
}

#[test]
fn dry_run_validates_everything_but_writes_nothing() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    let report = service(&fixture).apply(&preview.id, &[], true).unwrap();
    assert!(report.dry_run);
    assert_eq!(report.created.files, 8);
    assert_eq!(count(&fixture.database, "files"), 0);
    assert_eq!(count(&fixture.database, "subjects"), 0);
    assert_eq!(count(&fixture.database, "vault_adoption_items"), 0);
    assert_eq!(plan_status(&fixture.database, &preview.id), "previewed");
    service(&fixture).apply(&preview.id, &[], false).unwrap();
    assert_eq!(count(&fixture.database, "files"), 8);
}

#[test]
fn file_changed_after_preview_blocks_apply_without_partial_rows() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    write(
        &fixture.vault,
        "Предметы/Основы бизнес-аналитики/Неделя 05/02 Семинары/Семинар 5.pdf",
        "изменено",
    );
    let error = service(&fixture)
        .apply(&preview.id, &[], false)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(count(&fixture.database, "files"), 0);
    assert_eq!(count(&fixture.database, "subjects"), 0);
    assert_eq!(plan_status(&fixture.database, &preview.id), "failed");
    assert_eq!(
        service(&fixture)
            .apply(&preview.id, &[], false)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn file_replaced_by_symlink_after_preview_blocks_apply() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    let target = fixture
        .vault
        .join("Предметы/Основы бизнес-аналитики/Неделя 02/01 Лекции/Лекция 2.pdf");
    let outside = fixture.vault.parent().unwrap().join("outside.pdf");
    fs::write(&outside, "лекция 2").unwrap();
    fs::remove_file(&target).unwrap();
    symlink(&outside, &target).unwrap();
    assert!(service(&fixture).apply(&preview.id, &[], false).is_err());
    assert_eq!(count(&fixture.database, "files"), 0);
}

#[test]
fn symlinks_inside_vault_are_reported_and_never_followed() {
    let fixture = fixture();
    let outside = fixture.vault.parent().unwrap().join("private");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.pdf"), "secret").unwrap();
    let week = fixture
        .vault
        .join("Предметы/Основы бизнес-аналитики/Неделя 02");
    symlink(&outside, week.join("04 Дополнительно")).unwrap();
    symlink(outside.join("secret.pdf"), week.join("01 Лекции/link.pdf")).unwrap();
    let preview = service(&fixture).preview().unwrap();
    let skipped: Vec<_> = preview
        .plan
        .skipped
        .iter()
        .filter(|item| item.reason == SkipReason::Symlink)
        .map(|item| item.relative_path.as_str())
        .collect();
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    assert!(preview
        .plan
        .files
        .iter()
        .all(|file| !file.name.contains("secret") && file.name != "link.pdf"));
}

#[test]
fn manual_category_has_priority_and_invalid_override_keeps_plan() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    let lecture = "Предметы/Основы бизнес-аналитики/Неделя 02/01 Лекции/Лекция 2.pdf";
    let invalid = CategoryOverride {
        relative_path: lecture.to_owned(),
        category: FileCategory::HomeworkTask,
    };
    assert_eq!(
        service(&fixture)
            .apply(&preview.id, &[invalid], false)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    assert_eq!(plan_status(&fixture.database, &preview.id), "previewed");
    let week5 = "Предметы/Основы бизнес-аналитики/Неделя 05/03 ДЗ/week5.xlsx";
    let report = service(&fixture)
        .apply(
            &preview.id,
            &[CategoryOverride {
                relative_path: week5.to_owned(),
                category: FileCategory::HomeworkData,
            }],
            false,
        )
        .unwrap();
    assert_eq!(report.overrides_applied, 1);
    let category: String = fixture
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT category FROM files WHERE relative_path=?1",
                [week5],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(category, "homework_data");
}

#[test]
fn cancelled_or_expired_plan_cannot_be_applied() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    service(&fixture).cancel(&preview.id).unwrap();
    assert_eq!(
        service(&fixture)
            .apply(&preview.id, &[], false)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let preview = service(&fixture).preview().unwrap();
    fixture
        .database
        .with_connection(|connection| {
            // expires_at неизменяем триггером; имитируем течение времени заменой триггера.
            connection.execute_batch(&format!(
                "DROP TRIGGER vault_adoption_plans_immutable;
                 UPDATE vault_adoption_plans SET expires_at='2000-01-01T00:00:00Z' WHERE id='{}';",
                preview.id
            ))?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        service(&fixture)
            .apply(&preview.id, &[], false)
            .unwrap_err()
            .code,
        ErrorCode::ConfirmationRequired
    );
    assert_eq!(count(&fixture.database, "files"), 0);
}

#[test]
fn plan_contents_are_immutable() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    let result = fixture.database.with_connection(|connection| {
        Ok(connection.execute(
            "UPDATE vault_adoption_plans SET plan_json='{}' WHERE id=?1",
            [&preview.id],
        )?)
    });
    assert!(result.is_err());
    let delete = fixture.database.with_connection(|connection| {
        Ok(connection.execute("DELETE FROM vault_adoption_plans", [])?)
    });
    assert!(delete.is_err());
}

#[test]
fn file_changed_after_import_is_reported_and_not_overwritten() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    service(&fixture).apply(&preview.id, &[], false).unwrap();
    let path = "Предметы/Основы бизнес-аналитики/Неделя 05/03 ДЗ/week5.xlsx";
    write(&fixture.vault, path, "новая версия");
    let second = service(&fixture).preview().unwrap();
    let changed = second
        .plan
        .files
        .iter()
        .find(|file| file.relative_path == path)
        .unwrap();
    assert_eq!(changed.status, AdoptionStatus::Changed);
    assert_eq!(second.summary.files_changed, 1);
    let before: String = fixture
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT content_hash FROM files WHERE relative_path=?1",
                [path],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    let report = service(&fixture).apply(&second.id, &[], false).unwrap();
    assert_eq!(report.changed_not_applied, 1);
    let after: String = fixture
        .database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT content_hash FROM files WHERE relative_path=?1",
                [path],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn subject_with_same_name_in_other_folder_is_a_conflict() {
    let fixture = fixture();
    fixture
        .database
        .with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO subjects(id,name,short_name,folder_path)
                 VALUES ('manual','Основы бизнес-аналитики','ОБА','Предметы/ОБА')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let preview = service(&fixture).preview().unwrap();
    // Предмет и две его недели; ДЗ внутри конфликтующих недель не планируются.
    assert_eq!(preview.summary.conflicts, 1 + 2);
    assert!(preview
        .plan
        .files
        .iter()
        .all(|file| !file.relative_path.contains("бизнес-аналитики")));
    service(&fixture).apply(&preview.id, &[], false).unwrap();
    // Только матан: предмет, неделя 01, лекция.
    assert_eq!(count(&fixture.database, "subjects"), 2);
    assert_eq!(count(&fixture.database, "files"), 1);
}

#[test]
fn database_changed_after_preview_rolls_back_whole_apply() {
    let fixture = fixture();
    let preview = service(&fixture).preview().unwrap();
    fixture
        .database
        .with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO subjects(id,name,short_name,folder_path)
                 VALUES ('late','Математический анализ','МА','Предметы/Другая папка')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let error = service(&fixture)
        .apply(&preview.id, &[], false)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(count(&fixture.database, "subjects"), 1);
    assert_eq!(count(&fixture.database, "files"), 0);
    assert_eq!(plan_status(&fixture.database, &preview.id), "failed");
}

#[test]
fn replaced_vault_blocks_preview() {
    let fixture = fixture();
    let moved = fixture.vault.with_file_name("ЦУ-old");
    fs::rename(&fixture.vault, &moved).unwrap();
    fs::create_dir_all(&fixture.vault).unwrap();
    populate(&fixture.vault);
    assert_eq!(
        service(&fixture).preview().unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn heuristics_recognise_student_work_and_lms_tasks_from_real_names() {
    use super::{
        classify_homework, has_lms_suffix, has_version_suffix, transliterate, StudentHints,
    };
    assert_eq!(transliterate("гончарова"), "goncharova");
    let hints = StudentHints::new("Гончарова");
    let category = |name: &str| {
        let classified = classify_homework(&[name], name, &hints).unwrap();
        (classified.category, classified.confidence >= 0.7)
    };
    // Своя работа: фамилия кириллицей через пробел и латиницей.
    assert_eq!(
        category("Кейс_1_Гончарова Доминика.pdf"),
        (FileCategory::HomeworkDraft, true)
    );
    assert_eq!(
        category("HW_5 Goncharova Dominika.pdf"),
        (FileCategory::HomeworkDraft, true)
    );
    // Условие из LMS: хэш-суффикс.
    assert!(has_lms_suffix("Неделя 2_Бизнес-студия_ДЗ 1.1_01b5a28b.pdf"));
    assert!(!has_lms_suffix("Текст_защиты_ДЗ1.3.docx"));
    assert_eq!(
        category("Неделя 4_ОБА_ДЗ_0122060b.pdf"),
        (FileCategory::HomeworkTask, true)
    );
    // Версии и LaTeX — собственная работа.
    assert!(has_version_suffix("МіА — Домашняя работа 5 — v01.pdf"));
    assert!(has_version_suffix("Кейс преза_v02.pptx"));
    assert!(!has_version_suffix("Матан. Домашняя работа ~1.pdf"));
    assert_eq!(
        category("стата_дз2.tex"),
        (FileCategory::HomeworkDraft, true)
    );
    // Документ с «ДЗ» без LMS-суффикса — только с ручной проверкой.
    assert_eq!(
        category("Текст_защиты_ДЗ1.3.docx"),
        (FileCategory::HomeworkTask, false)
    );
    // Без настроенного имени фамилия не угадывается.
    assert!(!StudentHints::new("Не задано").matches("гончарова"));
}

#[test]
#[ignore]
fn mirror_of_real_vault_preview() {
    let list = fs::read_to_string(std::env::var("KONTUR_MIRROR_PATHS").unwrap()).unwrap();
    let temp = TempDir::new().unwrap();
    let app = temp.path().join("app");
    let vault = temp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    let vault = fs::canonicalize(vault).unwrap();
    for (index, line) in list.lines().enumerate() {
        let path = line.split('\t').nth(1).unwrap();
        write(&vault, path, &format!("placeholder {index}"));
    }
    let (catalog, _) = WorkspaceCatalog::bootstrap(&app, None).unwrap();
    let preview = catalog
        .preview_transition(&TransitionRequest::CreateReal, Some(&vault))
        .unwrap();
    catalog.confirm_transition(&preview.id).unwrap();
    drop(catalog);
    let (catalog, path) = WorkspaceCatalog::bootstrap(&app, None).unwrap();
    let database = Database::open_file(path).unwrap();
    database.migrate().unwrap();
    catalog.verify_active_database(&database).unwrap();
    database
        .with_transaction(|tx| {
            tx.execute(
                "INSERT INTO app_settings(key,value_json,created_at,updated_at) VALUES ('automation_rules','{\"studentName\":{\"lastName\":\"Гончарова\",\"firstName\":\"Доминика\"}}',CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let service = AdoptionService::new(&database, None);
    let preview = service.preview().unwrap();
    println!("SUMMARY {:?}", preview.summary);
    let mut reasons = std::collections::BTreeMap::<String, usize>::new();
    for item in &preview.plan.skipped {
        *reasons.entry(format!("{:?}", item.reason)).or_default() += 1;
    }
    println!("SKIPPED {reasons:?}");
    let mut cats = std::collections::BTreeMap::<String, usize>::new();
    for file in &preview.plan.files {
        *cats.entry(format!("{:?}", file.category)).or_default() += 1;
    }
    println!("CATEGORIES {cats:?}");
    for file in preview.plan.files.iter().filter(|f| f.needs_review) {
        println!("REVIEW {:?} {}", file.category, file.relative_path);
    }
    for file in preview
        .plan
        .files
        .iter()
        .filter(|f| f.category == FileCategory::HomeworkTask && !f.needs_review)
        .take(8)
    {
        println!("TASK {}", file.relative_path);
    }
    let report = service.apply(&preview.id, &[], true).unwrap();
    println!("DRYRUN {report:?}");
}
