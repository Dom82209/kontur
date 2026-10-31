use rusqlite::Connection;
use tempfile::TempDir;

use super::{SubjectInput, SubjectService};
use crate::{
    db::Database,
    dto::{ClassificationPatchDto, MaterialCategory, PatchField},
    error::ErrorCode,
    lms::export::classify_context,
    mutation_service::MutationService,
    repositories::ReadRepository,
};

fn database() -> Database {
    let database = Database::open_in_memory().unwrap();
    database.migrate().unwrap();
    database
}

fn input(name: &str, short: &str, keywords: &[&str]) -> SubjectInput {
    SubjectInput {
        name: name.to_owned(),
        short_name: short.to_owned(),
        keywords: keywords.iter().map(|k| (*k).to_owned()).collect(),
    }
}

fn exec(database: &Database, sql: &str) {
    database
        .with_connection(|connection| {
            connection.execute_batch(sql)?;
            Ok(())
        })
        .unwrap();
}

fn count(database: &Database, sql: &str) -> i64 {
    database
        .with_connection(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?))
        .unwrap()
}

/// Неделя с ДЗ и файлом у предмета.
fn add_week(database: &Database, subject: &str, folder: &str, number: i64) {
    exec(
        database,
        &format!(
            "INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path)
               VALUES ('{subject}-w{number}','{subject}',{number},'','','{folder}/Неделя {number:02}');
             INSERT INTO homeworks(id,week_id,title,deadline,stage,workflow_stage,folder_path)
               VALUES ('{subject}-h{number}','{subject}-w{number}','ДЗ {number}',NULL,'task','materials','{folder}/Неделя {number:02}/03 ДЗ');
             INSERT INTO files(id,week_id,homework_id,category,name,kind,size_bytes,relative_path,content_hash)
               VALUES ('{subject}-f{number}','{subject}-w{number}',NULL,'lecture','Лекция.pdf','pdf',1,'{folder}/Неделя {number:02}/01 Лекции/Лекция.pdf',NULL);"
        ),
    );
}

fn add_inbox(database: &Database, id: &str, subject: &str, source: &str) {
    exec(
        database,
        &format!(
            "INSERT INTO inbox_files(id,name,kind,size_bytes,added_at,origin,status,relative_path,classification_json)
             VALUES ('{id}','{id}.pdf','pdf',1,'2026-10-21T08:00:00Z','drop','ambiguous','Предметы/00 Входящие/{id}.pdf',
               '{{\"subjectId\":\"{subject}\",\"weekNumber\":null,\"category\":\"unknown\",\"confidence\":0.4,\"reasons\":[],\"source\":\"{source}\"}}')"
        ),
    );
}

#[test]
fn create_normalizes_input_and_derives_folder_without_touching_vault() {
    let database = database();
    let subject = SubjectService::new(&database)
        .create(&input(
            "  Теория   вероятностей ",
            "ТВ",
            &[" Вероятност", "вероятност", "ТВиМС"],
        ))
        .unwrap();
    assert_eq!(subject.name, "Теория вероятностей");
    assert_eq!(subject.folder_path, "Предметы/Теория вероятностей");
    assert_eq!(subject.keywords, vec!["вероятност", "твимс"]);
    assert!(!subject.archived);
    assert_eq!(subject.revision, 0);
}

#[test]
fn invalid_or_duplicate_names_are_rejected() {
    let database = database();
    let service = SubjectService::new(&database);
    service.create(&input("Статистика", "Стат", &[])).unwrap();
    for name in ["статистика", "СТАТИСТИКА "] {
        assert_eq!(
            service.create(&input(name, "С", &[])).unwrap_err().code,
            ErrorCode::Conflict,
            "{name}"
        );
    }
    for name in [
        "",
        "00 Входящие",
        ".hidden",
        "a/b",
        "Матан: продвинутый",
        "../x",
    ] {
        assert_eq!(
            service.create(&input(name, "X", &[])).unwrap_err().code,
            ErrorCode::Blocked,
            "{name}"
        );
    }
    assert_eq!(
        service.create(&input("Линал", "", &[])).unwrap_err().code,
        ErrorCode::Blocked
    );
}

