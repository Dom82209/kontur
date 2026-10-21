//! Предпросмотр и импорт экспорта CU LMS Enhancer.
//!
//! Экспорт — ZIP «со всеми материалами и ДЗ», ZIP «без ДЗ» или PDF «без ДЗ»,
//! скачанный пользователем через браузер. «Контур» не входит в LMS, не хранит
//! cookies/сессию и ничего не отправляет: он только читает уже скачанный файл.
//!
//! 1. [`preview`] — индекс архива (без распаковки), классификация по путям,
//!    предупреждения. SHA-256 архива возвращается как отпечаток preview.
//! 2. Пользователь подтверждает или исправляет каждую запись.
//! 3. [`import`] — проверка отпечатка (архив не изменился), распаковка только
//!    выбранных записей во временный каталог (`TempDir`, удаляется при любом
//!    исходе), импорт каждой записи существующим безопасным importer'ом
//!    «Входящих» с происхождением `cu_export` и подтверждённой классификацией.
//!    Ничего не раскладывается по предметам автоматически.

use std::{
    collections::HashSet,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    db::Database,
    dto::{
        ClassificationDto, FileKind, InboxOrigin, InboxStatus, MaterialCategory, ProvenanceDto,
        ProvenanceSource, ProvenanceVia,
    },
    error::{AppError, BackendError},
    file_service::{hash_source, DownloadsRoot, PersistentPath, ScratchRoot, SourceFile},
    intake::{
        detect_kind, is_risky_to_open, safe_inbox_name, validate_operation_id, ImportItemReport,
        ImportMeta, ImportOutcome, InboxImporter, IntakeGrants, MAX_IMPORT_BYTES, MAX_IMPORT_ITEMS,
    },
    repositories::ReadRepository,
};

use super::{
    candidates::{Candidate, CandidateKind, CandidateOrigin, CandidateSource},
    classify::{classify_path, manual_classification, ClassifyContext, SubjectMatcher},
    links::normalize_lms_url,
    store::{self as lms_store, SeenOutcome},
    zip_safe::{
        extract_entry, is_archive_name, read_index, ZipEntry, ZipIndex, ZipLimits, ZipRejection,
    },
};

