use std::collections::HashSet;

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::{
    db::Database,
    dto::{
        ClassificationDto, ClassificationSource, DuplicateInfoDto, InboxFileDto, InboxStatus,
        MaterialCategory, OperationKind, OperationResultDto, RouteActionDto, RouteDestinationDto,
        RoutePreviewDto, WeekDto,
    },
    error::{AppError, BackendError},
    file_service::{
        atomic_copy_committed, hash_source, plan_copy, stage_created_files, validate_created_undo,
        write_managed_markdown, CopyDecision, CreatedFileUndoEntry, ManagedMarkdownWrite,
        PersistentPath, SourceFile, UndoContext, UndoManifest, VerifiedDestination,
    },
    intake::RootsService,
    repositories::{writes, ReadRepository},
};

pub struct RoutingService<'a> {
    database: &'a Database,
    roots: RootsService<'a>,
}

impl<'a> RoutingService<'a> {
    pub fn new(database: &'a Database, home: Option<std::path::PathBuf>) -> Self {
        Self {
            database,
            roots: RootsService::new(database, home),
        }
    }

    pub fn create_week(
        &self,
        subject_id: &str,
        number: i64,
    ) -> Result<OperationResultDto<WeekDto>, AppError> {
        if number <= 0 {
            return Err(BackendError::blocked(
                "Номер недели должен быть положительным",
            ));
        }
        let (subject_folder, week_id, week_path) = self.database.with_connection(|connection| {
            let folder: String = connection
                .query_row(
                    "SELECT folder_path FROM subjects WHERE id=?1",
                    [subject_id],
                    |r| r.get(0),
                )
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        BackendError::not_found("Предмет не найден")
                    }
                    other => other.into(),
                })?;
            Ok((
                folder.clone(),
                Uuid::new_v4().to_string(),
                format!("{folder}/Неделя {number:02}"),
            ))
        })?;
        let vault = self.roots.open_vault()?;
        for suffix in [
            "01 Лекции",
            "02 Семинары",
            "03 ДЗ/01 Условие и данные",
            "03 ДЗ/02 Работа",
            "03 ДЗ/03 На сдачу",
            "03 ДЗ/04 Отправлено",
            "03 ДЗ/05 Служебные данные",
            "04 Дополнительно",
        ] {
            vault.ensure_directory(&PersistentPath::new(format!("{week_path}/{suffix}"))?)?;
        }
        let operation_id = Uuid::new_v4().to_string();
        let result = self.database.with_transaction(|tx| {
            writes::insert_running_operation(tx, &operation_id, OperationKind::CreateWeek, "Создание учебной недели")?;
            tx.execute("INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES(?1,?2,?3,strftime('%Y-%m-%dT00:00:00Z','now'),strftime('%Y-%m-%dT23:59:59Z','now','+6 days'),?4)", params![week_id,subject_id,number,week_path])?;
            writes::complete_operation(tx, &operation_id)?;
            Ok(())
        });
        result?;
        let week = ReadRepository::new(self.database).week(subject_id, number)?;
        let operation = ReadRepository::new(self.database).operation(&operation_id)?;
        let _ = subject_folder;
        Ok(OperationResultDto {
            operation,
            value: week,
        })
    }

    pub fn classify_files(
        &self,
        ids: &[String],
    ) -> Result<OperationResultDto<Vec<InboxFileDto>>, AppError> {
        if ids.is_empty() {
            return Err(BackendError::blocked("Не выбраны файлы для классификации"));
        }
        let operation_id = Uuid::new_v4().to_string();
        self.database.with_transaction(|tx| {
            writes::insert_running_operation(tx, &operation_id, OperationKind::Classify, "Классификация файлов")?;
            let mut subjects = Vec::new();
            { let mut q=tx.prepare("SELECT id,name,short_name FROM subjects")?; let rows=q.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?; for row in rows { subjects.push(row?); } }
            for id in ids {
                let (name, prior): (String, Option<String>) = tx.query_row("SELECT name,classification_json FROM inbox_files WHERE id=?1 AND status<>'routed'", [id], |r| Ok((r.get(0)?,r.get(1)?)))
                    .map_err(|e| match e { rusqlite::Error::QueryReturnedNoRows => BackendError::not_found("Файл во «Входящих» не найден"), other=>other.into() })?;
                if prior.as_deref().and_then(|raw| serde_json::from_str::<ClassificationDto>(raw).ok()).is_some_and(|c| c.source==ClassificationSource::Manual) { continue; }
                let classification = classify_name(&name, &subjects);
                let status = if classification.subject_id.is_some() && classification.week_number.is_some() && classification.category != MaterialCategory::Unknown && classification.confidence >= 0.7 { InboxStatus::Classified } else { InboxStatus::Ambiguous };
                writes::update_inbox_classification(tx, id, &classification, status)?;
            }
            writes::complete_operation(tx, &operation_id)?; Ok(())
        })?;
        let repository = ReadRepository::new(self.database);
        let value = ids
            .iter()
            .map(|id| repository.inbox_file(id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OperationResultDto {
            operation: repository.operation(&operation_id)?,
            value,
        })
    }

    pub fn preview(
        &self,
        file_id: &str,
        destination: &RouteDestinationDto,
    ) -> Result<RoutePreviewDto, AppError> {
        let vault = self.roots.open_vault()?;
        let row = route_row(self.database, file_id, destination)?;
        let source_path = PersistentPath::new(&row.source_path)?;
        let source =
            SourceFile::open_in(&VerifiedDestination::in_vault(&vault, source_path.clone())?)?;
        let initial = PersistentPath::new(format!("{}/{}", row.folder, row.name))?;
        let decision = plan_copy(
            &source,
            &VerifiedDestination::in_vault(&vault, initial.clone())?,
        )?;
        let creates_week = !row.week_exists;
        let (target_name, action, duplicate, sha_match, name_conflict, renamed) = match decision {
            CopyDecision::Copy => (
                row.name.clone(),
                RouteActionDto::Copy,
                None,
                false,
                false,
                false,
            ),
            CopyDecision::SameContent => (
                row.name.clone(),
                RouteActionDto::Skip,
                Some(DuplicateInfoDto {
                    existing_path: initial.as_str().to_owned(),
                    same_content: true,
                    suggested_name: row.name.clone(),
                }),
                true,
                false,
                false,
            ),
            CopyDecision::NameConflict { suggested_name } => (
                suggested_name.clone(),
                RouteActionDto::Rename,
                Some(DuplicateInfoDto {
                    existing_path: initial.as_str().to_owned(),
                    same_content: false,
                    suggested_name: suggested_name.clone(),
                }),
                false,
                true,
                true,
            ),
        };
        let target = PersistentPath::new(format!("{}/{}", row.folder, target_name))?;
        let mut creates_directories = Vec::new();
        if creates_week {
            creates_directories.push(row.week_folder.clone());
        }
        creates_directories.push(row.folder.clone());
        Ok(RoutePreviewDto {
            file_id: file_id.to_owned(),
            source_name: row.name,
            source_relative_path: source_path.as_str().to_owned(),
            target_path: target.as_str().to_owned(),
            target_name,
            creates_week,
            creates_directories,
            sha256_match: sha_match,
            name_conflict,
            renamed,
            action,
            undoable: true,
            duplicate,
        })
    }

    pub fn route(
        &self,
        ids: &[String],
        destination: Option<&RouteDestinationDto>,
    ) -> Result<OperationResultDto<Vec<RoutePreviewDto>>, AppError> {
        if ids.is_empty() {
            return Err(BackendError::blocked("Не выбраны файлы для раскладки"));
        }
        let mut previews = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            let dest = match destination {
                Some(v) => v.clone(),
                None => classification_destination(self.database, id)?,
            };
            let p = self.preview(id, &dest)?;
            if !seen.insert(p.target_path.clone()) {
                return Err(BackendError::conflict(
                    "Несколько файлов претендуют на один целевой путь",
                ));
            }
            previews.push((p, dest));
        }
        let vault = self.roots.open_vault()?;
        let operation_id = Uuid::new_v4().to_string();
        let mut committed = Vec::new();
        for (preview, _) in &previews {
            if preview.action == RouteActionDto::Skip {
                continue;
            }
            let src = SourceFile::open_in(&VerifiedDestination::in_vault(
                &vault,
                PersistentPath::new(&preview.source_relative_path)?,
            )?)?;
            let expected = hash_source(&src)?;
            let copy = atomic_copy_committed(
                &src,
                &VerifiedDestination::in_vault(&vault, PersistentPath::new(&preview.target_path)?)?,
                Some(&expected),
            )?;
            committed.push(copy);
        }
        let mut note_writes = match self.sync_notes(&vault, &previews) {
            Ok(value) => value,
            Err(error) => {
                for copy in committed {
                    let _ = copy.compensate();
                }
                return Err(error);
            }
        };
        let manifest = UndoManifest {
            operation_id: operation_id.clone(),
            created_files: committed
                .iter()
                .map(|c| CreatedFileUndoEntry {
                    path: c.receipt.destination.clone(),
                    expected_sha256: c.receipt.sha256.clone(),
                })
                .collect(),
            backups: Vec::new(),
        };
        let manifest_json =
            serde_json::to_string(&manifest).map_err(|e| BackendError::database(e.to_string()))?;
        let paths: Vec<String> = previews
            .iter()
            .map(|(p, _)| p.target_path.clone())
            .collect();
        let paths_json =
            serde_json::to_string(&paths).map_err(|e| BackendError::database(e.to_string()))?;
        let db_result=self.database.with_transaction(|tx|{
            writes::insert_running_operation(tx,&operation_id,OperationKind::Route,"Раскладка файлов")?;
            for (preview,dest) in &previews { let row=route_row_in(tx,&preview.file_id,dest)?; ensure_week_and_homework(tx,&row,dest)?; if preview.action!=RouteActionDto::Skip { let hash=committed.iter().find(|c|c.receipt.destination.as_str()==preview.target_path).map(|c|c.receipt.sha256.as_str()).ok_or_else(||BackendError::database("Нет квитанции копирования"))?; tx.execute("INSERT INTO files(id,week_id,homework_id,category,name,kind,size_bytes,modified_at,relative_path,content_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?8,?9)",params![Uuid::new_v4().to_string(),row.week_id,row.homework_id,category_db(dest.category),preview.target_name,row.kind,row.size,preview.target_path,hash])?; } tx.execute("INSERT INTO route_operation_items(operation_id,inbox_file_id,target_path,action) VALUES(?1,?2,?3,?4)",params![operation_id,preview.file_id,preview.target_path,action_db(preview.action)])?; tx.execute("UPDATE inbox_files SET status='routed',updated_at=CURRENT_TIMESTAMP WHERE id=?1",[&preview.file_id])?; }
            for note in &note_writes {
                register_managed_note(tx, note)?;
            }
            tx.execute("UPDATE operations SET undoable=1,undo_payload_json=?2,paths_json=?3,detail=?4 WHERE id=?1",params![operation_id,manifest_json,paths_json,format!("Разложено файлов: {}",previews.len())])?; writes::complete_operation(tx,&operation_id)?; Ok(())
        });
        if let Err(error) = db_result {
            while let Some(write) = note_writes.pop() {
                let _ = write.compensate();
            }
            for copy in committed {
                let _ = copy.compensate();
            }
            return Err(error);
        }
        let repository = ReadRepository::new(self.database);
        Ok(OperationResultDto {
            operation: repository.operation(&operation_id)?,
            value: previews.into_iter().map(|x| x.0).collect(),
        })
    }

    pub fn undo(&self, operation_id: &str) -> Result<OperationResultDto<()>, AppError> {
        let (undoable,status,payload,submitted):(bool,String,String,bool)=self.database.with_connection(|c|c.query_row("SELECT undoable,status,coalesce(undo_payload_json,''),EXISTS(SELECT 1 FROM homeworks h WHERE h.id=operations.homework_id AND h.stage='submitted') FROM operations WHERE id=?1",[operation_id],|r|Ok((r.get::<_,i64>(0)?==1,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(Into::into))?;
        let manifest: UndoManifest = serde_json::from_str(&payload).map_err(|_| {
            BackendError::new(
                crate::error::ErrorCode::NotUndoable,
                "Undo manifest повреждён",
            )
        })?;
        let vault = self.roots.open_vault()?;
        let validated = validate_created_undo(
            &manifest,
            UndoContext {
                undoable,
                already_undone: status == "undone",
                submitted_homework: submitted,
            },
            &vault,
        )?;
        let staged = stage_created_files(&validated, &vault)?;
        let undo_id = Uuid::new_v4().to_string();
        let db=self.database.with_transaction(|tx|{writes::insert_running_operation(tx,&undo_id,OperationKind::Undo,"Отмена файловой операции")?; for path in manifest.created_files.iter().map(|x|x.path.as_str()){tx.execute("DELETE FROM files WHERE relative_path=?1",[path])?;} tx.execute("UPDATE operations SET status='undone',undoable=0,completed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?1 AND status='completed'",[operation_id])?; for id in ids_from_manifest(tx,operation_id)? {tx.execute("UPDATE inbox_files SET status='classified' WHERE id=?1",[id])?;} writes::complete_operation(tx,&undo_id)?;Ok(())});
        if let Err(error) = db {
            drop(staged);
            return Err(error);
        }
        for file in staged {
            file.commit()?;
        }
        Ok(OperationResultDto {
            operation: ReadRepository::new(self.database).operation(&undo_id)?,
            value: (),
        })
    }

    fn sync_notes(
        &self,
        vault: &crate::file_service::VaultRoot,
        previews: &[(RoutePreviewDto, RouteDestinationDto)],
    ) -> Result<Vec<ManagedMarkdownWrite>, AppError> {
        let mut writes_out = Vec::new();
        let mut handled = HashSet::new();
        for (preview, dest) in previews {
            let (subject_name, subject_folder): (String, String) =
                self.database.with_connection(|c| {
                    c.query_row(
                        "SELECT name,folder_path FROM subjects WHERE id=?1",
                        [&dest.subject_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(Into::into)
                })?;
            let week_folder = format!("{subject_folder}/Неделя {:02}", dest.week_number);
            let subject_note = format!("{subject_folder}/{subject_name}.md");
            if handled.insert(subject_note.clone()) {
                let managed = format!(
                    "# {subject_name}\n\n## Недели\n- [[Неделя {0:02}/Обзор недели|Неделя {0:02}]]",
                    dest.week_number
                );
                push_note(
                    &mut writes_out,
                    write_managed_markdown(vault, PersistentPath::new(subject_note)?, &managed),
                )?;
            }
            let week_note = format!("{week_folder}/Обзор недели.md");
            if handled.insert(week_note.clone()) {
                let links = self.note_links(&dest.subject_id, dest.week_number, previews)?;
                let managed=format!("# {subject_name} · неделя {0:02}\n\n## Материалы\n{links}\n\n## Домашнее задание\n- [[03 ДЗ/Карточка ДЗ|Карточка ДЗ]]",dest.week_number);
                push_note(
                    &mut writes_out,
                    write_managed_markdown(vault, PersistentPath::new(week_note)?, &managed),
                )?;
            }
            if matches!(
                dest.category,
                MaterialCategory::HomeworkTask
                    | MaterialCategory::HomeworkData
                    | MaterialCategory::HomeworkDraft
            ) {
                let hw_note = format!("{week_folder}/03 ДЗ/Карточка ДЗ.md");
                if handled.insert(hw_note.clone()) {
                    let managed=format!("# Домашнее задание · неделя {0:02}\n\n## Файлы\n- [[../../{1}|{2}]]\n\n## Версии\n\n## Проверка\nСтатус: материалы собраны\n\n## Комментарии преподавателя\n\n## Итоговый комплект\n\n## Отправленный снимок",dest.week_number,preview.target_path.strip_prefix(&format!("{week_folder}/")).unwrap_or(&preview.target_path),preview.target_name);
                    push_note(
                        &mut writes_out,
                        write_managed_markdown(vault, PersistentPath::new(hw_note)?, &managed),
                    )?;
                }
            }
        }
        Ok(writes_out)
    }
    fn note_links(
        &self,
        subject_id: &str,
        week: i64,
        previews: &[(RoutePreviewDto, RouteDestinationDto)],
    ) -> Result<String, AppError> {
        let mut links=self.database.with_connection(|c|{let mut q=c.prepare("SELECT f.relative_path,f.name FROM files f JOIN weeks w ON w.id=f.week_id WHERE w.subject_id=?1 AND w.number=?2 ORDER BY f.category,f.name")?;let rows=q.query_map(params![subject_id,week],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?;Ok(rows.collect::<Result<Vec<_>,_>>()?)})?;
        for (p, d) in previews {
            if d.subject_id == subject_id && d.week_number == week {
                links.push((p.target_path.clone(), p.target_name.clone()));
            }
        }
        links.sort();
        links.dedup();
        Ok(links
            .into_iter()
            .map(|(p, n)| format!("- [[{p}|{n}]]"))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn register_managed_note(
    transaction: &rusqlite::Transaction<'_>,
    note: &ManagedMarkdownWrite,
) -> Result<(), AppError> {
    let relative_path = note.relative_path().as_str();
    let (parent, file_name) = relative_path
        .rsplit_once('/')
        .ok_or_else(|| BackendError::database("Неверный путь управляемой заметки"))?;
    let (entity_kind, entity_id): (&str, String) = match file_name {
        "Карточка ДЗ.md" => (
            "homework",
            transaction.query_row(
                "SELECT id FROM homeworks WHERE folder_path=?1",
                [parent],
                |row| row.get(0),
            )?,
        ),
        "Обзор недели.md" => (
            "week",
            transaction.query_row(
                "SELECT id FROM weeks WHERE folder_path=?1",
                [parent],
                |row| row.get(0),
            )?,
        ),
        _ => (
            "subject",
            transaction.query_row(
                "SELECT id FROM subjects WHERE folder_path=?1",
                [parent],
                |row| row.get(0),
            )?,
        ),
    };
    transaction.execute(
        "INSERT INTO managed_notes(id,entity_kind,entity_id,relative_path,managed_hash)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(entity_kind,entity_id) DO UPDATE SET
           relative_path=excluded.relative_path,
           managed_hash=excluded.managed_hash,
           updated_at=CURRENT_TIMESTAMP",
        params![
            Uuid::new_v4().to_string(),
            entity_kind,
            entity_id,
            relative_path,
            note.managed_hash()
        ],
    )?;
    Ok(())
}

struct RouteRow {
    source_path: String,
    name: String,
    kind: String,
    size: i64,
    folder: String,
    week_folder: String,
    week_exists: bool,
    week_id: String,
    homework_id: Option<String>,
}
fn route_row(db: &Database, id: &str, d: &RouteDestinationDto) -> Result<RouteRow, AppError> {
    db.with_connection(|c| route_row_in(c, id, d))
}
fn route_row_in(
    c: &rusqlite::Connection,
    id: &str,
    d: &RouteDestinationDto,
) -> Result<RouteRow, AppError> {
    let (source_path,name,kind,size):(String,String,String,i64)=c.query_row("SELECT relative_path,name,kind,size_bytes FROM inbox_files WHERE id=?1 AND status<>'routed'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|match e{rusqlite::Error::QueryReturnedNoRows=>BackendError::not_found("Файл во «Входящих» не найден"),o=>o.into()})?;
    let subject_folder: String = c
        .query_row(
            "SELECT folder_path FROM subjects WHERE id=?1",
            [&d.subject_id],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => BackendError::not_found("Предмет не найден"),
            o => o.into(),
        })?;
    let week_folder = format!("{subject_folder}/Неделя {:02}", d.week_number);
    let found:Option<(String,Option<String>)>=c.query_row("SELECT w.id,h.id FROM weeks w LEFT JOIN homeworks h ON h.week_id=w.id WHERE w.subject_id=?1 AND w.number=?2",params![d.subject_id,d.week_number],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let week_exists = found.is_some();
    let (week_id, homework_id) = found.unwrap_or_else(|| (Uuid::new_v4().to_string(), None));
    let folder = format!("{week_folder}/{}", category_folder(d.category));
    Ok(RouteRow {
        source_path,
        name,
        kind,
        size,
        folder,
        week_folder,
        week_exists,
        week_id,
        homework_id,
    })
}
fn ensure_week_and_homework(
    c: &rusqlite::Connection,
    row: &RouteRow,
    d: &RouteDestinationDto,
) -> Result<(), AppError> {
    if !row.week_exists {
        c.execute("INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES(?1,?2,?3,strftime('%Y-%m-%dT00:00:00Z','now'),strftime('%Y-%m-%dT23:59:59Z','now','+6 days'),?4)",params![row.week_id,d.subject_id,d.week_number,row.week_folder])?;
    }
    if matches!(
        d.category,
        MaterialCategory::HomeworkTask
            | MaterialCategory::HomeworkData
            | MaterialCategory::HomeworkDraft
    ) && row.homework_id.is_none()
    {
        c.execute("INSERT INTO homeworks(id,week_id,title,deadline,stage,workflow_stage,folder_path) VALUES(?1,?2,?3,strftime('%Y-%m-%dT23:59:59Z','now','+7 days'),'task','materials',?4)",params![Uuid::new_v4().to_string(),row.week_id,format!("Домашнее задание — неделя {}",d.week_number),format!("{}/03 ДЗ",row.week_folder)])?;
    }
    Ok(())
}
fn category_folder(c: MaterialCategory) -> &'static str {
    match c {
        MaterialCategory::Lecture => "01 Лекции",
        MaterialCategory::Seminar => "02 Семинары",
        MaterialCategory::HomeworkTask | MaterialCategory::HomeworkData => {
            "03 ДЗ/01 Условие и данные"
        }
        MaterialCategory::HomeworkDraft => "03 ДЗ/02 Работа",
        MaterialCategory::Material | MaterialCategory::Unknown => "04 Дополнительно",
    }
}
fn category_db(c: MaterialCategory) -> &'static str {
    match c {
        MaterialCategory::Lecture => "lecture",
        MaterialCategory::Seminar => "seminar",
        MaterialCategory::Material => "material",
        MaterialCategory::HomeworkTask => "homework_task",
        MaterialCategory::HomeworkData => "homework_data",
        MaterialCategory::HomeworkDraft => "homework_draft",
        MaterialCategory::Unknown => "material",
    }
}
fn classification_destination(db: &Database, id: &str) -> Result<RouteDestinationDto, AppError> {
    let file = ReadRepository::new(db).inbox_file(id)?;
    let c = file.classification.ok_or_else(|| {
        BackendError::new(
            crate::error::ErrorCode::ConfirmationRequired,
            "Нужно уточнить предмет, неделю и тип материала",
        )
    })?;
    if file.status == InboxStatus::Ambiguous
        || c.subject_id.is_none()
        || c.week_number.is_none()
        || c.category == MaterialCategory::Unknown
    {
        return Err(BackendError::new(
            crate::error::ErrorCode::ConfirmationRequired,
            "Неоднозначный файл нельзя разложить без ручного подтверждения",
        ));
    }
    Ok(RouteDestinationDto {
        subject_id: c.subject_id.unwrap(),
        week_number: c.week_number.unwrap(),
        category: c.category,
    })
}
fn classify_name(name: &str, subjects: &[(String, String, String)]) -> ClassificationDto {
    let lower = name.to_lowercase();
    let mut reasons = Vec::new();
    let subject = subjects.iter().find(|(_, name, short)| {
        lower.contains(&name.to_lowercase()) || lower.contains(&short.to_lowercase())
    });
    if subject.is_some() {
        reasons.push("Название содержит предмет".into());
    }
    let week = (1..=60).find(|n| {
        lower.contains(&format!("неделя {n}"))
            || lower.contains(&format!("week{n}"))
            || lower.contains(&format!("week {n}"))
            || lower.contains(&format!("w{n:02}"))
    });
    if week.is_some() {
        reasons.push("Номер недели найден в имени".into());
    }
    let category = if lower.contains("лекц") || lower.contains("lecture") {
        MaterialCategory::Lecture
    } else if lower.contains("семинар") || lower.contains("seminar") {
        MaterialCategory::Seminar
    } else if lower.contains("услов") || lower.contains("задан") || lower.contains("task")
    {
        MaterialCategory::HomeworkTask
    } else if lower.contains("решен") || lower.contains("solution") || lower.contains("draft")
    {
        MaterialCategory::HomeworkDraft
    } else if lower.contains("данн") || lower.contains("data") {
        MaterialCategory::HomeworkData
    } else {
        MaterialCategory::Unknown
    };
    if category != MaterialCategory::Unknown {
        reasons.push("Тип материала найден в имени".into());
    }
    let confidence = (if subject.is_some() { 0.4 } else { 0.0 })
        + (if week.is_some() { 0.3 } else { 0.0 })
        + (if category != MaterialCategory::Unknown {
            0.3
        } else {
            0.0
        });
    ClassificationDto {
        subject_id: subject.map(|x| x.0.clone()),
        week_number: week,
        category,
        confidence,
        reasons,
        source: ClassificationSource::Auto,
    }
}
fn ids_from_manifest(
    c: &rusqlite::Connection,
    operation_id: &str,
) -> Result<Vec<String>, AppError> {
    let mut q =
        c.prepare("SELECT inbox_file_id FROM route_operation_items WHERE operation_id=?1")?;
    let rows = q.query_map([operation_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
fn action_db(action: RouteActionDto) -> &'static str {
    match action {
        RouteActionDto::Copy => "copy",
        RouteActionDto::Skip => "skip",
        RouteActionDto::Rename => "rename",
    }
}
fn push_note(
    writes: &mut Vec<ManagedMarkdownWrite>,
    result: Result<ManagedMarkdownWrite, AppError>,
) -> Result<(), AppError> {
    match result {
        Ok(write) => {
            writes.push(write);
            Ok(())
        }
        Err(error) => {
            while let Some(write) = writes.pop() {
                let _ = write.compensate();
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RoutingService;
    use crate::{
        db::Database,
        dto::{MaterialCategory, RouteActionDto, RouteDestinationDto},
        intake::{RootKind, RootsService},
    };
    use rusqlite::params;
    use std::fs;
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        vault: std::path::PathBuf,
        database: Database,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let vault = temp.path().join("vault");
            let backup = temp.path().join("backup");
            fs::create_dir_all(&vault).unwrap();
            fs::create_dir_all(&backup).unwrap();
            let database = Database::open_in_memory().unwrap();
            database.migrate().unwrap();
            let roots = RootsService::new(&database, None);
            roots.set_root(RootKind::Vault, &vault).unwrap();
            roots.set_root(RootKind::Backup, &backup).unwrap();
            roots.prepare_inbox().unwrap();
            database.with_connection(|c|{c.execute("INSERT INTO subjects(id,name,short_name,folder_path) VALUES('math','Математика','мат','Предметы/Математика')",[])?;Ok(())}).unwrap();
            Self {
                _temp: temp,
                vault,
                database,
            }
        }
        fn add(&self, id: &str, name: &str, classification: Option<&str>) {
            let relative = format!("Предметы/00 Входящие/{name}");
            fs::write(self.vault.join(&relative), b"course data").unwrap();
            self.database.with_connection(|c|{c.execute("INSERT INTO inbox_files(id,name,kind,size_bytes,added_at,origin,status,relative_path,classification_json,sha256,mime_type,provenance_json) VALUES(?1,?2,'text',11,'2026-10-26T10:00:00Z','drop','new',?3,?4,NULL,'text/plain','{\"source\":\"manual\",\"via\":\"drop\",\"archiveName\":null,\"entryPath\":null,\"courseUrl\":null,\"assignmentUrl\":null}')",params![id,name,relative,classification])?;Ok(())}).unwrap();
        }
    }

    #[test]
    fn preview_route_and_undo_are_real_and_notes_preserve_user_text() {
        let f = Fixture::new();
        f.add("i1", "Математика week 7 лекция.md", None);
        let service = RoutingService::new(&f.database, None);
        let classified = service.classify_files(&["i1".into()]).unwrap();
        assert_eq!(
            classified.value[0]
                .classification
                .as_ref()
                .unwrap()
                .category,
            MaterialCategory::Lecture
        );
        let dest = RouteDestinationDto {
            subject_id: "math".into(),
            week_number: 7,
            category: MaterialCategory::Lecture,
        };
        let preview = service.preview("i1", &dest).unwrap();
        assert_eq!(preview.action, RouteActionDto::Copy);
        assert!(!f.vault.join(&preview.target_path).exists());
        let wire = serde_json::to_value(&preview).unwrap();
        assert_eq!(wire["sourceName"], "Математика week 7 лекция.md");
        assert_eq!(wire["action"], "copy");
        assert!(wire.get("sourceRelativePath").is_some());
        assert!(wire.get("createsDirectories").is_some());
        assert!(wire.get("source_relative_path").is_none());
        let routed = service.route(&["i1".into()], Some(&dest)).unwrap();
        assert!(f.vault.join(&routed.value[0].target_path).is_file());
        let registered_notes: i64 = f
            .database
            .with_connection(|c| {
                Ok(c.query_row("SELECT count(*) FROM managed_notes", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(registered_notes, 2);
        let note = f
            .vault
            .join("Предметы/Математика/Неделя 07/Обзор недели.md");
        let original = fs::read_to_string(&note).unwrap();
        fs::write(&note, format!("Мой текст\n{original}")).unwrap();
        let second_name = "Математика week 7 семинар.md";
        f.add("i2", second_name, None);
        let second = RouteDestinationDto {
            category: MaterialCategory::Seminar,
            ..dest.clone()
        };
        let second_route = service.route(&["i2".into()], Some(&second)).unwrap();
        assert!(fs::read_to_string(&note).unwrap().starts_with("Мой текст"));
        f.add("i3", "Математика week 7 ДЗ.md", None);
        let homework = RouteDestinationDto {
            category: MaterialCategory::HomeworkTask,
            ..dest
        };
        service.route(&["i3".into()], Some(&homework)).unwrap();
        let homework_note: String = f
            .database
            .with_connection(|c| {
                Ok(c.query_row(
                    "SELECT relative_path FROM managed_notes WHERE entity_kind='homework'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(
            homework_note,
            "Предметы/Математика/Неделя 07/03 ДЗ/Карточка ДЗ.md"
        );
        service.undo(&second_route.operation.id).unwrap();
        assert!(!f.vault.join(&second_route.value[0].target_path).exists());
        let status: String = f
            .database
            .with_connection(|c| {
                Ok(
                    c.query_row("SELECT status FROM inbox_files WHERE id='i2'", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(status, "classified");
    }

    #[test]
    fn manual_classification_has_priority_and_ambiguous_is_not_routed() {
        let f = Fixture::new();
        let manual = r#"{"subjectId":"math","weekNumber":3,"category":"material","confidence":1.0,"reasons":["Вручную"],"source":"manual"}"#;
        f.add("i1", "scan.pdf", Some(manual));
        let service = RoutingService::new(&f.database, None);
        let result = service.classify_files(&["i1".into()]).unwrap();
        assert_eq!(
            result.value[0].classification.as_ref().unwrap().week_number,
            Some(3)
        );
        f.add("i2", "scan2.pdf", None);
        service.classify_files(&["i2".into()]).unwrap();
        let error = service.route(&["i2".into()], None).unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::ConfirmationRequired);
    }
}
