//! Импорт внешних файлов во «Входящие».
//!
//! Порядок для каждого файла:
//!
//! 1. source разрешён native picker/drop ([`IntakeGrants`]);
//! 2. source открыт один раз ([`SourceFile::open_external`]) — symlink, каталоги и
//!    специальные файлы отклоняются;
//! 3. безопасное имя, тип/MIME по расширению, размер, SHA-256 того же inode;
//! 4. дедупликация по SHA-256 среди активных записей «Входящих» (идемпотентно);
//! 5. выбор имени без overwrite: занятое на диске или в БД имя получает « (N)»;
//! 6. atomic copy (temp → fsync → `RENAME_NOREPLACE` → повторная проверка хэша);
//! 7. запись `InboxFile` в SQLite **только после** успешного filesystem commit;
//! 8. если запись в SQLite не удалась — компенсация: удаляется ровно созданный inode.
//!
//! Рассинхронизация, оставшаяся после аварийного завершения между шагами 6 и 7
//! (файл на диске без записи), исправляется при повторном импорте того же
//! содержимого: существующий файл с тем же SHA-256 принимается без копии.
//! Запись без файла на диске восстанавливается повторным копированием.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::Database,
    dto::{DuplicateInfoDto, InboxFileDto, OperationDto},
    error::{AppError, BackendError, ErrorCode},
    file_service::{
        atomic_copy_committed, hash_source, inspect_stored_file, plan_copy, CopyDecision,
        PersistentPath, SourceFile, StoredFileState, VerifiedDestination,
    },
    repositories::ReadRepository,
};

use super::{
    grants::IntakeGrants,
    naming::{detect_kind, display_name, numbered_name, safe_inbox_name},
    roots::{ImportRoots, INBOX_RELATIVE},
    store::{self, FinishImport, NewInboxRow, OperationLookup},
};