/// Идентификатор единственной записи для экспорта-PDF и обычного файла.
pub const SINGLE_ENTRY_ID: &str = "file";
const MAX_WEEK: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Zip,
    Single,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportEntryPreviewDto {
    pub entry_id: String,
    /// Относительный путь внутри архива (для экспорта-файла — его имя).
    pub path: String,
    pub name: String,
    /// Имя, под которым файл появится во «Входящих» (до разрешения конфликтов).
    pub inbox_name: String,
    pub size_bytes: u64,
    pub kind: FileKind,
    pub classification: ClassificationDto,
    pub status: InboxStatus,
    /// Можно ли выбрать запись для импорта (вложенный архив — нельзя).
    pub selectable: bool,
    /// Выбрана ли запись по умолчанию.
    pub selected: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreviewDto {
    pub candidate_id: String,
    pub archive_name: String,
    /// Отпечаток предпросмотра: импорт отклоняется, если архив изменился.
    pub archive_sha256: String,
    pub format: ExportFormat,
    pub course_title: Option<String>,
    pub entries: Vec<ExportEntryPreviewDto>,
    /// Служебные файлы архиваторов, пропущенные без импорта.
    pub ignored: Vec<String>,
    pub warnings: Vec<String>,
    pub total_files: usize,
    pub ambiguous_files: usize,
    pub total_bytes: u64,
}

/// Подтверждённая пользователем запись. Отсутствующие в списке записи не импортируются.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportImportEntry {
    pub entry_id: String,
    pub subject_id: Option<String>,
    pub week_number: Option<i64>,
    pub category: MaterialCategory,
    /// Пользователь изменил классификацию в preview.
    #[serde(default)]
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportImportRequest {
    pub candidate_id: String,
    pub archive_sha256: String,
    pub entries: Vec<ExportImportEntry>,
    #[serde(default)]
    pub course_url: Option<String>,
    #[serde(default)]
    pub assignment_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportPhase {
    Inspect,
    Extract,
    Import,
    Cleanup,
}

#[derive(Debug, Clone)]
pub struct ExportProgress {
    pub phase: ExportPhase,
    pub completed: usize,
    pub total: usize,
    pub entry_name: Option<String>,
}

/// Откуда открыть исходный файл кандидата.
pub struct SourceContext<'a> {
    pub downloads_path: Option<PathBuf>,
    pub home: Option<&'a Path>,
    pub grants: &'a IntakeGrants,
}

/// Повторно открывает файл кандидата: из папки загрузок — fd-relative по имени
/// со сверкой inode, из диалога — только если путь разрешён `IntakeGrants`.
pub fn open_candidate(
    candidate: &Candidate,
    context: &SourceContext<'_>,
) -> Result<SourceFile, AppError> {
    match &candidate.source {
        CandidateSource::Downloads {
            identity,
            size_bytes,
            modified_ns,
        } => {
            let path = context.downloads_path.as_ref().ok_or_else(|| {
                BackendError::blocked(
                    "Папка загрузок не выбрана в «Настройки → Управляемые каталоги»",
                )
            })?;
            let source =
                DownloadsRoot::new(path, context.home)?.open_file(&candidate.name, *identity)?;
            // inode может быть переиспользован после удаления: сверяем и размер, и mtime.
            if source.revalidate()? != *size_bytes || source.modified_ns()? != *modified_ns {
                return Err(BackendError::conflict(
                    "Файл в папке загрузок изменился после обнаружения; проверьте его заново",
                ));
            }
            Ok(source)
        }
        CandidateSource::External { path } => {
            if !context.grants.is_granted(path) {
                return Err(BackendError::blocked(
                    "Файл не был выбран через системный диалог — выберите экспорт заново",
                ));
            }
            SourceFile::open_external(path)
        }
    }
}

/// Контекст классификации из SQLite: предметы, ключевые слова, порог.
pub fn classify_context(
    database: &Database,
    course_title: Option<String>,
) -> Result<ClassifyContext, AppError> {
    let repository = ReadRepository::new(database);
    let rules = repository.automation_rules()?;
    let subjects = repository
        .subjects()?
        .into_iter()
        .map(|subject| SubjectMatcher {
            keywords: rules
                .subject_rules
                .iter()
                .filter(|rule| rule.subject_id == subject.id)
                .flat_map(|rule| rule.keywords.clone())
                .collect(),
            id: subject.id,
            name: subject.name,
            short_name: subject.short_name,
        })
        .collect();
    Ok(ClassifyContext {
        subjects,
        threshold: rules.low_confidence_threshold,
        course_title,
    })
}

/// Экспорт-архив определяется по расширению `.zip`; документы Office (тоже ZIP
/// по формату) импортируются как обычные файлы и не распаковываются.
fn is_zip(candidate: &Candidate, _source: &SourceFile) -> bool {
    candidate.kind == CandidateKind::Zip
}

/// Предпросмотр без распаковки и без записи на диск.
pub fn preview(
    database: &Database,
    candidate: &Candidate,
    source: &SourceFile,
    limits: &ZipLimits,
) -> Result<ExportPreviewDto, AppError> {
    let archive_sha256 = hash_source(source)?.as_str().to_owned();
    if candidate
        .sha256
        .as_deref()
        .is_some_and(|expected| expected != archive_sha256)
    {
        return Err(BackendError::conflict(
            "Содержимое файла изменилось после обнаружения; проверьте его заново",
        ));
    }
    if !is_zip(candidate, source) {
        return preview_single(database, candidate, source, archive_sha256);
    }
    let index = read_index(source.file(), limits).map_err(ZipRejection::into_app_error)?;
    let title = course_title(&index, &candidate.name);
    let context = classify_context(database, title.clone())?;

    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    let mut any_week = false;
    for entry in index.files() {
        let mut result = classify_path(&entry.path, &context);
        any_week |= result.classification.week_number.is_some();
        let (kind, _) = detect_kind(entry.file_name());
        let inbox_name = safe_inbox_name(OsStr::new(entry.file_name()))?;
        let mut warnings = Vec::new();
        let mut selectable = true;
        let mut selected = true;
        if is_archive_name(entry.file_name()) {
            selectable = false;
            selected = false;
            warnings.push("Вложенный архив не распаковывается и не импортируется".to_owned());
        }
        if is_risky_to_open(entry.file_name()) {
            selected = false;
            warnings.push("Исполняемый файл или скрипт — по умолчанию не выбран".to_owned());
        }
        if entry.uncompressed_size > MAX_IMPORT_BYTES {
            selectable = false;
            selected = false;
            warnings.push("Файл больше 4 ГБ — импорт невозможен".to_owned());
        }
        if entry.uncompressed_size == 0 {
            warnings.push("Пустой файл".to_owned());
        }
        if inbox_name != entry.file_name() {
            warnings.push(format!(
                "Будет сохранён под безопасным именем «{inbox_name}»"
            ));
        }
        if result.status == InboxStatus::Ambiguous {
            warnings.push("Предмет, неделя или тип не определены уверенно — проверьте".to_owned());
        }
        if !selectable {
            result.status = InboxStatus::Ambiguous;
        }
        total_bytes = total_bytes.saturating_add(entry.uncompressed_size);
        entries.push(ExportEntryPreviewDto {
            entry_id: entry.index.to_string(),
            path: entry.path.clone(),
            name: entry.file_name().to_owned(),
            inbox_name,
            size_bytes: entry.uncompressed_size,
            kind,
            classification: result.classification,
            status: result.status,
            selectable,
            selected,
            warnings,
        });
    }

    let ambiguous_files = entries
        .iter()
        .filter(|entry| entry.selectable && entry.status == InboxStatus::Ambiguous)
        .count();
    let mut warnings = Vec::new();
    if entries.is_empty() {
        warnings.push("В архиве нет файлов для импорта".to_owned());
    }
    if !entries.is_empty() && !any_week {
        warnings.push(
            "В путях архива нет папок недель — возможно, это не экспорт CU LMS Enhancer".to_owned(),
        );
    }
    if ambiguous_files > 0 {
        warnings.push(format!(
            "Требуют уточнения: {ambiguous_files} — такие файлы попадут во «Входящие» без раскладки"
        ));
    }
    if !index.ignored.is_empty() {
        warnings.push(format!(
            "Пропущены служебные файлы архиватора: {}",
            index.ignored.len()
        ));
    }
    Ok(ExportPreviewDto {
        candidate_id: candidate.id.clone(),
        archive_name: candidate.name.clone(),
        archive_sha256,
        format: ExportFormat::Zip,
        course_title: title,
        total_files: entries.len(),
        entries,
        ignored: index.ignored,
        warnings,
        ambiguous_files,
        total_bytes,
    })
}

fn preview_single(
    database: &Database,
    candidate: &Candidate,
    source: &SourceFile,
    archive_sha256: String,
) -> Result<ExportPreviewDto, AppError> {
    let size = source.revalidate()?;
    let title = clean_stem(&candidate.name);
    let context = classify_context(database, title.clone())?;
    let result = classify_path(&candidate.name, &context);
    let (kind, _) = detect_kind(&candidate.name);
    let inbox_name = safe_inbox_name(OsStr::new(&candidate.name))?;
    let mut warnings = Vec::new();
    let mut selected = true;
    let selectable = size <= MAX_IMPORT_BYTES;
    if !selectable {
        selected = false;
        warnings.push("Файл больше 4 ГБ — импорт невозможен".to_owned());
    }
    if is_risky_to_open(&candidate.name) {
        selected = false;
        warnings.push("Исполняемый файл или скрипт — по умолчанию не выбран".to_owned());
    }
    if result.status == InboxStatus::Ambiguous {
        warnings.push("Предмет, неделя или тип не определены уверенно — проверьте".to_owned());
    }
    let ambiguous_files = usize::from(result.status == InboxStatus::Ambiguous);
    Ok(ExportPreviewDto {
        candidate_id: candidate.id.clone(),
        archive_name: candidate.name.clone(),
        archive_sha256,
        format: ExportFormat::Single,
        course_title: title,
        entries: vec![ExportEntryPreviewDto {
            entry_id: SINGLE_ENTRY_ID.to_owned(),
            path: candidate.name.clone(),
            name: candidate.name.clone(),
            inbox_name,
            size_bytes: size,
            kind,
            classification: result.classification,
            status: result.status,
            selectable,
            selected,
            warnings,
        }],
        ignored: Vec::new(),
        warnings: Vec::new(),
        total_files: 1,
        ambiguous_files,
        total_bytes: size,
    })
}

/// Общий корневой каталог всех файлов — название курса; иначе имя архива.
fn course_title(index: &ZipIndex, archive_name: &str) -> Option<String> {
    let mut roots = index.files().map(|entry| {
        let mut parts = entry.path.splitn(2, '/');
        let first = parts.next().unwrap_or_default();
        parts.next().map(|_| first.to_owned())
    });
    let first = roots.next().flatten();
    if let Some(first) = first {
        if roots.all(|root| root.as_deref() == Some(first.as_str())) {
            return Some(first);
        }
    }
    clean_stem(archive_name)
}

/// «Основы_бизнес-аналитики (1).zip» → «Основы бизнес-аналитики».
fn clean_stem(name: &str) -> Option<String> {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let mut stem = stem.trim().to_owned();
    if let Some(open) = stem.rfind(" (") {
        if stem.ends_with(')')
            && stem[open + 2..stem.len() - 1]
                .chars()
                .all(|ch| ch.is_ascii_digit())
        {
            stem.truncate(open);
        }
    }
    let cleaned: String = stem.replace('_', " ").trim().to_owned();
    (!cleaned.is_empty()).then_some(cleaned)
}

pub struct ExportImport<'a> {
    pub database: &'a Database,
    pub importer: &'a InboxImporter<'a>,
    pub scratch_base: &'a Path,
    pub limits: ZipLimits,
}