#[test]
fn keyword_cannot_belong_to_two_active_subjects() {
    let database = database();
    let service = SubjectService::new(&database);
    service
        .create(&input("Статистика", "Стат", &["стат"]))
        .unwrap();
    assert_eq!(
        service
            .create(&input("Матстат", "МС", &["Стат"]))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(count(&database, "SELECT count(*) FROM subjects"), 1);
}

#[test]
fn rename_keeps_folder_and_stale_revision_is_rejected() {
    let database = database();
    let service = SubjectService::new(&database);
    let created = service.create(&input("Мяу", "Мяу", &["мяу"])).unwrap();
    let renamed = service
        .update(
            &created.id,
            0,
            &input("Математика и алгебра", "МиА", &["миа"]),
        )
        .unwrap();
    assert_eq!(renamed.name, "Математика и алгебра");
    assert_eq!(renamed.folder_path, "Предметы/Мяу");
    assert_eq!(renamed.keywords, vec!["миа"]);
    assert_eq!(renamed.revision, 1);
    assert_eq!(
        service
            .update(&created.id, 0, &input("Другое", "Д", &[]))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[test]
fn archived_subject_is_not_offered_and_manual_choice_is_blocked() {
    let database = database();
    let service = SubjectService::new(&database);
    let subject = service.create(&input("ОРГ", "ОРГ", &["орг"])).unwrap();
    add_week(&database, &subject.id, &subject.folder_path, 1);
    add_inbox(&database, "inbox-1", "", "auto");
    let archived = service.set_archived(&subject.id, 0, true).unwrap();
    assert!(archived.archived);
    // Данные сохранены.
    assert_eq!(count(&database, "SELECT count(*) FROM weeks"), 1);
    assert!(classify_context(&database, None)
        .unwrap()
        .subjects
        .iter()
        .all(|matcher| matcher.id != subject.id));
    assert!(ReadRepository::new(&database)
        .automation_rules()
        .unwrap()
        .subject_rules
        .is_empty());
    let error = MutationService::new(&database)
        .update_classification(
            "inbox-1",
            ClassificationPatchDto {
                homework_id: PatchField::Absent,
                subject_id: PatchField::Value(subject.id.clone()),
                week_number: PatchField::Absent,
                category: PatchField::Value(MaterialCategory::Lecture),
            },
        )
        .unwrap_err();
    assert_eq!(error.error.code, ErrorCode::Blocked);
    let restored = service.set_archived(&subject.id, 1, false).unwrap();
    assert!(!restored.archived);
}

#[test]
fn restore_is_blocked_when_keyword_was_taken_meanwhile() {
    let database = database();
    let service = SubjectService::new(&database);
    let old = service.create(&input("Старый", "С", &["эконом"])).unwrap();
    service.set_archived(&old.id, 0, true).unwrap();
    service
        .create(&input("Экономика", "Э", &["эконом"]))
        .unwrap();
    assert_eq!(
        service.set_archived(&old.id, 1, false).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn merge_preview_writes_nothing_and_apply_moves_weeks_keywords_and_inbox() {
    let database = database();
    let service = SubjectService::new(&database);
    let source = service.create(&input("Матан", "МА", &["матан"])).unwrap();
    let target = service
        .create(&input("Математический анализ", "МатАн", &["анализ"]))
        .unwrap();
    add_week(&database, &source.id, &source.folder_path, 1);
    add_week(&database, &source.id, &source.folder_path, 2);
    add_week(&database, &target.id, &target.folder_path, 3);
    add_inbox(&database, "auto", &source.id, "auto");
    add_inbox(&database, "manual", &source.id, "manual");

    let preview = service.preview_merge(&source.id, &target.id).unwrap();
    assert!(preview.can_merge);
    assert_eq!(preview.moved_weeks.len(), 2);
    assert_eq!(preview.retargeted_inbox, 2);
    assert_eq!(preview.added_keywords, vec!["матан"]);
    assert!(preview
        .effects
        .iter()
        .any(|effect| effect.contains("не перемещаются")));
    assert_eq!(
        count(
            &database,
            &format!(
                "SELECT count(*) FROM weeks WHERE subject_id='{}'",
                source.id
            )
        ),
        2,
        "preview ничего не меняет"
    );

    let result = service
        .merge(&source.id, &target.id, &preview.fingerprint)
        .unwrap();
    assert_eq!(result.moved_weeks, 2);
    assert_eq!(result.retargeted_inbox, 2);
    assert!(result.source.archived);
    assert_eq!(
        result.source.merged_into_id.as_deref(),
        Some(target.id.as_str())
    );
    assert_eq!(result.target.keywords, vec!["анализ", "матан"]);
    assert_eq!(result.target.week_count, 3);
    // Папки недель не меняются: файлы в vault остаются на месте.
    assert_eq!(
        count(
            &database,
            "SELECT count(*) FROM weeks WHERE folder_path LIKE 'Предметы/Матан/%'"
        ),
        2
    );
    assert_eq!(
        count(
            &database,
            &format!(
                "SELECT count(*) FROM inbox_files WHERE json_extract(classification_json,'$.subjectId')='{}'",
                target.id
            )
        ),
        2
    );
    // Ручное решение пользователя остаётся ручным.
    assert_eq!(
        count(
            &database,
            "SELECT count(*) FROM inbox_files WHERE id='manual' AND json_extract(classification_json,'$.source')='manual'"
        ),
        1
    );
    assert_eq!(count(&database, "SELECT count(*) FROM subject_merges"), 1);

    // Повторное слияние, возврат из архива и правка слитого предмета запрещены.
    assert_eq!(
        service
            .preview_merge(&source.id, &target.id)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    let revision = result.source.revision;
    assert_eq!(
        service
            .set_archived(&source.id, revision, false)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    assert_eq!(
        service
            .update(&source.id, revision, &input("Матан", "МА", &[]))
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    assert!(database
        .with_connection(|connection| {
            Ok(connection.execute("UPDATE subjects SET merged_into_id=NULL", [])?)
        })
        .is_err());
}

#[test]
fn merge_is_blocked_by_weeks_with_same_number_and_by_changes_after_preview() {
    let database = database();
    let service = SubjectService::new(&database);
    let source = service.create(&input("Линал", "ЛА", &[])).unwrap();
    let target = service
        .create(&input("Линейная алгебра", "ЛинАл", &[]))
        .unwrap();
    add_week(&database, &source.id, &source.folder_path, 2);
    add_week(&database, &target.id, &target.folder_path, 2);
    let preview = service.preview_merge(&source.id, &target.id).unwrap();
    assert!(!preview.can_merge);
    assert_eq!(preview.conflicting_weeks, vec![2]);
    assert_eq!(
        service
            .merge(&source.id, &target.id, &preview.fingerprint)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );

    let other = service.create(&input("Алгебра", "Алг", &[])).unwrap();
    let preview = service.preview_merge(&source.id, &other.id).unwrap();
    add_week(&database, &source.id, &source.folder_path, 5);
    assert_eq!(
        service
            .merge(&source.id, &other.id, &preview.fingerprint)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(count(&database, "SELECT count(*) FROM subject_merges"), 0);
    assert_eq!(
        service
            .preview_merge(&source.id, &source.id)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
    let archived = service.set_archived(&other.id, 0, true).unwrap();
    assert!(archived.archived);
    assert_eq!(
        service
            .preview_merge(&source.id, &other.id)
            .unwrap_err()
            .code,
        ErrorCode::Blocked
    );
}

#[test]
fn automation_rules_save_and_load_keywords_through_subject_table() {
    let database = database();
    let service = SubjectService::new(&database);
    let subject = service
        .create(&input("Статистика", "Стат", &["стат"]))
        .unwrap();
    let mut rules = ReadRepository::new(&database).automation_rules().unwrap();
    assert_eq!(rules.subject_rules.len(), 1);
    rules.subject_rules[0].keywords = vec!["Статистик".to_owned(), "вероятност".to_owned()];
    MutationService::new(&database)
        .update_automation_rules(rules)
        .unwrap();
    assert_eq!(
        service.subject(&subject.id).unwrap().keywords,
        vec!["вероятност", "статистик"]
    );
    let stored: String = database
        .with_connection(|connection| {
            Ok(connection.query_row(
                "SELECT value_json FROM app_settings WHERE key='automation_rules'",
                [],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert!(
        stored.contains("\"subjectRules\":[]"),
        "JSON не хранит копию ключевых слов"
    );
}

#[test]
fn schema_v12_moves_existing_keywords_from_settings() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("v11.sqlite3");
    {
        let mut connection = Connection::open(&path).unwrap();
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
        ];
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
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
                "INSERT INTO subjects(id,name,short_name,folder_path) VALUES ('stat','Статистика','Стат','Предметы/Статистика');
                 INSERT INTO app_settings(key,value_json,created_at,updated_at) VALUES ('automation_rules',
                   '{\"subjectRules\":[{\"subjectId\":\"stat\",\"keywords\":[\"стат\",\" вероятност \",\"x\"]},{\"subjectId\":\"gone\",\"keywords\":[\"нет\"]}]}',
                   CURRENT_TIMESTAMP, CURRENT_TIMESTAMP);",
            )
            .unwrap();
    }
    let database = Database::open_file(&path).unwrap();
    database.migrate().unwrap();
    let subjects = ReadRepository::new(&database).subjects().unwrap();
    assert_eq!(subjects.len(), 1);
    assert_eq!(subjects[0].keywords, vec!["вероятност", "стат"]);
    assert!(!subjects[0].archived);
    assert_eq!(count(&database, "SELECT count(*) FROM subject_keywords"), 2);
}

#[test]
fn inbox_classification_uses_subject_keywords_threshold_and_keeps_manual_choice() {
    use crate::{dto::InboxStatus, routing::RoutingService};
    let database = database();
    let service = SubjectService::new(&database);
    let subject = service
        .create(&input("Теория вероятностей", "ТВ", &["твимс"]))
        .unwrap();
    exec(
        &database,
        "INSERT INTO inbox_files(id,name,kind,size_bytes,added_at,origin,status,relative_path)
         VALUES ('k','ТВиМС неделя 3 лекция.pdf','pdf',1,'2026-10-21T08:00:00Z','drop','new','Предметы/00 Входящие/k.pdf')",
    );
    add_inbox(&database, "manual", "", "manual");
    let routing = RoutingService::new(&database, None);
    let result = routing
        .classify_files(&["k".to_owned(), "manual".to_owned()])
        .unwrap();
    let classified = &result.value[0];
    let classification = classified.classification.as_ref().unwrap();
    assert_eq!(
        classification.subject_id.as_deref(),
        Some(subject.id.as_str())
    );
    assert_eq!(classification.week_number, Some(3));
    assert_eq!(classified.status, InboxStatus::Classified);
    // Ручное решение не перезаписывается автоматикой.
    assert_eq!(
        result.value[1]
            .classification
            .as_ref()
            .unwrap()
            .subject_id
            .as_deref(),
        Some("")
    );

    // Порог из настроек, а не зашитые 0.7.
    let mut rules = ReadRepository::new(&database).automation_rules().unwrap();
    rules.low_confidence_threshold = 0.99;
    MutationService::new(&database)
        .update_automation_rules(rules)
        .unwrap();
    let result = routing.classify_files(&["k".to_owned()]).unwrap();
    assert!(result.value[0].classification.as_ref().unwrap().confidence < 0.99);
    assert_eq!(result.value[0].status, InboxStatus::Ambiguous);

    // Архивный предмет не предлагается.
    service.set_archived(&subject.id, 0, true).unwrap();
    let result = routing.classify_files(&["k".to_owned()]).unwrap();
    assert_eq!(
        result.value[0].classification.as_ref().unwrap().subject_id,
        None
    );
}
