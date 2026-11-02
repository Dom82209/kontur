use std::collections::HashSet;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    db::Database,
    dto::{FileKind, FileRefDto, OperationKind, OperationResultDto},
    error::{AppError, BackendError, ErrorCode},
    file_service::{
        atomic_copy_committed, hash_source, write_managed_markdown, CommittedCopy, PersistentPath,
        SourceFile, VerifiedDestination,
    },
    intake::RootsService,
    repositories::{writes, ReadRepository},
    workflow::{
        dto::{
            PackageCheckDto, PackageCheckId, PackageItemDto, PackageItemRole, PackageLinkDto,
            StyleProfileDto, SubmissionConfirmationInput, SubmissionPackageDto,
            SubmissionReceiptDto,
        },
        repository,
    },
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestEntry {
    path: String,
    sha256: String,
    size_bytes: u64,
}

pub struct SubmissionService<'a> {
    database: &'a Database,
    roots: RootsService<'a>,
}
impl<'a> SubmissionService<'a> {
    pub fn new(database: &'a Database, home: Option<std::path::PathBuf>) -> Self {
        Self {
            database,
            roots: RootsService::new(database, home),
        }
    }

    /// Итоговое имя по шаблону из настроек (этап 6): фамилия, имя, неделя,
    /// предмет и метка итоговой версии.
    fn final_name(
        &self,
        homework_id: &str,
        version_id: &str,
        source_name: &str,
    ) -> Result<String, AppError> {
        let rules = crate::repositories::ReadRepository::new(self.database).automation_rules()?;
        let (week, subject, version): (i64, String, String) =
            self.database.with_connection(|c| {
                c.query_row(
                    "SELECT w.number, s.name, v.label FROM homeworks h
                       JOIN weeks w ON w.id = h.week_id
                       JOIN subjects s ON s.id = w.subject_id
                       JOIN homework_versions v ON v.id = ?2 AND v.homework_id = h.id
                      WHERE h.id = ?1",
                    [homework_id, version_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(Into::into)
            })?;
        let extension = std::path::Path::new(source_name)
            .extension()
            .and_then(|x| x.to_str());
        crate::naming::final_file_name(
            &rules.final_name_pattern,
            &crate::naming::NameTokens {
                last_name: &rules.student_name.last_name,
                first_name: &rules.student_name.first_name,
                week,
                subject: &subject,
                version: &version,
            },
            extension,
        )
    }

    pub fn style_profile(&self) -> Result<Option<StyleProfileDto>, AppError> {
        self.database.with_connection(|c|{
  let row:Option<(String,String,String,String)>=c.query_row("SELECT id,relative_path,traits_json,created_at FROM style_profile_sources WHERE explicitly_selected=1 ORDER BY created_at DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
  row.map(|(id,path,traits,updated)|{let file_name=std::path::Path::new(&path).file_name().and_then(|x|x.to_str()).unwrap_or("Профиль стиля").to_owned();let traits=serde_json::from_str(&traits).map_err(|e|BackendError::database(e.to_string()))?;Ok(StyleProfileDto{id,file_name,relative_path:path,updated_at:updated,traits})}).transpose()
 })
    }

    pub fn remove_teacher_comment(&self, id: &str) -> Result<(), AppError> {
        self.database.with_transaction(|tx| {
            let homework: String = tx
                .query_row(
                    "SELECT homework_id FROM teacher_comments WHERE id=?1",
                    [id],
                    |r| r.get(0),
                )
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        BackendError::not_found("Комментарий не найден")
                    }
                    o => o.into(),
                })?;
            let submitted: bool = tx.query_row(
                "SELECT stage='submitted' FROM homeworks WHERE id=?1",
                [&homework],
                |r| r.get(0),
            )?;
            if submitted {
                return Err(BackendError::immutable("Отправленная работа неизменяема"));
            }
            // Этап 7: требования удалённого ответа отзываются и остаются в истории.
            crate::workflow::repository::withdraw_comment_requirements(tx, id)?;
            tx.execute("DELETE FROM teacher_comments WHERE id=?1", [id])?;
            let (stage, ready): (String, Option<String>) = tx.query_row(
                "SELECT workflow_stage, ready_fingerprint FROM homeworks WHERE id=?1",
                [&homework],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if stage == "ready_to_submit"
                && !crate::workflow::fingerprint::current(tx, &homework)?
                    .is_current(ready.as_deref())
            {
                crate::workflow::repository::set_workflow_stage(
                    tx,
                    &homework,
                    crate::domain::HomeworkWorkflowStage::TeacherReview,
                )?;
            }
            tx.execute(
                "UPDATE homeworks SET revision=revision+1,updated_at=CURRENT_TIMESTAMP WHERE id=?1",
                [homework],
            )?;
            Ok(())
        })
    }

    pub fn build(
        &self,
        homework_id: &str,
    ) -> Result<OperationResultDto<SubmissionPackageDto>, AppError> {
        let submitted: bool = self.database.with_connection(|c| {
            c.query_row(
                "SELECT stage='submitted' FROM homeworks WHERE id=?1",
                [homework_id],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => BackendError::not_found("ДЗ не найдено"),
                o => o.into(),
            })
        })?;
        if submitted {
            return Err(BackendError::immutable(
                "Отправленная работа и её снимок неизменяемы",
            ));
        }
        if let Some(existing) = self
            .database
            .with_connection(|c| repository::submission_package(c, homework_id))?
        {
            let op_id = Uuid::new_v4().to_string();
            self.database.with_transaction(|tx| {
                writes::insert_running_operation(
                    tx,
                    &op_id,
                    OperationKind::BuildPackage,
                    "Комплект уже собран",
                )?;
                writes::complete_operation(tx, &op_id)?;
                Ok(())
            })?;
            return Ok(OperationResultDto {
                operation: ReadRepository::new(self.database).operation(&op_id)?,
                value: existing,
            });
        }
        let (version_id,source_path,source_name,kind,size,folder,assignment_url,final_file_name,files_json)=self.database.with_connection(|c|c.query_row("SELECT v.id,v.file_path,v.file_name,coalesce(f.kind,'other'),v.size_bytes,h.folder_path,h.assignment_url,h.final_file_name,v.files_json FROM homeworks h JOIN homework_versions v ON v.id=h.final_version_id LEFT JOIN files f ON f.id=v.source_file_id WHERE h.id=?1 AND h.stage='ready_to_submit'",[homework_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,String>(8)?))).map_err(|e|match e{rusqlite::Error::QueryReturnedNoRows=>BackendError::blocked("Сначала выберите итоговую версию и пройдите проверку готовности"),o=>o.into()}))?;
        let version_files: Vec<FileRefDto> = serde_json::from_str(&files_json)
            .map_err(|error| BackendError::database(format!("Повреждён состав версии: {error}")))?;
        let teacher_requirements_done: bool = self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT NOT EXISTS(SELECT 1 FROM teacher_requirements WHERE homework_id=?1 AND status='not_done')",
                    [homework_id],
                    |row| row.get(0),
                )
                .map_err(Into::into)
        })?;
        let package_folder = format!("{folder}/03 На сдачу");
        let vault = self.roots.open_vault()?;
        let src = SourceFile::open_in(&VerifiedDestination::in_vault(
            &vault,
            PersistentPath::new(&source_path)?,
        )?)?;
        let expected = hash_source(&src)?;
        let safe_name = match final_file_name {
            Some(name) if safe_package_name(&name) => name,
            Some(_) => return Err(BackendError::blocked("Итоговое имя файла небезопасно")),
            None => self.final_name(homework_id, &version_id, &source_name)?,
        };
        let target = PersistentPath::new(format!("{package_folder}/{safe_name}"))?;
        let copy = atomic_copy_committed(
            &src,
            &VerifiedDestination::in_vault(&vault, target.clone())?,
            Some(&expected),
        )?;
        let file = FileRefDto {
            id: Uuid::new_v4().to_string(),
            name: safe_name.clone(),
            kind: FileKind::try_from_db(&kind).unwrap_or(FileKind::Other),
            size_bytes: size,
            modified_at: now(),
            relative_path: target.as_str().to_owned(),
        };
        let mut items = vec![PackageItemDto {
            id: Uuid::new_v4().to_string(),
            role: PackageItemRole::Main,
            file: Some(file.clone()),
            final_name: safe_name.clone(),
            required: true,
            present: true,
        }];
        let links = assignment_url
            .filter(|u| !u.trim().is_empty())
            .map(|url| {
                vec![PackageLinkDto {
                    id: Uuid::new_v4().to_string(),
                    label: "Задание в системе вуза".into(),
                    url,
                    required: false,
                    verified: true,
                }]
            })
            .unwrap_or_default();
        let checklist = vec![
            check(
                PackageCheckId::FinalSelected,
                "Итоговая версия выбрана",
                true,
                true,
            ),
            check(
                PackageCheckId::NameChecked,
                "Имя файла проверено",
                true,
                true,
            ),
            check(
                PackageCheckId::Attachments,
                "Обязательные приложения на месте",
                true,
                true,
            ),
            check(
                PackageCheckId::Links,
                "Внешние ссылки проверены",
                links.iter().all(|x| !x.required || x.verified),
                true,
            ),
            check(
                PackageCheckId::Teacher,
                "Требования преподавателя учтены",
                teacher_requirements_done,
                true,
            ),
            check(
                PackageCheckId::ManualReview,
                "Я вручную проверил комплект",
                false,
                false,
            ),
        ];
        let mut manifest = vec![ManifestEntry {
            path: target.as_str().to_owned(),
            sha256: copy.receipt.sha256.as_str().to_owned(),
            size_bytes: copy.receipt.size_bytes,
        }];
        let mut committed = vec![copy];
        let mut upload_list = vec![safe_name.clone()];
        let mut names = HashSet::from([safe_name.clone()]);
        for attachment in version_files
            .into_iter()
            .filter(|file| file.relative_path != source_path)
        {
            if !safe_package_name(&attachment.name) {
                compensate_all(committed);
                return Err(BackendError::blocked(
                    "Имя приложения небезопасно для комплекта",
                ));
            }
            if !names.insert(attachment.name.clone()) {
                compensate_all(committed);
                return Err(BackendError::conflict(
                    "В версии есть несколько файлов с одинаковым итоговым именем",
                ));
            }
            let copied = (|| {
                let source = SourceFile::open_in(&VerifiedDestination::in_vault(
                    &vault,
                    PersistentPath::new(&attachment.relative_path)?,
                )?)?;
                let expected = hash_source(&source)?;
                let target = PersistentPath::new(format!("{package_folder}/{}", attachment.name))?;
                let copy = atomic_copy_committed(
                    &source,
                    &VerifiedDestination::in_vault(&vault, target.clone())?,
                    Some(&expected),
                )?;
                Ok::<_, AppError>((target, copy))
            })();
            let (attachment_target, attachment_copy) = match copied {
                Ok(value) => value,
                Err(error) => {
                    compensate_all(committed);
                    return Err(error);
                }
            };
            let packaged = FileRefDto {
                relative_path: attachment_target.as_str().to_owned(),
                modified_at: now(),
                ..attachment
            };
            manifest.push(ManifestEntry {
                path: attachment_target.as_str().to_owned(),
                sha256: attachment_copy.receipt.sha256.as_str().to_owned(),
                size_bytes: attachment_copy.receipt.size_bytes,
            });
            upload_list.push(packaged.name.clone());
            items.push(PackageItemDto {
                id: Uuid::new_v4().to_string(),
                role: PackageItemRole::Attachment,
                final_name: packaged.name.clone(),
                file: Some(packaged),
                required: true,
                present: true,
            });
            committed.push(attachment_copy);
        }
        let id = Uuid::new_v4().to_string();
        let op_id = Uuid::new_v4().to_string();
        let note_path = PersistentPath::new(format!("{folder}/Карточка ДЗ.md"))?;
        let note_content = self.homework_note(homework_id, Some(&package_folder), None)?;
        let note = match write_managed_markdown(&vault, note_path, &note_content) {
            Ok(v) => v,
            Err(e) => {
                compensate_all(committed);
                return Err(e);
            }
        };
        let package_paths = manifest
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();
        let db=self.database.with_transaction(|tx|{writes::insert_running_operation(tx,&op_id,OperationKind::BuildPackage,"Комплект на сдачу собран")?;tx.execute("INSERT INTO submission_packages(id,homework_id,final_version_id,status,package_path,checklist_json,items_json,links_json,warnings_json,upload_list_json,built_at,manifest_json) VALUES(?1,?2,?3,'draft',?4,?5,?6,?7,'[]',?8,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?9)",params![id,homework_id,version_id,package_folder,json(&checklist)?,json(&items)?,json(&links)?,json(&upload_list)?,json(&manifest)?])?;tx.execute("UPDATE operations SET paths_json=?2,undoable=0,homework_id=?3 WHERE id=?1",params![op_id,json(&package_paths)?,homework_id])?;writes::complete_operation(tx,&op_id)?;Ok(())});
        if let Err(e) = db {
            let _ = note.compensate();
            compensate_all(committed);
            return Err(e);
        }
        let value = self
            .database
            .with_connection(|c| repository::submission_package(c, homework_id))?
            .ok_or_else(|| BackendError::database("Комплект не найден после сборки"))?;
        Ok(OperationResultDto {
            operation: ReadRepository::new(self.database).operation(&op_id)?,
            value,
        })
    }

    pub fn set_check(
        &self,
        package_id: &str,
        check_id: PackageCheckId,
        done: bool,
    ) -> Result<SubmissionPackageDto, AppError> {
        self.database.with_transaction(|tx|{let (homework,raw):(String,String)=tx.query_row("SELECT homework_id,checklist_json FROM submission_packages WHERE id=?1 AND status<>'submitted'",[package_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|match e{rusqlite::Error::QueryReturnedNoRows=>BackendError::immutable("Комплект отправленной работы неизменяем"),o=>o.into()})?;let mut checks:Vec<PackageCheckDto>=serde_json::from_str(&raw).map_err(|e|BackendError::database(e.to_string()))?;let item=checks.iter_mut().find(|x|x.id==check_id).ok_or_else(||BackendError::not_found("Пункт checklist не найден"))?;if item.auto{return Err(BackendError::blocked("Автоматический пункт нельзя изменить вручную"));}item.done=done;let ready=checks.iter().all(|x|x.done);tx.execute("UPDATE submission_packages SET checklist_json=?2,status=?3,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![package_id,json(&checks)?,if ready{"ready"}else{"draft"}])?;repository::submission_package(tx,&homework)?.ok_or_else(||BackendError::database("Комплект исчез"))})
    }

    pub fn confirm(
        &self,
        homework_id: &str,
        input: &SubmissionConfirmationInput,
        proof_source: Option<(SourceFile, String)>,
    ) -> Result<OperationResultDto<SubmissionReceiptDto>, AppError> {
        if !input.user_confirmed {
            return Err(BackendError::new(
                ErrorCode::ConfirmationRequired,
                "Подтвердите, что отправили работу вручную",
            ));
        }
        if !input.assignment_url.starts_with("https://") {
            return Err(BackendError::blocked(
                "Ссылка на задание должна начинаться с https://",
            ));
        }
        if input.proof.is_some() != proof_source.is_some() {
            return Err(BackendError::blocked(
                "Файл подтверждения должен быть выбран системным диалогом",
            ));
        }
        let package = self
            .database
            .with_connection(|c| repository::submission_package(c, homework_id))?
            .ok_or_else(|| BackendError::blocked("Сначала соберите комплект"))?;
        if package.checklist.iter().any(|x| !x.done) {
            return Err(BackendError::blocked("Не все пункты checklist выполнены"));
        }
        let vault = self.roots.open_vault()?;
        let snapshot_folder = format!(
            "{}/04 Отправлено/{}",
            homework_folder(self.database, homework_id)?,
            Uuid::new_v4()
        );
        let mut committed = Vec::new();
        let mut files = Vec::new();
        let mut manifest = Vec::new();
        for item in &package.items {
            if !item.present {
                if item.required {
                    return Err(BackendError::blocked("Обязательный файл отсутствует"));
                }
                continue;
            }
            let source = item
                .file
                .as_ref()
                .ok_or_else(|| BackendError::blocked("У файла комплекта нет ссылки"))?;
            let src = SourceFile::open_in(&VerifiedDestination::in_vault(
                &vault,
                PersistentPath::new(&source.relative_path)?,
            )?)?;
            let expected = hash_source(&src)?;
            let target = PersistentPath::new(format!("{snapshot_folder}/{}", item.final_name))?;
            let copy = atomic_copy_committed(
                &src,
                &VerifiedDestination::in_vault(&vault, target.clone())?,
                Some(&expected),
            )?;
            files.push(FileRefDto {
                relative_path: target.as_str().to_owned(),
                ..source.clone()
            });
            manifest.push(ManifestEntry {
                path: target.as_str().to_owned(),
                sha256: copy.receipt.sha256.as_str().to_owned(),
                size_bytes: copy.receipt.size_bytes,
            });
            committed.push(copy);
        }
        let proof = if let Some((source, name)) = proof_source {
            if !safe_package_name(&name) {
                compensate_all(committed);
                return Err(BackendError::blocked("Имя файла подтверждения небезопасно"));
            }
            let copied = (|| {
                let expected = hash_source(&source)?;
                let target =
                    PersistentPath::new(format!("{snapshot_folder}/Подтверждение_{name}"))?;
                let copy = atomic_copy_committed(
                    &source,
                    &VerifiedDestination::in_vault(&vault, target.clone())?,
                    Some(&expected),
                )?;
                let size_bytes = match i64::try_from(copy.receipt.size_bytes) {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = copy.compensate();
                        return Err(BackendError::blocked("Файл подтверждения слишком большой"));
                    }
                };
                Ok::<_, AppError>((target, copy, size_bytes))
            })();
            let (target, copy, size_bytes) = match copied {
                Ok(value) => value,
                Err(error) => {
                    compensate_all(committed);
                    return Err(error);
                }
            };
            let proof = FileRefDto {
                id: Uuid::new_v4().to_string(),
                kind: proof_kind(&name),
                name,
                size_bytes,
                modified_at: now(),
                relative_path: target.as_str().to_owned(),
            };
            manifest.push(ManifestEntry {
                path: target.as_str().to_owned(),
                sha256: copy.receipt.sha256.as_str().to_owned(),
                size_bytes: copy.receipt.size_bytes,
            });
            committed.push(copy);
            Some(proof)
        } else {
            None
        };
        let snapshot_hash = aggregate_hash(&manifest);
        let receipt_id = Uuid::new_v4().to_string();
        let op_id = Uuid::new_v4().to_string();
        let proof_json = match proof.as_ref().map(json).transpose() {
            Ok(value) => value,
            Err(error) => {
                compensate_all(committed);
                return Err(error);
            }
        };
        let folder = homework_folder(self.database, homework_id)?;
        let note_path = PersistentPath::new(format!("{folder}/Карточка ДЗ.md"))?;
        let note_content = self.homework_note(
            homework_id,
            Some(&package.folder_path),
            Some(&snapshot_folder),
        )?;
        let note = match write_managed_markdown(&vault, note_path, &note_content) {
            Ok(v) => v,
            Err(e) => {
                for c in committed {
                    let _ = c.compensate();
                }
                return Err(e);
            }
        };
        let db=self.database.with_transaction(|tx|{writes::insert_running_operation(tx,&op_id,OperationKind::ConfirmSubmission,"Ручная отправка подтверждена")?;tx.execute("INSERT INTO submission_receipts(id,package_id,homework_id,submitted_version_id,snapshot_path,snapshot_hash,confirmed_at,submitted_at,assignment_url,proof_json,files_json,manifest_json) VALUES(?1,?2,?3,?4,?5,?6,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?7,?8,?9,?10,?11)",params![receipt_id,package.id,homework_id,package.version_id,snapshot_folder,snapshot_hash,input.submitted_at,input.assignment_url,proof_json,json(&files)?,json(&manifest)?])?;tx.execute("UPDATE submission_packages SET status='submitted',updated_at=CURRENT_TIMESTAMP WHERE id=?1",[&package.id])?;tx.execute("UPDATE homeworks SET stage='submitted',workflow_stage='submitted',submitted_at=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![homework_id,input.submitted_at])?;tx.execute("UPDATE operations SET homework_id=?2,paths_json=?3 WHERE id=?1",params![op_id,homework_id,json(&vec![snapshot_folder.clone()])?])?;writes::complete_operation(tx,&op_id)?;Ok(())});
        if let Err(e) = db {
            let _ = note.compensate();
            for c in committed {
                let _ = c.compensate();
            }
            return Err(e);
        }
        let value = self
            .database
            .with_connection(|c| repository::submission_receipt(c, homework_id))?
            .ok_or_else(|| BackendError::database("Квитанция не найдена"))?;
        Ok(OperationResultDto {
            operation: ReadRepository::new(self.database).operation(&op_id)?,
            value,
        })
    }
    fn homework_note(
        &self,
        homework_id: &str,
        package: Option<&str>,
        snapshot: Option<&str>,
    ) -> Result<String, AppError> {
        self.database.with_connection(|c|{let (title,week,stage):(String,i64,String)=c.query_row("SELECT h.title,w.number,h.stage FROM homeworks h JOIN weeks w ON w.id=h.week_id WHERE h.id=?1",[homework_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;let mut versions=Vec::new();{let mut q=c.prepare("SELECT label,file_path FROM homework_versions WHERE homework_id=?1 ORDER BY number")?;for row in q.query_map([homework_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?{versions.push(row?);}}let mut comments=Vec::new();{let mut q=c.prepare("SELECT answer FROM teacher_comments WHERE homework_id=?1 ORDER BY created_at")?;for row in q.query_map([homework_id],|r|r.get::<_,String>(0))?{comments.push(row?);}}let version_lines=versions.into_iter().map(|(label,path)|format!("- [[{path}|{label}]]")).collect::<Vec<_>>().join("\n");let comment_lines=comments.into_iter().map(|text|format!("- {}",text.replace('\n'," "))).collect::<Vec<_>>().join("\n");Ok(format!("# {title}\n\nНеделя: {week:02}\n\n## Версии\n{version_lines}\n\n## Проверка\nСтатус: {stage}\n\n## Комментарии преподавателя\n{comment_lines}\n\n## Итоговый комплект\n{}\n\n## Отправленный снимок\n{}",package.map(|p|format!("[[{p}|Открыть комплект]]")).unwrap_or_default(),snapshot.map(|p|format!("[[{p}|Открыть неизменяемый снимок]]")).unwrap_or_default()))})
    }
}
fn check(id: PackageCheckId, label: &str, done: bool, auto: bool) -> PackageCheckDto {
    PackageCheckDto {
        id,
        label: label.into(),
        done,
        auto,
    }
}
fn json<T: Serialize>(v: &T) -> Result<String, AppError> {
    serde_json::to_string(v).map_err(|e| BackendError::database(e.to_string()))
}
fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn homework_folder(db: &Database, id: &str) -> Result<String, AppError> {
    db.with_connection(|c| {
        c.query_row("SELECT folder_path FROM homeworks WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .map_err(Into::into)
    })
}
fn aggregate_hash(entries: &[ManifestEntry]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for e in entries {
        h.update(e.path.as_bytes());
        h.update(e.sha256.as_bytes());
        h.update(e.size_bytes.to_le_bytes());
    }
    format!("{:x}", h.finalize())
}

fn safe_package_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
}

fn compensate_all(copies: Vec<CommittedCopy>) {
    for copy in copies.into_iter().rev() {
        let _ = copy.compensate();
    }
}

fn proof_kind(name: &str) -> FileKind {
    match std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => FileKind::Pdf,
        "png" | "jpg" | "jpeg" | "heic" | "webp" => FileKind::Image,
        _ => FileKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::SubmissionService;
    use crate::{
        db::Database,
        intake::{RootKind, RootsService},
        workflow::dto::{PackageCheckId, SubmissionConfirmationInput},
    };
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
            let path = "Предметы/Математика/Неделя 07/03 ДЗ/02 Работа/v01.md";
            let attachment_path = "Предметы/Математика/Неделя 07/03 ДЗ/02 Работа/приложение.csv";
            fs::create_dir_all(vault.join("Предметы/Математика/Неделя 07/03 ДЗ/02 Работа"))
                .unwrap();
            fs::write(vault.join(path), b"final answer").unwrap();
            fs::write(vault.join(attachment_path), b"a,b\n1,2\n").unwrap();
            database.with_connection(|c|{c.execute_batch(&format!(r#"
INSERT INTO subjects(id,name,short_name,folder_path) VALUES('s','Математика','мат','Предметы/Математика');
INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path) VALUES('w','s',7,'2026-10-20','2026-10-26','Предметы/Математика/Неделя 07');
INSERT INTO homeworks(id,week_id,title,deadline,stage,workflow_stage,folder_path,assignment_url) VALUES('h','w','ДЗ','2026-10-27','ready_to_submit','ready_to_submit','Предметы/Математика/Неделя 07/03 ДЗ','https://my.centraluniversity.ru/task');
INSERT INTO files(id,homework_id,category,name,kind,size_bytes,relative_path,content_hash) VALUES('f','h','homework_draft','v01.md','text',12,'{path}','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
INSERT INTO homework_versions(id,homework_id,number,label,file_path,size_bytes,author,file_name,source_file_id,sections_json,files_json) VALUES('v','h',1,'v01','{path}',12,'user','v01.md','f','[]','[{{"id":"attachment-file","name":"приложение.csv","kind":"sheet","sizeBytes":8,"modifiedAt":"2026-10-26T10:00:00Z","relativePath":"{attachment_path}"}}]');
UPDATE homeworks SET final_version_id='v',final_file_name='Итог.md' WHERE id='h';
"#))?;Ok(())}).unwrap();
            Self {
                _temp: temp,
                vault,
                database,
            }
        }
    }
    #[test]
    fn package_file_name_follows_saved_pattern() {
        let f = Fixture::new();
        f.database
            .with_connection(|c| {
                c.execute("UPDATE homeworks SET final_file_name=NULL WHERE id='h'", [])?;
                Ok(())
            })
            .unwrap();
        let service = SubmissionService::new(&f.database, None);
        // Шаблон использует фамилию, а она не задана: сборка останавливается.
        assert_eq!(
            service.build("h").unwrap_err().code,
            crate::error::ErrorCode::Blocked
        );
        let rules = crate::dto::AutomationRulesDto {
            student_name: crate::dto::StudentNameDto {
                last_name: "Гончарова".into(),
                first_name: "Доминика".into(),
            },
            final_name_pattern: "{Фамилия}_{Имя}_Неделя{Неделя} {Предмет} {Версия}".into(),
            ..Default::default()
        };
        crate::mutation_service::MutationService::new(&f.database)
            .update_automation_rules(rules)
            .unwrap();
        let built = service.build("h").unwrap();
        let main = built.value.items[0].file.as_ref().unwrap();
        assert_eq!(main.name, "Гончарова_Доминика_Неделя7 Математика v01.md");
        assert!(f.vault.join(&main.relative_path).is_file());
    }

    #[test]
    fn package_and_submitted_snapshot_are_real_and_immutable() {
        let f = Fixture::new();
        let service = SubmissionService::new(&f.database, None);
        let built = service.build("h").unwrap();
        assert_eq!(built.value.items.len(), 2);
        assert!(f
            .vault
            .join(&built.value.items[0].file.as_ref().unwrap().relative_path)
            .is_file());
        let package = service
            .set_check(&built.value.id, PackageCheckId::ManualReview, true)
            .unwrap();
        assert!(package.checklist.iter().all(|x| x.done));
        let proof_path = f._temp.path().join("proof.png");
        fs::write(&proof_path, b"not-a-real-image").unwrap();
        let receipt = service
            .confirm(
                "h",
                &SubmissionConfirmationInput {
                    submitted_at: "2026-10-26T12:00:00Z".into(),
                    assignment_url: "https://my.centraluniversity.ru/task".into(),
                    proof: Some(crate::workflow::dto::SubmissionProofInput {
                        name: "proof.png".into(),
                        size_bytes: 16,
                        path: Some(proof_path.to_string_lossy().into_owned()),
                    }),
                    user_confirmed: true,
                },
                Some((
                    crate::file_service::SourceFile::open_external(&proof_path).unwrap(),
                    "proof.png".into(),
                )),
            )
            .unwrap();
        assert_eq!(receipt.value.files.len(), 2);
        assert!(receipt.value.proof.as_ref().is_some_and(|proof| {
            f.vault.join(&proof.relative_path).is_file()
                && !proof
                    .relative_path
                    .contains(proof_path.to_string_lossy().as_ref())
        }));
        assert!(f.vault.join(&receipt.value.snapshot_path).is_dir());
        assert!(f
            .database
            .with_connection(|c| c
                .execute(
                    "UPDATE submission_receipts SET snapshot_hash='changed' WHERE id=?1",
                    [&receipt.value.id]
                )
                .map(|_| ())
                .map_err(Into::into))
            .is_err());
        assert_eq!(
            service.build("h").unwrap_err().code,
            crate::error::ErrorCode::Immutable
        );
    }
}