/// Результат импорта: итог операции и затронутый кандидат.
pub struct ExportImportOutcome {
    pub outcome: ImportOutcome,
    pub archive_sha256: Option<String>,
}

impl ExportImport<'_> {
    /// Импортирует подтверждённые записи. Все проверки запроса выполняются до
    /// любых файловых действий; распаковка идёт только во временный каталог.
    pub fn run(
        &self,
        operation_id: &str,
        candidate: &Candidate,
        source: &SourceFile,
        request: &ExportImportRequest,
        progress: &mut dyn FnMut(ExportProgress),
    ) -> Result<ExportImportOutcome, AppError> {
        validate_operation_id(operation_id)?;
        if let Some(replayed) = self.importer.replay_if_finished(operation_id)? {
            return Ok(ExportImportOutcome {
                outcome: replayed,
                archive_sha256: None,
            });
        }
        if request.candidate_id != candidate.id {
            return Err(BackendError::blocked(
                "Запрос относится к другому кандидату",
            ));
        }
        if request.entries.is_empty() {
            return Err(BackendError::blocked(
                "Не выбрано ни одного файла для импорта",
            ));
        }
        if request.entries.len() > MAX_IMPORT_ITEMS {
            return Err(BackendError::blocked(format!(
                "За один раз можно импортировать не больше {MAX_IMPORT_ITEMS} файлов"
            )));
        }
        let mut ids = HashSet::new();
        for entry in &request.entries {
            if !ids.insert(entry.entry_id.as_str()) {
                return Err(BackendError::blocked("Запись указана дважды"));
            }
            if let Some(subject_id) = &entry.subject_id {
                if !lms_store::subject_exists(self.database, subject_id)? {
                    return Err(BackendError::not_found("Указанный предмет не найден"));
                }
            }
            if entry
                .week_number
                .is_some_and(|week| !(1..=MAX_WEEK).contains(&week))
            {
                return Err(BackendError::blocked("Номер недели должен быть от 1 до 60"));
            }
        }
        let course_url = request
            .course_url
            .as_deref()
            .map(normalize_lms_url)
            .transpose()?
            .flatten();
        let assignment_url = request
            .assignment_url
            .as_deref()
            .map(normalize_lms_url)
            .transpose()?
            .flatten();

        progress(ExportProgress {
            phase: ExportPhase::Inspect,
            completed: 0,
            total: request.entries.len(),
            entry_name: None,
        });
        let archive_sha256 = hash_source(source)?.as_str().to_owned();
        if archive_sha256 != request.archive_sha256 {
            return Err(BackendError::conflict(
                "Файл экспорта изменился после предпросмотра — откройте предпросмотр заново",
            ));
        }
        let zip = is_zip(candidate, source);
        let index = if zip {
            Some(read_index(source.file(), &self.limits).map_err(ZipRejection::into_app_error)?)
        } else {
            None
        };
        let selected: Vec<(&ExportImportEntry, Option<&ZipEntry>)> = request
            .entries
            .iter()
            .map(|entry| match &index {
                None if entry.entry_id == SINGLE_ENTRY_ID => Ok((entry, None)),
                None => Err(BackendError::blocked("Запись не найдена в файле экспорта")),
                Some(index) => index
                    .files()
                    .find(|file| file.index.to_string() == entry.entry_id)
                    .filter(|file| !is_archive_name(file.file_name()))
                    .filter(|file| file.uncompressed_size <= MAX_IMPORT_BYTES)
                    .map(|file| (entry, Some(file)))
                    .ok_or_else(|| {
                        BackendError::blocked(
                            "Запись не найдена в архиве или не может быть импортирована",
                        )
                    }),
            })
            .collect::<Result<_, _>>()?;
        let skipped = index.as_ref().map_or(0, |index| {
            index.files().count().saturating_sub(selected.len())
        });

        let total = selected.len();
        self.importer.begin(
            operation_id,
            "Импорт экспорта ЦУ",
            &format!("{} · выбрано файлов: {total}", candidate.name),
        )?;
        let via = match candidate.origin {
            CandidateOrigin::Watcher => ProvenanceVia::Watcher,
            CandidateOrigin::Manual => ProvenanceVia::Picker,
        };
        // Одиночный файл из watcher, не выбранный как экспорт ЦУ, остаётся «загрузкой».
        let (origin, source_kind) = if !zip
            && candidate.origin == CandidateOrigin::Watcher
            && candidate.kind == CandidateKind::File
        {
            (InboxOrigin::Downloads, ProvenanceSource::Watcher)
        } else {
            (InboxOrigin::Lms, ProvenanceSource::CuExport)
        };
        // Без правок пользователя сохраняется автоматическая классификация,
        // пересчитанная backend по тем же правилам (клиент не может её «повысить»).
        let title = index
            .as_ref()
            .and_then(|index| course_title(index, &candidate.name))
            .or_else(|| clean_stem(&candidate.name));
        let auto_context = classify_context(self.database, title)?;
        let meta_for = |entry: &ExportImportEntry, path: Option<&str>| {
            let classification = if entry.edited {
                manual_classification(entry.subject_id.clone(), entry.week_number, entry.category)
            } else {
                classify_path(path.unwrap_or(&candidate.name), &auto_context)
            };
            ImportMeta {
                origin,
                provenance: ProvenanceDto {
                    source: source_kind,
                    via,
                    archive_name: (zip || source_kind == ProvenanceSource::CuExport)
                        .then(|| safe_display(&candidate.name)),
                    entry_path: path.map(str::to_owned),
                    course_url: course_url.clone(),
                    assignment_url: assignment_url.clone(),
                },
                classification: Some((classification.classification, classification.status)),
            }
        };

        let mut reports: Vec<ImportItemReport> = Vec::with_capacity(total);
        match &index {
            None => {
                let (entry, _) = selected[0];
                progress(ExportProgress {
                    phase: ExportPhase::Import,
                    completed: 0,
                    total,
                    entry_name: Some(candidate.name.clone()),
                });
                let meta = meta_for(entry, None);
                reports.push(self.importer.import_source(
                    operation_id,
                    source,
                    OsStr::new(&candidate.name),
                    &meta,
                ));
                progress(ExportProgress {
                    phase: ExportPhase::Import,
                    completed: 1,
                    total,
                    entry_name: Some(candidate.name.clone()),
                });
            }
            Some(_) => {
                let scratch = match self.scratch() {
                    Ok(scratch) => scratch,
                    Err(error) => {
                        let outcome = self.importer.fail_before_items(operation_id, &error)?;
                        return Ok(ExportImportOutcome {
                            outcome,
                            archive_sha256: Some(archive_sha256),
                        });
                    }
                };
                let root = ScratchRoot::new(scratch.path())?;
                let mut budget = self.limits.max_total_bytes;
                for (position, (entry, zip_entry)) in selected.iter().enumerate() {
                    let zip_entry = zip_entry.expect("zip entries are resolved above");
                    let name = zip_entry.file_name().to_owned();
                    progress(ExportProgress {
                        phase: ExportPhase::Extract,
                        completed: position,
                        total,
                        entry_name: Some(name.clone()),
                    });
                    let report =
                        match self.extract_one(&root, position, source, zip_entry, &mut budget) {
                            Ok(extracted) => {
                                progress(ExportProgress {
                                    phase: ExportPhase::Import,
                                    completed: position,
                                    total,
                                    entry_name: Some(name.clone()),
                                });
                                let meta = meta_for(entry, Some(&zip_entry.path));
                                self.importer.import_source(
                                    operation_id,
                                    &extracted,
                                    OsStr::new(&name),
                                    &meta,
                                )
                            }
                            Err(error) => ImportItemReport::Failed {
                                name: safe_display(&name),
                                error,
                            },
                        };
                    reports.push(report);
                    let _stored = crate::intake::record_progress(
                        self.database,
                        operation_id,
                        (position + 1) as f64 / total as f64,
                    );
                }
                progress(ExportProgress {
                    phase: ExportPhase::Cleanup,
                    completed: total,
                    total,
                    entry_name: None,
                });
                // Явное удаление временного каталога до записи итога.
                drop(root);
                scratch.close().map_err(|_| {
                    BackendError::io("Не удалось удалить временный каталог распаковки")
                })?;
            }
        }
        let outcome = self.importer.finish(operation_id, reports, skipped)?;
        if outcome.failure.is_none() {
            let size = source.revalidate().unwrap_or(candidate.size_bytes);
            let _ = lms_store::mark_seen(
                self.database,
                &archive_sha256,
                &candidate.name,
                size,
                SeenOutcome::Imported,
                Some(operation_id),
            );
        }
        Ok(ExportImportOutcome {
            outcome,
            archive_sha256: Some(archive_sha256),
        })
    }

    fn scratch(&self) -> Result<tempfile::TempDir, AppError> {
        create_private_dir(self.scratch_base)?;
        tempfile::Builder::new()
            .prefix(".kontur-cu-")
            .tempdir_in(self.scratch_base)
            .map_err(|_| BackendError::io("Не удалось создать временный каталог распаковки"))
    }

    fn extract_one(
        &self,
        root: &ScratchRoot,
        position: usize,
        source: &SourceFile,
        entry: &ZipEntry,
        budget: &mut u64,
    ) -> Result<SourceFile, AppError> {
        let name = safe_inbox_name(OsStr::new(entry.file_name()))?;
        let relative = PersistentPath::new(format!("{position:04}/{name}"))?;
        let mut file = root.create_file(&relative)?;
        extract_entry(source.file(), entry, &mut file, &self.limits, budget)
            .map_err(ZipRejection::into_app_error)?;
        drop(file);
        root.open_source(&relative)
    }
}

/// Создаёт базовый каталог для временной распаковки с правами только владельца.
fn create_private_dir(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(BackendError::blocked(
            "Каталог временной распаковки небезопасен",
        )),
        Err(_) => std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| BackendError::io("Не удалось создать каталог временной распаковки")),
    }
}

fn safe_display(name: &str) -> String {
    safe_inbox_name(OsStr::new(name)).unwrap_or_else(|_| "Файл".to_owned())
}
