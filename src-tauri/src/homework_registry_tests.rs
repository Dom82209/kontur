use rusqlite::Connection;
use tempfile::TempDir;

use super::{normalize_deadline, HomeworkRegistry, NewHomeworkInput};
use crate::{db::Database, error::ErrorCode, repositories::ReadRepository};

fn database() -> Database {
    let database = Database::open_in_memory().unwrap();
    database.migrate().unwrap();
    database
        .with_connection(|c| {
            c.execute_batch(
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES ('stat','Статистика','Стат','Предметы/Статистика');
                 INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES ('w5','stat',5,'','','Предметы/Статистика/Неделя 05');",
            )?;
            Ok(())
        })
        .unwrap();
    database
}

fn input(title: &str, deadline: Option<&str>) -> NewHomeworkInput {
    NewHomeworkInput {
        subject_id: "stat".into(),
        week_number: 5,
        title: title.into(),
        deadline: deadline.map(str::to_owned),
    }
}

#[test]
fn three_homeworks_in_one_week_get_ordinals_and_folders() {
    let database = database();
    let registry = HomeworkRegistry::new(&database);
    let first = registry.create(&input("ДЗ 5", None)).unwrap();
    let second = registry
        .create(&input(
            " Кейс  «Золотая жила» ",
            Some("2026-11-05T23:59:00+03:00"),
        ))
        .unwrap();
    let third = registry.create(&input("Тест", None)).unwrap();
    assert_eq!((first.ordinal, second.ordinal, third.ordinal), (1, 2, 3));
    assert_eq!(first.folder_path, "Предметы/Статистика/Неделя 05/03 ДЗ");
    assert_eq!(
        second.folder_path,
        "Предметы/Статистика/Неделя 05/03 ДЗ (2)"
    );
    assert_eq!(second.title, "Кейс «Золотая жила»");
    assert_eq!(second.deadline.as_deref(), Some("2026-11-05T20:59:00Z"));
    assert_eq!(first.deadline, None);
    let week = ReadRepository::new(&database).week("stat", 5).unwrap();
    assert_eq!(week.homework_ids, vec![first.id, second.id, third.id]);
    assert_eq!(
        ReadRepository::new(&database)
            .homework_list()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn invalid_input_is_rejected_and_identity_is_immutable() {
    let database = database();
    let registry = HomeworkRegistry::new(&database);
    assert_eq!(
        registry.create(&input("  ", None)).unwrap_err().code,
        ErrorCode::Blocked
    );
    assert_eq!(
        registry
            .create(&input("ДЗ", Some("завтра")))
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    let missing = NewHomeworkInput {
        week_number: 9,
        ..input("ДЗ", None)
    };
    assert_eq!(
        registry.create(&missing).unwrap_err().code,
        ErrorCode::NotFound
    );
    let created = registry.create(&input("ДЗ", None)).unwrap();
    for sql in [
        "UPDATE homeworks SET ordinal=7",
        "UPDATE homeworks SET week_id='other'",
    ] {
        assert!(
            database
                .with_connection(|c| Ok(c.execute(sql, [])?))
                .is_err(),
            "{sql}"
        );
    }
    database
        .with_connection(|c| {
            c.execute(
                "UPDATE homeworks SET import_key='lms:1' WHERE id=?1",
                [&created.id],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(
        database
            .with_connection(|c| Ok(c.execute("UPDATE homeworks SET import_key='lms:2'", [])?))
            .is_err(),
        "import_key задаётся один раз"
    );
    assert!(database
        .with_connection(|c| Ok(c.execute(
            "INSERT INTO homeworks(id,week_id,ordinal,title,deadline,stage,workflow_stage,folder_path)
             VALUES ('dup','w5',1,'Дубль',NULL,'task','materials','Предметы/Статистика/Неделя 05/x')",
            []
        )?))
        .is_err(), "(week, ordinal) уникальны");
    database
        .with_connection(|c| {
            c.execute(
                "UPDATE subjects SET archived_at='2026-10-01T00:00:00Z' WHERE id='stat'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        registry.create(&input("ДЗ 2", None)).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn deadline_is_normalized_to_utc_or_unknown() {
    assert_eq!(normalize_deadline(None).unwrap(), None);
    assert_eq!(normalize_deadline(Some("  ")).unwrap(), None);
    assert_eq!(
        normalize_deadline(Some("2026-12-31T23:59:00+03:00"))
            .unwrap()
            .as_deref(),
        Some("2026-12-31T20:59:00Z")
    );
    assert_eq!(
        normalize_deadline(Some("2027-01-01T00:30:00+05:00"))
            .unwrap()
            .as_deref(),
        Some("2026-12-31T19:30:00Z"),
        "граница года"
    );
}

/// Обновление схемы v12 → v13 не теряет ДЗ и зависимые строки.
#[test]
fn schema_v13_upgrade_keeps_every_homework_and_dependent_row() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("v12.sqlite3");
    {
        let mut connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        let migrations = [
            include_str!("db/migrations/v1.sql"),
            include_str!("db/migrations/v2.sql"),
            include_str!("db/migrations/v3.sql"),
            include_str!("db/migrations/v4.sql"),
            include_str!("db/migrations/v5.sql"),
            include_str!("db/migrations/v6.sql"),
            include_str!("db/migrations/v7.sql"),
            include_str!("db/migrations/v8.sql"),
            include_str!("db/migrations/v9.sql"),
            include_str!("db/migrations/v10.sql"),
            include_str!("db/migrations/v11.sql"),
            include_str!("db/migrations/v12.sql"),
        ];
        for (index, sql) in migrations.iter().enumerate() {
            let transaction = connection.transaction().unwrap();
            transaction.execute_batch(sql).unwrap();
            transaction
                .pragma_update(None, "user_version", index as i64 + 1)
                .unwrap();
            transaction.commit().unwrap();
        }
        connection
            .execute_batch(
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES ('s','Стат','С','Предметы/Стат');
                 INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES
                   ('w1','s',1,'','','Предметы/Стат/Неделя 01'),('w2','s',2,'','','Предметы/Стат/Неделя 02');
                 INSERT INTO homeworks(id,week_id,title,deadline,stage,workflow_stage,folder_path) VALUES
                   ('h1','w1','ДЗ 1','2026-10-01T20:59:00Z','task','materials','Предметы/Стат/Неделя 01/03 ДЗ'),
                   ('h2','w2','ДЗ 2',NULL,'task','materials','Предметы/Стат/Неделя 02/03 ДЗ');
                 INSERT INTO files(id,week_id,homework_id,category,name,kind,size_bytes,relative_path) VALUES
                   ('f1','w1','h1','homework_task','a.pdf','pdf',1,'Предметы/Стат/Неделя 01/03 ДЗ/a.pdf'),
                   ('f2','w2','h2','homework_draft','b.xlsx','xlsx',1,'Предметы/Стат/Неделя 02/03 ДЗ/b.xlsx');
                 INSERT INTO vault_adoption_plans(id,workspace_id,vault_fingerprint,status,plan_json,plan_sha256,created_at,expires_at,finished_at)
                   VALUES ('p','ws','fp','applied','{}','0000000000000000000000000000000000000000000000000000000000000000','t','t','t');
                 INSERT INTO vault_adoption_items(plan_id,entity_kind,entity_id,relative_path) VALUES ('p','homework','h2','Предметы/Стат/Неделя 02/03 ДЗ');
                 UPDATE homeworks SET stage='submitted', submitted_at='2026-10-02T10:00:00Z' WHERE id='h1';",
            )
            .unwrap();
    }
    let database = Database::open_file(&path).unwrap();
    database.migrate().unwrap();
    let rows: Vec<(String, i64, Option<String>, String)> = database
        .with_connection(|c| {
            let mut q =
                c.prepare("SELECT id, ordinal, import_key, stage FROM homeworks ORDER BY id")?;
            let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("h1".to_owned(), 1, None, "submitted".to_owned()),
            (
                "h2".to_owned(),
                1,
                Some("vault:Предметы/Стат/Неделя 02/03 ДЗ".to_owned()),
                "task".to_owned()
            ),
        ]
    );
    let files: i64 = database
        .with_connection(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM files WHERE homework_id IS NOT NULL",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(files, 2, "каскадное удаление при перестройке не произошло");
    let violations: i64 = database
        .with_connection(|c| {
            Ok(
                c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(violations, 0);
    let fk_on: bool = database
        .with_connection(|c| Ok(c.pragma_query_value(None, "foreign_keys", |r| r.get(0))?))
        .unwrap();
    assert!(fk_on, "внешние ключи снова включены");
    // Защита сданного ДЗ пережила перестройку.
    assert!(database
        .with_connection(|c| Ok(c.execute("UPDATE homeworks SET title='x' WHERE id='h1'", [])?))
        .is_err());
    assert!(database
        .with_connection(|c| Ok(c.execute(
            "INSERT INTO files(id,week_id,homework_id,category,name,kind,size_bytes,relative_path)
             VALUES ('f3','w1','h1','homework_task','c.pdf','pdf',1,'Предметы/Стат/Неделя 01/03 ДЗ/c.pdf')",
            []
        )?))
        .is_err());
}

/// Этап 8: обновление до v16 сохраняет прежний комплект как ревизию 1 и квитанцию.
#[test]
fn schema_v16_upgrade_keeps_legacy_package_as_revision_one() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("v12-package.sqlite3");
    {
        let mut connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        let migrations = [
            include_str!("db/migrations/v1.sql"),
            include_str!("db/migrations/v2.sql"),
            include_str!("db/migrations/v3.sql"),
            include_str!("db/migrations/v4.sql"),
            include_str!("db/migrations/v5.sql"),
            include_str!("db/migrations/v6.sql"),
            include_str!("db/migrations/v7.sql"),
            include_str!("db/migrations/v8.sql"),
            include_str!("db/migrations/v9.sql"),
            include_str!("db/migrations/v10.sql"),
            include_str!("db/migrations/v11.sql"),
            include_str!("db/migrations/v12.sql"),
        ];
        for (index, sql) in migrations.iter().enumerate() {
            let transaction = connection.transaction().unwrap();
            transaction.execute_batch(sql).unwrap();
            transaction
                .pragma_update(None, "user_version", index as i64 + 1)
                .unwrap();
            transaction.commit().unwrap();
        }
        connection
            .execute_batch(
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES ('s','Стат','С','Предметы/Стат');
                 INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES ('w1','s',1,'','','Предметы/Стат/Неделя 01');
                 INSERT INTO homeworks(id,week_id,title,deadline,stage,workflow_stage,folder_path) VALUES
                   ('h1','w1','ДЗ 1',NULL,'ready_to_submit','ready_to_submit','Предметы/Стат/Неделя 01/03 ДЗ');
                 INSERT INTO files(id,week_id,homework_id,category,name,kind,size_bytes,relative_path) VALUES
                   ('f1','w1','h1','homework_draft','a.md','text',1,'Предметы/Стат/Неделя 01/03 ДЗ/02 Работа/a.md');
                 INSERT INTO homework_versions(id,homework_id,number,label,file_path,size_bytes,author,file_name,source_file_id,sections_json,files_json)
                   VALUES ('v1','h1',1,'v01','Предметы/Стат/Неделя 01/03 ДЗ/02 Работа/a.md',1,'user','a.md','f1','[]','[]');
                 UPDATE homeworks SET final_version_id='v1' WHERE id='h1';
                 INSERT INTO submission_packages(id,homework_id,final_version_id,status,package_path,built_at)
                   VALUES ('pkg','h1','v1','ready','Предметы/Стат/Неделя 01/03 ДЗ/03 На сдачу','2026-10-01T10:00:00Z');
                 INSERT INTO submission_receipts(id,package_id,homework_id,submitted_version_id,snapshot_path,snapshot_hash,confirmed_at,submitted_at,assignment_url)
                   VALUES ('r','pkg','h1','v1','Предметы/Стат/Неделя 01/03 ДЗ/04 Отправлено/x','hash','2026-10-01T11:00:00Z','2026-10-01T11:00:00Z','https://my.centraluniversity.ru/t');",
            )
            .unwrap();
    }
    let database = Database::open_file(&path).unwrap();
    database.migrate().unwrap();
    let package = database
        .with_connection(|c| crate::workflow::repository::submission_package(c, "h1"))
        .unwrap()
        .unwrap();
    assert_eq!(package.revision, 1);
    assert!(
        package.stale,
        "комплект до этапа 7 не сопоставлен с входными данными"
    );
    let receipt: String = database
        .with_connection(|c| {
            Ok(
                c.query_row("SELECT package_id FROM submission_receipts", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(receipt, "pkg");
}

#[test]
fn schema_v18_upgrade_gives_legacy_versions_clear_origin_and_no_parent() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("v17-versions.sqlite3");
    {
        let mut connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        let migrations = [
            include_str!("db/migrations/v1.sql"),
            include_str!("db/migrations/v2.sql"),
            include_str!("db/migrations/v3.sql"),
            include_str!("db/migrations/v4.sql"),
            include_str!("db/migrations/v5.sql"),
            include_str!("db/migrations/v6.sql"),
            include_str!("db/migrations/v7.sql"),
            include_str!("db/migrations/v8.sql"),
            include_str!("db/migrations/v9.sql"),
            include_str!("db/migrations/v10.sql"),
            include_str!("db/migrations/v11.sql"),
            include_str!("db/migrations/v12.sql"),
            include_str!("db/migrations/v13.sql"),
            include_str!("db/migrations/v14.sql"),
            include_str!("db/migrations/v15.sql"),
            include_str!("db/migrations/v16.sql"),
            include_str!("db/migrations/v17.sql"),
        ];
        for (index, sql) in migrations.iter().enumerate() {
            let transaction = connection.transaction().unwrap();
            transaction.execute_batch(sql).unwrap();
            transaction
                .pragma_update(None, "user_version", index as i64 + 1)
                .unwrap();
            transaction.commit().unwrap();
        }
        connection
            .execute_batch(
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES ('s','Стат','С','Предметы/Стат');
                 INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES ('w1','s',1,'','','Предметы/Стат/Неделя 01');
                 INSERT INTO homeworks(id,week_id,title,deadline,stage,workflow_stage,folder_path) VALUES
                   ('h1','w1','ДЗ 1',NULL,'in_progress','review','Предметы/Стат/Неделя 01/03 ДЗ');
                 INSERT INTO files(id,week_id,homework_id,category,name,kind,size_bytes,relative_path) VALUES
                   ('f1','w1','h1','homework_draft','a.md','text',1,'Предметы/Стат/Неделя 01/03 ДЗ/02 Работа/a.md');
                 INSERT INTO homework_versions(id,homework_id,number,label,file_path,size_bytes,author,file_name,source_file_id,sections_json,files_json)
                   VALUES ('v1','h1',1,'v01','Предметы/Стат/Неделя 01/03 ДЗ/02 Работа/a.md',1,'claude','a.md','f1','[]','[]'),
                          ('v2','h1',2,'v02','Предметы/Стат/Неделя 01/03 ДЗ/02 Работа/b.md',1,'user','b.md','f1','[]','[]');",
            )
            .unwrap();
    }
    let database = Database::open_file(&path).unwrap();
    database.migrate().unwrap();
    let versions = database
        .with_connection(|c| crate::workflow::repository::homework_dto(c, "h1"))
        .unwrap()
        .versions;
    assert_eq!(versions.len(), 2);
    for version in &versions {
        assert_eq!(
            version.lineage.parent_version_id, None,
            "родитель прежних версий неизвестен"
        );
        assert!(version
            .lineage
            .creation_reason
            .contains("родитель неизвестен"));
        assert_eq!(
            version.lineage.decision,
            crate::dto::VersionDecision::Pending
        );
    }
    assert_eq!(
        versions[0].lineage.origin,
        crate::dto::VersionOrigin::ClaudeResult
    );
    assert_eq!(
        versions[1].lineage.origin,
        crate::dto::VersionOrigin::UserImport
    );
    // Неизменяемость прежних версий восстановлена после разметки.
    let update = database.with_connection(|c| {
        c.execute("UPDATE homework_versions SET note = 'x'", [])?;
        Ok(())
    });
    assert!(update.is_err());
}