/// Максимальный размер одного импортируемого файла.
pub const MAX_IMPORT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Максимум файлов в одной команде импорта.
pub const MAX_IMPORT_ITEMS: usize = 500;
const MAX_NAME_ATTEMPTS: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportOrigin {
    Picker,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportItemInput {
    pub source_path: String,
    pub origin: ImportOrigin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum ImportItemReport {
    /// Новый файл скопирован во «Входящие».
    #[serde(rename = "imported", rename_all = "camelCase")]
    Imported {
        name: String,
        file: InboxFileDto,
        /// Исходное безопасное имя, если файл сохранён под « (N)».
        renamed_from: Option<String>,
        /// Файл уже был на диске без записи в БД и принят без копирования.
        recovered: bool,
    },
    /// Такое же содержимое уже есть во «Входящих»; новая копия не создавалась.
    #[serde(rename = "duplicate", rename_all = "camelCase")]
    Duplicate {
        name: String,
        file: InboxFileDto,
        /// Запись была, а файла на диске не было — он восстановлен.
        restored: bool,
    },
    #[serde(rename = "failed", rename_all = "camelCase")]
    Failed { name: String, error: AppError },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxImportReport {
    pub operation_id: String,
    pub items: Vec<ImportItemReport>,
    pub imported: usize,
    pub duplicates: usize,
    pub failed: usize,
    /// Ответ повторён из сохранённого итога операции, файловые действия не выполнялись.
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct ImportProgress {
    pub operation_id: String,
    pub progress: f64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ImportOutcome {
    pub report: InboxImportReport,
    pub operation: OperationDto,
    /// Для `operation:failed`: все файлы завершились ошибкой.
    pub failure: Option<AppError>,
}

impl ImportOutcome {
    /// Файлы, которые впервые появились во «Входящих» (для `inbox:file-added`).
    pub fn added_files(&self) -> Vec<InboxFileDto> {
        self.report
            .items
            .iter()
            .filter_map(|item| match item {
                ImportItemReport::Imported { file, .. } => Some(file.clone()),
                _ => None,
            })
            .collect()
    }

    /// Все затронутые записи (для `inbox:updated`).
    pub fn touched_file_ids(&self) -> Vec<String> {
        self.report
            .items
            .iter()
            .filter_map(|item| match item {
                ImportItemReport::Imported { file, .. }
                | ImportItemReport::Duplicate { file, .. } => Some(file.id.clone()),
                ImportItemReport::Failed { .. } => None,
            })
            .collect()
    }
}

pub struct InboxImporter<'a> {
    database: &'a Database,
    roots: &'a ImportRoots,
    grants: &'a IntakeGrants,
}

impl<'a> InboxImporter<'a> {
    pub fn new(database: &'a Database, roots: &'a ImportRoots, grants: &'a IntakeGrants) -> Self {
        Self {
            database,
            roots,
            grants,
        }
    }

    /// Выполняет импорт. `operation_id` задаёт клиент (UUID v4): повтор команды с тем же
    /// id возвращает сохранённый итог без повторных файловых действий.
    pub fn run(
        &self,
        operation_id: &str,
        items: &[ImportItemInput],
        progress: &mut dyn FnMut(ImportProgress),
    ) -> Result<ImportOutcome, AppError> {
        validate_operation_id(operation_id)?;
        if items.is_empty() {
            return Err(BackendError::blocked("Не выбрано ни одного файла"));
        }
        if items.len() > MAX_IMPORT_ITEMS {
            return Err(BackendError::blocked(format!(
                "За один раз можно импортировать не больше {MAX_IMPORT_ITEMS} файлов"
            )));
        }

        match store::lookup_operation(self.database, operation_id)? {
            OperationLookup::Finished(json) => return self.replay(operation_id, &json),
            OperationLookup::Unfinished => {
                return Err(BackendError::conflict(
                    "Импорт с этим идентификатором уже начат и не завершён; обновите «Входящие»",
                ))
            }
            OperationLookup::Missing => {}
        }

        let total = items.len();
        store::start_import_operation(
            self.database,
            operation_id,
            "Импорт во «Входящие»",
            &format!("Файлов: {total}"),
        )?;
        progress(ImportProgress {
            operation_id: operation_id.to_owned(),
            progress: 0.0,
            message: "Проверка файлов".to_owned(),
        });

        let mut reports = Vec::with_capacity(total);
        for (index, item) in items.iter().enumerate() {
            let report = self.import_one(operation_id, item);
            let name = match &report {
                ImportItemReport::Imported { name, .. }
                | ImportItemReport::Duplicate { name, .. }
                | ImportItemReport::Failed { name, .. } => name.clone(),
            };
            reports.push(report);
            let fraction = (index + 1) as f64 / total as f64;
            let _stored = store::update_operation_progress(self.database, operation_id, fraction);
            progress(ImportProgress {
                operation_id: operation_id.to_owned(),
                progress: fraction,
                message: name,
            });
        }

        let imported = count(&reports, |item| {
            matches!(item, ImportItemReport::Imported { .. })
        });
        let duplicates = count(&reports, |item| {
            matches!(item, ImportItemReport::Duplicate { .. })
        });
        let failed = count(&reports, |item| {
            matches!(item, ImportItemReport::Failed { .. })
        });
        let report = InboxImportReport {
            operation_id: operation_id.to_owned(),
            items: reports,
            imported,
            duplicates,
            failed,
            replayed: false,
        };
        let failure = (failed == total).then(|| {
            first_error(&report).unwrap_or_else(|| BackendError::io("Ни один файл не импортирован"))
        });
        let paths: Vec<String> = report
            .items
            .iter()
            .filter_map(|item| match item {
                ImportItemReport::Imported { file, .. } => self.relative_path_of(&file.id),
                ImportItemReport::Duplicate {
                    restored: true,
                    file,
                    ..
                } => self.relative_path_of(&file.id),
                _ => None,
            })
            .collect();
        let detail = format!("Добавлено: {imported} · дублей: {duplicates} · ошибок: {failed}");
        let result_json = serde_json::to_string(&report)
            .map_err(|error| BackendError::database(error.to_string()))?;
        store::finish_import_operation(
            self.database,
            &FinishImport {
                operation_id,
                failed: failure.is_some(),
                detail: &detail,
                paths: &paths,
                error: failure.as_ref(),
                result_json: &result_json,
            },
        )
        .map_err(|error| {
            BackendError::new(
                error.code,
                "Файлы обработаны, но итог операции не сохранён; обновите «Входящие»",
            )
        })?;
        let operation = ReadRepository::new(self.database).operation(operation_id)?;
        Ok(ImportOutcome {
            report,
            operation,
            failure,
        })
    }

    fn replay(&self, operation_id: &str, json: &str) -> Result<ImportOutcome, AppError> {
        let mut report: InboxImportReport = serde_json::from_str(json)
            .map_err(|_| BackendError::database("Сохранённый итог импорта повреждён"))?;
        report.replayed = true;
        let operation = ReadRepository::new(self.database).operation(operation_id)?;
        Ok(ImportOutcome {
            report,
            operation,
            failure: None,
        })
    }

    fn relative_path_of(&self, file_id: &str) -> Option<String> {
        store::inbox_relative_path(self.database, file_id).ok()
    }

    fn import_one(&self, operation_id: &str, item: &ImportItemInput) -> ImportItemReport {
        let path = PathBuf::from(&item.source_path);
        let name = display_name(path.file_name());
        match self.try_import(operation_id, &path, &name) {
            Ok(report) => report,
            Err(error) => ImportItemReport::Failed {
                name,
                error: sanitize_error(error),
            },
        }
    }

    fn try_import(
        &self,
        operation_id: &str,
        path: &Path,
        name: &str,
    ) -> Result<ImportItemReport, AppError> {
        if !path.is_absolute() {
            return Err(BackendError::blocked(
                "Исходный файл должен иметь абсолютный путь",
            ));
        }
        if !self.grants.is_granted(path) {
            return Err(BackendError::blocked(
                "Файл не был выбран через системный диалог или перетаскивание — импорт отклонён",
            ));
        }
        let source = SourceFile::open_external(path)?;
        let size_bytes = source.revalidate()?;
        if size_bytes > MAX_IMPORT_BYTES {
            return Err(BackendError::blocked("Файл больше 4 ГБ — импорт отклонён"));
        }
        let file_name = path
            .file_name()
            .ok_or_else(|| BackendError::blocked("Не указано имя исходного файла"))?;
        let safe_name = safe_inbox_name(file_name)?;
        let sha256 = hash_source(&source)?;

        if let Some(existing) = store::active_inbox_by_sha(self.database, sha256.as_str())? {
            return self.existing_duplicate(
                &source,
                &sha256,
                name,
                &existing.id,
                &existing.relative_path,
            );
        }

        let mut renamed_from = None;
        for attempt in 1..=MAX_NAME_ATTEMPTS {
            let candidate = numbered_name(&safe_name, attempt);
            let relative = PersistentPath::new(format!("{INBOX_RELATIVE}/{candidate}"))?;
            if let Some((existing_id, existing_sha)) =
                store::inbox_by_path(self.database, relative.as_str())?
            {
                // Имя занято записью БД. Legacy-запись без хэша с тем же содержимым
                // на диске считается тем же файлом.
                if existing_sha.is_none() {
                    let destination =
                        VerifiedDestination::in_vault(&self.roots.vault, relative.clone())?;
                    if let StoredFileState::Present { sha256: stored, .. } =
                        inspect_stored_file(&destination)?
                    {
                        if stored == sha256 {
                            return Ok(ImportItemReport::Duplicate {
                                name: name.to_owned(),
                                file: ReadRepository::new(self.database)
                                    .inbox_file(&existing_id)?,
                                restored: false,
                            });
                        }
                    }
                }
                renamed_from.get_or_insert_with(|| safe_name.clone());
                continue;
            }
            let destination = VerifiedDestination::in_vault(&self.roots.vault, relative.clone())?;
            match plan_copy(&source, &destination)? {
                CopyDecision::Copy => {
                    return self.copy_and_record(
                        operation_id,
                        &source,
                        &destination,
                        &candidate,
                        size_bytes,
                        &sha256,
                        name,
                        renamed_from,
                    );
                }
                CopyDecision::SameContent => {
                    // Файл уже на диске (прерванный прежний импорт) без записи в БД.
                    return self.record(
                        operation_id,
                        &candidate,
                        relative.as_str(),
                        size_bytes,
                        &sha256,
                        name,
                        renamed_from,
                        true,
                        None,
                    );
                }
                CopyDecision::NameConflict { .. } => {
                    renamed_from.get_or_insert_with(|| safe_name.clone());
                }
            }
        }
        Err(BackendError::conflict(
            "Не удалось подобрать свободное имя во «Входящих»",
        ))
    }

    fn existing_duplicate(
        &self,
        source: &SourceFile,
        sha256: &crate::file_service::Sha256Digest,
        name: &str,
        existing_id: &str,
        relative_path: &str,
    ) -> Result<ImportItemReport, AppError> {
        let relative = PersistentPath::new(relative_path)
            .map_err(|_| BackendError::database("Небезопасный путь записи «Входящих»"))?;
        let destination = VerifiedDestination::in_vault(&self.roots.vault, relative)?;
        let restored = match inspect_stored_file(&destination)? {
            StoredFileState::Present { sha256: stored, .. } if &stored == sha256 => false,
            StoredFileState::Missing => {
                // Компенсация рассинхронизации «запись есть, файла нет».
                atomic_copy_committed(source, &destination, Some(sha256))?;
                true
            }
            _ => {
                return Err(BackendError::conflict(
                    "Запись во «Входящих» расходится с файлом на диске; ничего не изменено",
                ))
            }
        };
        Ok(ImportItemReport::Duplicate {
            name: name.to_owned(),
            file: ReadRepository::new(self.database).inbox_file(existing_id)?,
            restored,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_and_record(
        &self,
        operation_id: &str,
        source: &SourceFile,
        destination: &VerifiedDestination,
        candidate: &str,
        size_bytes: u64,
        sha256: &crate::file_service::Sha256Digest,
        name: &str,
        renamed_from: Option<String>,
    ) -> Result<ImportItemReport, AppError> {
        let committed = atomic_copy_committed(source, destination, Some(sha256))?;
        let relative = committed.receipt.destination.as_str().to_owned();
        self.record(
            operation_id,
            candidate,
            &relative,
            size_bytes,
            sha256,
            name,
            renamed_from,
            false,
            Some(committed),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        operation_id: &str,
        candidate: &str,
        relative: &str,
        size_bytes: u64,
        sha256: &crate::file_service::Sha256Digest,
        name: &str,
        renamed_from: Option<String>,
        recovered: bool,
        committed: Option<crate::file_service::CommittedCopy>,
    ) -> Result<ImportItemReport, AppError> {
        let (kind, mime_type) = detect_kind(candidate);
        let duplicate = renamed_from.as_ref().map(|original| DuplicateInfoDto {
            existing_path: format!("{INBOX_RELATIVE}/{original}"),
            same_content: false,
            suggested_name: candidate.to_owned(),
        });
        let id = Uuid::new_v4().to_string();
        let inserted = store::insert_inbox_file(
            self.database,
            &NewInboxRow {
                id: &id,
                name: candidate,
                kind,
                size_bytes,
                relative_path: relative,
                sha256: sha256.as_str(),
                mime_type,
                operation_id,
                duplicate: duplicate.as_ref(),
            },
        );
        if let Err(error) = inserted {
            let message = match committed.map(crate::file_service::CommittedCopy::compensate) {
                Some(true) => {
                    "Запись в базу не удалась; созданная копия удалена, «Входящие» не изменены"
                }
                Some(false) => {
                    "Запись в базу не удалась, и копию не удалось удалить; повторите импорт — файл будет принят без дубля"
                }
                None => "Запись в базу не удалась; существующий файл не изменён",
            };
            return Err(BackendError::new(
                if error.code == ErrorCode::Conflict {
                    ErrorCode::Conflict
                } else {
                    ErrorCode::Database
                },
                message,
            ));
        }
        Ok(ImportItemReport::Imported {
            name: name.to_owned(),
            file: ReadRepository::new(self.database).inbox_file(&id)?,
            renamed_from,
            recovered,
        })
    }
}

/// Читает сохранённый итог импорта (команда «получить результат операции»).
pub fn stored_import_report(
    database: &Database,
    operation_id: &str,
) -> Result<Option<InboxImportReport>, AppError> {
    match store::operation_result(database, operation_id)? {
        None => Ok(None),
        Some((kind, json)) if kind == store::RESULT_KIND_INBOX_IMPORT => {
            serde_json::from_str(&json)
                .map(Some)
                .map_err(|_| BackendError::database("Сохранённый итог импорта повреждён"))
        }
        Some(_) => Ok(None),
    }
}

pub fn validate_operation_id(operation_id: &str) -> Result<(), AppError> {
    match Uuid::parse_str(operation_id) {
        Ok(uuid) if uuid.get_version_num() == 4 && operation_id.len() == 36 => Ok(()),
        _ => Err(BackendError::blocked(
            "Идентификатор операции должен быть UUID v4",
        )),
    }
}

fn count(items: &[ImportItemReport], predicate: impl Fn(&ImportItemReport) -> bool) -> usize {
    items.iter().filter(|item| predicate(item)).count()
}

fn first_error(report: &InboxImportReport) -> Option<AppError> {
    report.items.iter().find_map(|item| match item {
        ImportItemReport::Failed { error, .. } => Some(error.clone()),
        _ => None,
    })
}

/// Сообщения ошибок уже не содержат путей; на всякий случай отрезаем всё, что
/// похоже на абсолютный путь, чтобы итог можно было сохранить в SQLite.
fn sanitize_error(error: AppError) -> AppError {
    if error.message.contains('/') || error.message.contains("file:") {
        AppError::new(error.code, "Файл не импортирован")
    } else {
        error
    }
}
