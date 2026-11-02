use std::collections::HashSet;

use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    db::Database,
    domain::{HomeworkWorkflowStage, RequirementStatus},
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
            PackageRequirementDto, PackageStatus, StyleProfileDto, SubmissionConfirmationInput,
            SubmissionPackageDto, SubmissionReceiptDto,
        },
        fingerprint, repository,
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

    /// Этап 8: проверяет, что работа действительно готова по актуальным входным
    /// данным: этап «Готово к сдаче», пройденная готовность и действующая отметка.
    fn require_current_ready(&self, homework_id: &str) -> Result<(), AppError> {
        self.database.with_connection(|c| {
            let snapshot = repository::load_snapshot(c, homework_id)?;
            if snapshot.homework.workflow_stage != HomeworkWorkflowStage::ReadyToSubmit {
                return Err(BackendError::blocked(
                    "Сначала отметьте работу «Готово к сдаче» после всех проверок",
                ));
            }
            if !snapshot
                .inputs
                .is_current(snapshot.ready_fingerprint.as_deref())
            {
                return Err(BackendError::blocked(
                    "«Готово к сдаче» отмечено по устаревшим входным данным — проверьте версию заново",
                ));
            }
            let readiness = crate::workflow::projections::readiness(&snapshot);
            if !readiness.ready {
                let blockers: Vec<_> = readiness
                    .checks
                    .iter()
                    .filter(|check| !check.ok)
                    .map(|check| check.label.as_str())
                    .collect();
                return Err(BackendError::blocked(format!(
                    "Не пройдены проверки готовности: {}",
                    blockers.join("; ")
                )));
            }
            Ok(())
        })
    }

    /// Этап 8: каждая сборка — новая неизменяемая ревизия в своей папке
    /// «03 На сдачу/Ревизия NN». Прежние ревизии остаются в истории.
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
        self.require_current_ready(homework_id)?;
        let inputs = self
            .database
            .with_connection(|c| fingerprint::current(c, homework_id))?;
        let (version_id,source_path,source_name,kind,folder,assignment_url,final_file_name,files_json)=self.database.with_connection(|c|c.query_row("SELECT v.id,v.file_path,v.file_name,coalesce(f.kind,'other'),h.folder_path,h.assignment_url,h.final_file_name,v.files_json FROM homeworks h JOIN homework_versions v ON v.id=h.final_version_id LEFT JOIN files f ON f.id=v.source_file_id WHERE h.id=?1",[homework_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?))).map_err(|e|match e{rusqlite::Error::QueryReturnedNoRows=>BackendError::blocked("Сначала выберите итоговую версию"),o=>o.into()}))?;
        let version_files: Vec<FileRefDto> = serde_json::from_str(&files_json)
            .map_err(|error| BackendError::database(format!("Повреждён состав версии: {error}")))?;
        let requirements: Vec<PackageRequirementDto> = self.database.with_connection(|c| {
            Ok(repository::teacher_requirements(c, homework_id)?
                .into_iter()
                .filter(|item| item.withdrawn_at.is_none())
                .map(|item| PackageRequirementDto {
                    id: item.id,
                    text: item.text,
                    status: item.status,
                })
                .collect())
        })?;
        let revision: i64 = self.database.with_connection(|c| {
            c.query_row(
                "SELECT coalesce(max(revision), 0) + 1 FROM submission_packages WHERE homework_id=?1",
                [homework_id],
                |r| r.get(0),
            )
            .map_err(Into::into)
        })?;
        let package_folder = format!("{folder}/03 На сдачу/Ревизия {revision:02}");
        let vault = self.roots.open_vault()?;
        let safe_name = match final_file_name {
            Some(name) if safe_package_name(&name) => name,
            Some(_) => return Err(BackendError::blocked("Итоговое имя файла небезопасно")),
            None => self.final_name(homework_id, &version_id, &source_name)?,
        };

        // Файлы: основной и приложения версии. Каждый копируется с проверкой SHA-256.
        let mut sources = vec![(
            PackageItemRole::Main,
            source_path.clone(),
            safe_name.clone(),
            FileKind::try_from_db(&kind).unwrap_or(FileKind::Other),
        )];
        let mut names = HashSet::from([safe_name.clone()]);
        for attachment in version_files
            .into_iter()
            .filter(|file| file.relative_path != source_path)
        {
            if !safe_package_name(&attachment.name) {
                return Err(BackendError::blocked(
                    "Имя приложения небезопасно для комплекта",
                ));
            }
            if !names.insert(attachment.name.clone()) {
                return Err(BackendError::conflict(
                    "В версии есть несколько файлов с одинаковым итоговым именем",
                ));
            }
            sources.push((
                PackageItemRole::Attachment,
                attachment.relative_path,
                attachment.name,
                attachment.kind,
            ));
        }
        let mut committed = Vec::new();
        let mut items = Vec::new();
        let mut manifest = Vec::new();
        let mut upload_list = Vec::new();
        for (role, source, name, kind) in sources {
            let copied = (|| {
                let src = SourceFile::open_in(&VerifiedDestination::in_vault(
                    &vault,
                    PersistentPath::new(&source)?,
                )?)?;
                let expected = hash_source(&src)?;
                let target = PersistentPath::new(format!("{package_folder}/{name}"))?;
                let copy = atomic_copy_committed(
                    &src,
                    &VerifiedDestination::in_vault(&vault, target.clone())?,
                    Some(&expected),
                )?;
                Ok::<_, AppError>((target, copy))
            })();
            let (target, copy) = match copied {
                Ok(value) => value,
                Err(error) => {
                    compensate_all(committed);
                    return Err(BackendError::blocked(format!(
                        "Файл «{name}» недоступен для комплекта: {}",
                        error.message
                    )));
                }
            };
            let size = i64::try_from(copy.receipt.size_bytes).unwrap_or(i64::MAX);
            let sha256 = copy.receipt.sha256.as_str().to_owned();
            manifest.push(ManifestEntry {
                path: target.as_str().to_owned(),
                sha256: sha256.clone(),
                size_bytes: copy.receipt.size_bytes,
            });
            upload_list.push(name.clone());
            items.push(PackageItemDto {
                id: Uuid::new_v4().to_string(),
                role,
                file: Some(FileRefDto {
                    id: Uuid::new_v4().to_string(),
                    name: name.clone(),
                    kind,
                    size_bytes: size,
                    modified_at: now(),
                    relative_path: target.as_str().to_owned(),
                }),
                final_name: name,
                required: true,
                present: true,
                status: PackageStatus::Verified,
                sha256: Some(sha256),
                detail: "Скопирован, размер и SHA-256 совпадают".to_owned(),
            });
            committed.push(copy);
        }

        // Ссылки: сетевой проверки нет — только ручное подтверждение.
        let links: Vec<PackageLinkDto> = assignment_url
            .filter(|u| !u.trim().is_empty())
            .map(|url| {
                vec![PackageLinkDto {
                    id: Uuid::new_v4().to_string(),
                    label: "Задание в системе вуза".into(),
                    url,
                    required: true,
                    verified: false,
                    status: PackageStatus::ManualRequired,
                }]
            })
            .unwrap_or_default();
        let not_done = requirements
            .iter()
            .filter(|item| item.status == RequirementStatus::NotDone)
            .count();
        let manual = requirements
            .iter()
            .filter(|item| item.status == RequirementStatus::Manual)
            .count();
        let attachments = items.len() - 1;
        let checklist = vec![
            check(
                PackageCheckId::FinalSelected,
                "Итоговая версия выбрана",
                PackageStatus::Verified,
                true,
                "Выбранная версия совпадает с проверенной",
            ),
            check(
                PackageCheckId::NameChecked,
                "Имя файла по шаблону",
                PackageStatus::Verified,
                true,
                &safe_name,
            ),
            check(
                PackageCheckId::Attachments,
                "Приложения версии на месте",
                if attachments == 0 {
                    PackageStatus::NotApplicable
                } else {
                    PackageStatus::Verified
                },
                true,
                &format!("Скопировано приложений: {attachments}, размер и SHA-256 совпадают"),
            ),
            check(
                PackageCheckId::Links,
                "Внешние ссылки открыты и проверены вручную",
                if links.is_empty() {
                    PackageStatus::NotApplicable
                } else {
                    PackageStatus::ManualRequired
                },
                false,
                "«Контур» не проверяет доступность ссылок по сети",
            ),
            check(
                PackageCheckId::Teacher,
                "Требования преподавателя выполнены",
                if not_done > 0 {
                    PackageStatus::Missing
                } else if manual > 0 {
                    PackageStatus::ManualRequired
                } else {
                    PackageStatus::Verified
                },
                true,
                &format!(
                    "Действующих требований: {}, не выполнено: {not_done}, требует ручной проверки: {manual}",
                    requirements.len()
                ),
            ),
            check(
                PackageCheckId::ManualReview,
                "Я вручную проверил комплект",
                PackageStatus::ManualRequired,
                false,
                "Откройте файлы ревизии и сверьте с заданием",
            ),
        ];
        let mut warnings = Vec::new();
        if !links.is_empty() {
            warnings.push(
                "Ссылку на задание откройте сами: доступность ссылок не проверяется автоматически"
                    .to_owned(),
            );
        }
        if manual > 0 {
            warnings.push(format!(
                "Требований с ручной проверкой: {manual} — закройте их на вкладке преподавателя"
            ));
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
        let inputs_json = json(&inputs)?;
        let db = self.database.with_transaction(|tx| {
            // Входные данные не должны измениться за время сборки.
            if fingerprint::current(tx, homework_id)?.combined != inputs.combined {
                return Err(BackendError::conflict(
                    "Входные данные изменились во время сборки — соберите комплект заново",
                ));
            }
            writes::insert_running_operation(
                tx,
                &op_id,
                OperationKind::BuildPackage,
                &format!("Собрана ревизия комплекта {revision}"),
            )?;
            tx.execute(
                "INSERT INTO submission_packages(id,homework_id,revision,final_version_id,status,package_path,checklist_json,items_json,links_json,warnings_json,upload_list_json,manifest_json,requirements_json,inputs_json,inputs_fingerprint,built_at)
                 VALUES(?1,?2,?3,?4,'draft',?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![
                    id,
                    homework_id,
                    revision,
                    version_id,
                    package_folder,
                    json(&checklist)?,
                    json(&items)?,
                    json(&links)?,
                    json(&warnings)?,
                    json(&upload_list)?,
                    json(&manifest)?,
                    json(&requirements)?,
                    inputs_json,
                    inputs.combined,
                ],
            )?;
            tx.execute(
                "UPDATE operations SET paths_json=?2,undoable=0,homework_id=?3 WHERE id=?1",
                params![op_id, json(&package_paths)?, homework_id],
            )?;
            writes::complete_operation(tx, &op_id)?;
            Ok(())
        });
        if let Err(e) = db {
            let _ = note.compensate();
            compensate_all(committed);
            return Err(e);
        }
        let value = self
            .database
            .with_connection(|c| repository::submission_package_by_id(c, &id))?;
        Ok(OperationResultDto {
            operation: ReadRepository::new(self.database).operation(&op_id)?,
            value,
        })
    }

    /// Этап 8: ручное подтверждение пункта действующей ревизии (журнал, без правки ревизии).
    pub fn set_check(
        &self,
        package_id: &str,
        check_id: PackageCheckId,
        done: bool,
    ) -> Result<SubmissionPackageDto, AppError> {
        self.database.with_transaction(|tx| {
            let package = repository::submission_package_by_id(tx, package_id)?;
            if package.submitted {
                return Err(BackendError::immutable(
                    "Комплект отправленной работы неизменяем",
                ));
            }
            if package.stale {
                return Err(BackendError::blocked(
                    "Ревизия устарела — пересоберите комплект",
                ));
            }
            let item = package
                .checklist
                .iter()
                .find(|x| x.id == check_id)
                .ok_or_else(|| BackendError::not_found("Пункт checklist не найден"))?;
            if item.auto || item.status != PackageStatus::ManualRequired {
                return Err(BackendError::blocked(
                    "Этот пункт проверяет «Контур» — вручную его не отметить",
                ));
            }
            tx.execute(
                "INSERT INTO package_manual_checks(id, package_id, check_id, confirmed) VALUES (?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), package_id, check_id.as_db_str(), done],
            )?;
            repository::submission_package_by_id(tx, package_id)
        })
    }

    /// Этап 8: проверка файлов ревизии на диске — существование, размер и SHA-256.
    /// Запись в JSON сама по себе не доказывает наличие файла.
    pub fn verify(&self, homework_id: &str) -> Result<Option<SubmissionPackageDto>, AppError> {
        let Some(mut package) = self
            .database
            .with_connection(|c| repository::submission_package(c, homework_id))?
        else {
            return Ok(None);
        };
        let vault = self.roots.open_vault()?;
        for item in &mut package.items {
            verify_item(&vault, item);
        }
        Ok(Some(package))
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
        // Этап 8: только текущая ревизия по актуальным данным, с проверкой файлов на диске.
        self.require_current_ready(homework_id)?;
        let package = self
            .verify(homework_id)?
            .ok_or_else(|| BackendError::blocked("Сначала соберите комплект"))?;
        if package.submitted {
            return Err(BackendError::immutable("Комплект уже отправлен"));
        }
        if package.stale {
            return Err(BackendError::blocked(
                "Комплект устарел — пересоберите его по актуальным данным",
            ));
        }
        let broken: Vec<String> = package
            .items
            .iter()
            .filter(|item| item.status.blocks())
            .map(|item| format!("{} — {}", item.final_name, item.detail))
            .collect();
        if !broken.is_empty() {
            return Err(BackendError::blocked(format!(
                "Файлы комплекта не прошли проверку: {}",
                broken.join("; ")
            )));
        }
        let open: Vec<&str> = package
            .checklist
            .iter()
            .filter(|check| !check.done)
            .map(|check| check.label.as_str())
            .collect();
        if !open.is_empty() {
            return Err(BackendError::blocked(format!(
                "Не выполнены пункты комплекта: {}",
                open.join("; ")
            )));
        }
        let package_fingerprint = package.inputs_fingerprint.clone().unwrap_or_default();
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
        let db=self.database.with_transaction(|tx|{
            // Повторная проверка в той же транзакции, что и квитанция.
            let (stage, latest): (String, String) = tx.query_row(
                "SELECT workflow_stage, (SELECT id FROM submission_packages WHERE homework_id = ?1 ORDER BY revision DESC LIMIT 1) FROM homeworks WHERE id = ?1",
                [homework_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if stage != "ready_to_submit" || latest != package.id || fingerprint::current(tx, homework_id)?.combined != package_fingerprint {
                return Err(BackendError::conflict("Данные ДЗ изменились во время подтверждения — проверьте комплект заново"));
            }
            writes::insert_running_operation(tx,&op_id,OperationKind::ConfirmSubmission,"Ручная отправка подтверждена")?;tx.execute("INSERT INTO submission_receipts(id,package_id,homework_id,submitted_version_id,snapshot_path,snapshot_hash,confirmed_at,submitted_at,assignment_url,proof_json,files_json,manifest_json) VALUES(?1,?2,?3,?4,?5,?6,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?7,?8,?9,?10,?11)",params![receipt_id,package.id,homework_id,package.version_id,snapshot_folder,snapshot_hash,input.submitted_at,input.assignment_url,proof_json,json(&files)?,json(&manifest)?])?;tx.execute("UPDATE submission_packages SET status='submitted',updated_at=CURRENT_TIMESTAMP WHERE id=?1",[&package.id])?;tx.execute("UPDATE homeworks SET stage='submitted',workflow_stage='submitted',submitted_at=?2,updated_at=CURRENT_TIMESTAMP WHERE id=?1",params![homework_id,input.submitted_at])?;tx.execute("UPDATE operations SET homework_id=?2,paths_json=?3 WHERE id=?1",params![op_id,homework_id,json(&vec![snapshot_folder.clone()])?])?;writes::complete_operation(tx,&op_id)?;Ok(())});
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
fn check(
    id: PackageCheckId,
    label: &str,
    status: PackageStatus,
    auto: bool,
    detail: &str,
) -> PackageCheckDto {
    PackageCheckDto {
        id,
        label: label.into(),
        done: matches!(
            status,
            PackageStatus::Verified | PackageStatus::NotApplicable
        ),
        auto,
        status,
        detail: detail.into(),
        manual_confirmed: false,
    }
}

/// Существование, размер и SHA-256 файла ревизии в vault.
fn verify_item(vault: &crate::file_service::VaultRoot, item: &mut PackageItemDto) {
    let Some(file) = item.file.as_ref() else {
        item.status = PackageStatus::Missing;
        item.present = false;
        item.detail = "У файла нет пути в комплекте".to_owned();
        return;
    };
    let opened = PersistentPath::new(&file.relative_path)
        .and_then(|path| VerifiedDestination::in_vault(vault, path))
        .and_then(|destination| SourceFile::open_in(&destination));
    let source = match opened {
        Ok(source) => source,
        Err(error) => {
            item.present = false;
            if error.code == ErrorCode::NotFound {
                item.status = PackageStatus::Missing;
                item.detail = "Файла нет в папке ревизии".to_owned();
            } else {
                item.status = PackageStatus::Inaccessible;
                item.detail = format!("Файл недоступен: {}", error.message);
            }
            return;
        }
    };
    let size = source.revalidate().ok();
    let hash = hash_source(&source).ok();
    match (size, hash) {
        (Some(size), Some(hash))
            if i64::try_from(size).ok() == Some(file.size_bytes)
                && item.sha256.as_deref() == Some(hash.as_str()) =>
        {
            item.present = true;
            item.status = PackageStatus::Verified;
            item.detail = "Файл на месте, размер и SHA-256 совпадают".to_owned();
        }
        (Some(_), Some(_)) => {
            item.present = true;
            item.status = PackageStatus::Mismatch;
            item.detail = "Файл изменён после сборки: размер или SHA-256 не совпадают".to_owned();
        }
        _ => {
            item.present = false;
            item.status = PackageStatus::Inaccessible;
            item.detail = "Файл не удалось прочитать".to_owned();
        }
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
