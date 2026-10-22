//! Сценарии этапа 7 без зависимости от GUI: анализ одного материала и пакета,
//! cache по SHA-256, подборка материалов и предпросмотр корпуса.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

use crate::{
    db::Database,
    dto::OperationDto,
    error::{AppError, BackendError, ErrorCode},
    file_service::{read_vault_file, PersistentPath},
    intake::{validate_operation_id, RootsService},
    repositories::ReadRepository,
};

use super::{
    dto::{
        BatchItemOutcome, BatchItemReportDto, CorpusDocumentDto, CorpusExcludedDto, CorpusItemDto,
        CorpusPreviewDto, CorpusSelectionDto, ExtractionBatchReport, ExtractionLimitsDto,
        ExtractionSettingsDto, ExtractionSummaryDto, MaterialExtractionDto,
        MaterialExtractionItemDto, MaterialRef,
    },
    engine::{hint_from_name, options_key, run_isolated, OcrKey},
    model::{Extracted, ExtractionLimits, ExtractionStatus, Interrupt, SourceFormat},
    ocr::{status_of, OcrEngine},
    store::{self, BatchLookup, ExtractionSettings, MaterialInfo, NewExtraction, StoredExtraction},
};

/// Предел числа материалов в одном пакете.
pub const MAX_BATCH_ITEMS: usize = 500;
/// Предел числа материалов в подборке.
pub const MAX_SELECTION_ITEMS: usize = 200;
/// Сколько символов корпуса подборки будет передано анализу.
pub const MAX_CORPUS_CHARS: u64 = 600_000;

/// Прогресс пакета для событий.
#[derive(Debug, Clone)]
pub struct BatchProgress {
    pub completed: usize,
    pub total: usize,
    pub current: Option<(MaterialRef, String)>,
    pub outcome: Option<BatchItemOutcome>,
}

/// Итог пакета: операция журнала, отчёт и ошибка уровня операции (если есть).
#[derive(Debug)]
pub struct BatchOutcome {
    pub operation: OperationDto,
    pub report: ExtractionBatchReport,
    pub failure: Option<AppError>,
    pub touched: Vec<MaterialRef>,
}

pub struct ExtractionService<'a> {
    database: &'a Database,
    home: Option<PathBuf>,
    limits: ExtractionLimits,
    ocr: Option<Arc<dyn OcrEngine>>,
}

fn cancelled_error() -> AppError {
    AppError::new(ErrorCode::Cancelled, "Извлечение остановлено пользователем")
}

impl<'a> ExtractionService<'a> {
    pub fn new(
        database: &'a Database,
        home: Option<PathBuf>,
        limits: ExtractionLimits,
        ocr: Option<Arc<dyn OcrEngine>>,
    ) -> Self {
        Self {
            database,
            home,
            limits,
            ocr,
        }
    }

    fn ocr_key(&self) -> Result<OcrKey, AppError> {
        let settings = store::read_settings(self.database)?;
        Ok(match (settings.ocr_enabled, self.ocr.is_some()) {
            (false, _) => OcrKey::Disabled,
            (true, true) => OcrKey::Enabled,
            (true, false) => OcrKey::Unavailable,
        })
    }

    fn require_material(&self, material: &MaterialRef) -> Result<MaterialInfo, AppError> {
        store::material(self.database, material)?
            .ok_or_else(|| BackendError::not_found("Материал не найден — обновите список"))
    }

    // ───────────────────────── Чтение ─────────────────────────

    pub fn list(&self) -> Result<Vec<MaterialExtractionItemDto>, AppError> {
        let materials = store::all_materials(self.database)?;
        let links: HashMap<MaterialRef, store::LinkSummary> = store::link_summaries(self.database)?
            .into_iter()
            .map(|summary| (summary.material.clone(), summary))
            .collect();
        Ok(materials
            .into_iter()
            .map(|info| {
                let extraction = links.get(&info.material).map(|link| ExtractionSummaryDto {
                    sha256: link.sha256.clone(),
                    format: link.format,
                    status: link.status,
                    status_message: link.status_message.clone(),
                    text_chars: link.text_chars,
                    truncated: link.truncated,
                    warning_count: link.warning_count,
                    analyzed_at: link.analyzed_at.clone(),
                    stale: is_stale(&info, &link.sha256),
                    ocr_used: link.ocr_used,
                });
                MaterialExtractionItemDto {
                    material: info.material,
                    name: info.name,
                    file_kind: info.kind,
                    size_bytes: info.size_bytes,
                    context: info.context,
                    extraction,
                }
            })
            .collect())
    }

    /// Сохранённый результат материала. `not_found`, если анализа ещё не было.
    pub fn get(&self, material: &MaterialRef) -> Result<MaterialExtractionDto, AppError> {
        let info = self.require_material(material)?;
        let (sha256, extraction_id, _) = store::material_link(self.database, material)?
            .ok_or_else(|| BackendError::not_found("Текст из этого материала ещё не извлекался"))?;
        let stored = extraction_id
            .map(|id| store::extraction_by_id(self.database, &id))
            .transpose()?
            .flatten()
            .ok_or_else(|| {
                BackendError::not_found(
                    "Результат извлечения удалён из cache — проанализируйте материал заново",
                )
            })?;
        let stale = is_stale(&info, &sha256);
        Ok(to_dto(&info, stored, stale, true))
    }

    // ───────────────────────── Анализ ─────────────────────────

    /// Анализ одного материала. Повтор для того же SHA-256 берёт результат из
    /// cache; `force` перестраивает его. Оригинал только читается.
    pub fn analyze(
        &self,
        material: &MaterialRef,
        force: bool,
        cancel: Arc<AtomicBool>,
    ) -> Result<MaterialExtractionDto, AppError> {
        let info = self.require_material(material)?;
        let key = options_key(&info.name, self.ocr_key()?);
        if !force {
            if let Some(sha256) = &info.sha256 {
                if let Some(hit) = self.reusable(sha256, &key)? {
                    store::link(self.database, material, sha256, &hit.id)?;
                    return Ok(to_dto(&info, hit, false, true));
                }
            }
        }

        let vault = RootsService::new(self.database, self.home.clone()).open_vault()?;
        let relative = PersistentPath::new(&info.relative_path)?;
        let started = Instant::now();
        let snapshot = read_vault_file(&vault, relative, self.limits.max_input_bytes)?;
        if let Some(expected) = &info.sha256 {
            if snapshot.sha256.as_str() != expected {
                return Err(BackendError::conflict(format!(
                    "Файл «{}» изменился вне «Контура» после импорта — текст не извлекается, чтобы не подменить материал",
                    info.name
                )));
            }
        }
        let sha256 = snapshot.sha256.as_str().to_owned();
        if !force && info.sha256.is_none() {
            if let Some(hit) = self.reusable(&sha256, &key)? {
                store::link(self.database, material, &sha256, &hit.id)?;
                return Ok(to_dto(&info, hit, false, true));
            }
        }
        let extracted = match snapshot.bytes {
            None => Extracted::empty(
                hint_from_name(&info.name).unwrap_or(SourceFormat::Unknown),
                ExtractionStatus::TooLarge,
                format!(
                    "Файл больше {} МБ — текст не извлекается",
                    self.limits.max_input_bytes / (1024 * 1024)
                ),
            ),
            Some(bytes) => {
                let ocr_enabled = matches!(self.ocr_key()?, OcrKey::Enabled | OcrKey::Unavailable);
                match run_isolated(
                    &info.name,
                    Arc::new(bytes),
                    self.limits,
                    cancel,
                    self.ocr.clone(),
                    ocr_enabled,
                ) {
                    Ok(extracted) => extracted,
                    Err(Interrupt::Cancelled) => return Err(cancelled_error()),
                    Err(Interrupt::Timeout) => Extracted::empty(
                        hint_from_name(&info.name).unwrap_or(SourceFormat::Unknown),
                        ExtractionStatus::Timeout,
                        "Разбор не уложился в лимит времени",
                    ),
                }
            }
        };
        let stored = store::save_extraction(
            self.database,
            material,
            &NewExtraction {
                sha256: &sha256,
                options_key: &key,
                extracted: &extracted,
                source_size_bytes: snapshot.size_bytes,
                duration_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u64,
            },
        )?;
        Ok(to_dto(&info, stored, false, false))
    }

    fn reusable(&self, sha256: &str, key: &str) -> Result<Option<StoredExtraction>, AppError> {
        Ok(store::find_cached(self.database, sha256, key)?
            .filter(|stored| stored.extracted.status.reusable()))
    }

    /// Пакетный анализ с прогрессом и отменой. Каждый готовый файл сохраняется
    /// сразу, поэтому отмена не теряет уже извлечённое.
    pub fn run_batch(
        &self,
        operation_id: &str,
        materials: &[MaterialRef],
        force: bool,
        cancel: Arc<AtomicBool>,
        progress: &mut dyn FnMut(BatchProgress),
    ) -> Result<BatchOutcome, AppError> {
        validate_operation_id(operation_id)?;
        match store::lookup_batch(self.database, operation_id)? {
            BatchLookup::Finished(json) => {
                let mut report: ExtractionBatchReport = serde_json::from_str(&json)
                    .map_err(|_| BackendError::database("Сохранённый итог извлечения повреждён"))?;
                report.replayed = true;
                let operation = ReadRepository::new(self.database).operation(operation_id)?;
                return Ok(BatchOutcome {
                    operation,
                    report,
                    failure: None,
                    touched: Vec::new(),
                });
            }
            BatchLookup::Unfinished => {
                return Err(BackendError::conflict(
                    "Извлечение с этим идентификатором уже начато и не завершено",
                ))
            }
            BatchLookup::Missing => {}
        }
        let mut seen = HashSet::new();
        let materials: Vec<MaterialRef> = materials
            .iter()
            .filter(|material| seen.insert((*material).clone()))
            .cloned()
            .collect();
        if materials.is_empty() {
            return Err(BackendError::blocked("Выберите хотя бы один материал"));
        }
        if materials.len() > MAX_BATCH_ITEMS {
            return Err(BackendError::blocked(format!(
                "За один раз можно проанализировать не больше {MAX_BATCH_ITEMS} материалов"
            )));
        }
        let total = materials.len();
        store::start_batch(self.database, operation_id, &format!("Материалов: {total}"))?;
        progress(BatchProgress {
            completed: 0,
            total,
            current: None,
            outcome: None,
        });

        let mut items: Vec<BatchItemReportDto> = Vec::with_capacity(total);
        let mut stopped = false;
        for (index, material) in materials.iter().enumerate() {
            let name = store::material(self.database, material)?
                .map(|info| info.name)
                .unwrap_or_else(|| "Материал не найден".into());
            if stopped || cancel.load(std::sync::atomic::Ordering::Relaxed) {
                stopped = true;
                items.push(item(
                    material,
                    &name,
                    BatchItemOutcome::Cancelled,
                    None,
                    0,
                    None,
                ));
                continue;
            }
            let report = match self.analyze(material, force, Arc::clone(&cancel)) {
                Ok(result) => item(
                    material,
                    &name,
                    if result.cached {
                        BatchItemOutcome::Cached
                    } else {
                        BatchItemOutcome::Extracted
                    },
                    Some(result.status),
                    result.text_chars,
                    None,
                ),
                Err(error) if error.code == ErrorCode::Cancelled => {
                    stopped = true;
                    item(material, &name, BatchItemOutcome::Cancelled, None, 0, None)
                }
                Err(error) => item(
                    material,
                    &name,
                    BatchItemOutcome::Failed,
                    None,
                    0,
                    Some(error),
                ),
            };
            let outcome = report.outcome;
            items.push(report);
            let completed = index + 1;
            let _ =
                store::batch_progress(self.database, operation_id, completed as f64 / total as f64);
            progress(BatchProgress {
                completed,
                total,
                current: Some((material.clone(), name)),
                outcome: Some(outcome),
            });
        }

        let count =
            |outcome: BatchItemOutcome| items.iter().filter(|item| item.outcome == outcome).count();
        let report = ExtractionBatchReport {
            operation_id: operation_id.to_owned(),
            extracted: count(BatchItemOutcome::Extracted),
            cached: count(BatchItemOutcome::Cached),
            failed: count(BatchItemOutcome::Failed),
            cancelled: count(BatchItemOutcome::Cancelled),
            stopped_by_user: stopped,
            replayed: false,
            items,
        };
        let processed = report.extracted + report.cached;
        let failure = if stopped {
            Some(AppError::new(
                ErrorCode::Cancelled,
                format!("Остановлено пользователем: обработано {processed} из {total}"),
            ))
        } else if processed == 0 {
            Some(AppError::new(
                ErrorCode::IoError,
                "Ни один материал не обработан",
            ))
        } else {
            None
        };
        let detail = format!(
            "Извлечено: {}, из cache: {}, ошибок: {}{}",
            report.extracted,
            report.cached,
            report.failed,
            if stopped {
                format!(", остановлено: {}", report.cancelled)
            } else {
                String::new()
            }
        );
        let report_json = serde_json::to_string(&report)
            .map_err(|error| BackendError::database(error.to_string()))?;
        store::finish_batch(
            self.database,
            operation_id,
            failure.as_ref(),
            &detail,
            &report_json,
        )?;
        let operation = ReadRepository::new(self.database).operation(operation_id)?;
        let touched = report
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item.outcome,
                    BatchItemOutcome::Extracted | BatchItemOutcome::Cached
                )
            })
            .map(|item| item.material.clone())
            .collect();
        Ok(BatchOutcome {
            operation,
            report,
            failure,
            touched,
        })
    }

    pub fn clear_cache(&self) -> Result<u64, AppError> {
        store::clear_cache(self.database)
    }

    // ───────────────────────── Настройки ─────────────────────────

    pub fn settings(&self) -> Result<ExtractionSettingsDto, AppError> {
        let settings = store::read_settings(self.database)?;
        Ok(ExtractionSettingsDto {
            ocr_enabled: settings.ocr_enabled,
            ocr: status_of(self.ocr.as_deref()),
            limits: ExtractionLimitsDto::from_limits(&self.limits, MAX_CORPUS_CHARS),
        })
    }

    pub fn set_ocr_enabled(&self, enabled: bool) -> Result<ExtractionSettingsDto, AppError> {
        store::write_settings(
            self.database,
            ExtractionSettings {
                ocr_enabled: enabled,
            },
        )?;
        self.settings()
    }

    // ───────────────────────── Подборка ─────────────────────────

    fn check_homework(&self, homework_id: Option<&str>) -> Result<(), AppError> {
        if let Some(homework_id) = homework_id {
            if !store::homework_exists(self.database, homework_id)? {
                return Err(BackendError::not_found("ДЗ не найдено"));
            }
        }
        Ok(())
    }

    pub fn selection(&self, homework_id: Option<&str>) -> Result<CorpusSelectionDto, AppError> {
        self.check_homework(homework_id)?;
        let row = store::read_selection(self.database, homework_id)?;
        let mut items = Vec::new();
        let mut total = 0_u64;
        if let Some(row) = &row {
            for (material, pinned) in &row.items {
                let item = self.selection_item(material, pinned)?;
                if item.ready {
                    total += item.text_chars;
                }
                items.push(item);
            }
        }
        Ok(CorpusSelectionDto {
            homework_id: homework_id.map(str::to_owned),
            updated_at: row.map(|row| row.updated_at),
            items,
            total_chars: total,
            limit_chars: MAX_CORPUS_CHARS,
        })
    }

    fn selection_item(
        &self,
        material: &MaterialRef,
        pinned: &str,
    ) -> Result<CorpusItemDto, AppError> {
        let Some(info) = store::material(self.database, material)? else {
            return Ok(CorpusItemDto {
                material: material.clone(),
                name: "Материал удалён".into(),
                context: String::new(),
                sha256: pinned.to_owned(),
                status: None,
                text_chars: 0,
                warning_count: 0,
                ready: false,
                problem: Some("Материал больше не существует".into()),
            });
        };
        let (status, text_chars, warning_count, problem) =
            match self.current_extraction(&info, pinned)? {
                Ok(stored) => (
                    Some(stored.extracted.status),
                    stored.extracted.text_chars as u64,
                    stored.extracted.warnings.len() as u64,
                    None,
                ),
                Err(problem) => (None, 0, 0, Some(problem)),
            };
        Ok(CorpusItemDto {
            material: material.clone(),
            name: info.name,
            context: info.context,
            sha256: pinned.to_owned(),
            status,
            text_chars,
            warning_count,
            ready: problem.is_none(),
            problem,
        })
    }

    /// Текущий пригодный результат для закреплённого содержимого или причина отказа.
    fn current_extraction(
        &self,
        info: &MaterialInfo,
        pinned: &str,
    ) -> Result<Result<StoredExtraction, String>, AppError> {
        if info
            .sha256
            .as_deref()
            .is_some_and(|current| current != pinned)
        {
            return Ok(Err(
                "Файл изменился после выбора — выберите его заново".into()
            ));
        }
        let Some((sha256, extraction_id, _)) = store::material_link(self.database, &info.material)?
        else {
            return Ok(Err("Текст ещё не извлечён".into()));
        };
        if sha256 != pinned {
            return Ok(Err(
                "Текст извлечён из другой версии файла — выберите заново".into(),
            ));
        }
        let Some(stored) = extraction_id
            .map(|id| store::extraction_by_id(self.database, &id))
            .transpose()?
            .flatten()
        else {
            return Ok(Err(
                "Результат удалён из cache — проанализируйте заново".into()
            ));
        };
        if !stored.extracted.status.usable_for_corpus() {
            return Ok(Err(stored
                .extracted
                .status_message
                .clone()
                .unwrap_or_else(|| "Из файла не извлечён текст".into())));
        }
        Ok(Ok(stored))
    }

    /// Заменяет подборку. Принимаются только материалы с извлечённым текстом.
    pub fn set_selection(
        &self,
        homework_id: Option<&str>,
        materials: &[MaterialRef],
    ) -> Result<CorpusSelectionDto, AppError> {
        self.check_homework(homework_id)?;
        if let Some(homework_id) = homework_id {
            if store::homework_submitted(self.database, homework_id)? {
                return Err(BackendError::immutable(
                    "Работа отправлена — подборку материалов изменить нельзя",
                ));
            }
        }
        let mut seen = HashSet::new();
        let materials: Vec<&MaterialRef> = materials
            .iter()
            .filter(|material| seen.insert((*material).clone()))
            .collect();
        if materials.len() > MAX_SELECTION_ITEMS {
            return Err(BackendError::blocked(format!(
                "В подборке может быть не больше {MAX_SELECTION_ITEMS} материалов"
            )));
        }
        let mut items = Vec::with_capacity(materials.len());
        for material in materials {
            let info = self.require_material(material)?;
            let Some((sha256, _, _)) = store::material_link(self.database, material)? else {
                return Err(BackendError::blocked(format!(
                    "«{}»: сначала извлеките текст",
                    info.name
                )));
            };
            if let Err(problem) = self.current_extraction(&info, &sha256)? {
                return Err(BackendError::blocked(format!("«{}»: {problem}", info.name)));
            }
            if is_stale(&info, &sha256) {
                return Err(BackendError::blocked(format!(
                    "«{}»: файл изменился после анализа — проанализируйте заново",
                    info.name
                )));
            }
            items.push((material.clone(), sha256));
        }
        store::replace_selection(self.database, homework_id, &items)?;
        self.selection(homework_id)
    }

    /// Корпус подборки в порядке выбора, с общим лимитом символов.
    pub fn preview_corpus(&self, homework_id: Option<&str>) -> Result<CorpusPreviewDto, AppError> {
        self.check_homework(homework_id)?;
        let row = store::read_selection(self.database, homework_id)?;
        let mut documents = Vec::new();
        let mut excluded = Vec::new();
        let mut total = 0_u64;
        let mut truncated = false;
        for (material, pinned) in row.map(|row| row.items).unwrap_or_default() {
            let Some(info) = store::material(self.database, &material)? else {
                excluded.push(CorpusExcludedDto {
                    material,
                    name: "Материал удалён".into(),
                    reason: "Материал больше не существует".into(),
                });
                continue;
            };
            let stored = match self.current_extraction(&info, &pinned)? {
                Ok(stored) => stored,
                Err(reason) => {
                    excluded.push(CorpusExcludedDto {
                        material,
                        name: info.name,
                        reason,
                    });
                    continue;
                }
            };
            let remaining = MAX_CORPUS_CHARS.saturating_sub(total);
            if remaining == 0 {
                truncated = true;
                excluded.push(CorpusExcludedDto {
                    material,
                    name: info.name,
                    reason: format!("Не помещается в лимит корпуса {MAX_CORPUS_CHARS} символов"),
                });
                continue;
            }
            let chars = stored.extracted.text_chars as u64;
            let (text, cut) = if chars > remaining {
                truncated = true;
                (
                    stored
                        .extracted
                        .text
                        .chars()
                        .take(remaining as usize)
                        .collect::<String>(),
                    true,
                )
            } else {
                (stored.extracted.text.clone(), false)
            };
            let text_chars = text.chars().count() as u64;
            total += text_chars;
            documents.push(CorpusDocumentDto {
                material,
                name: info.name,
                sha256: stored.sha256,
                format: stored.extracted.format,
                status: stored.extracted.status,
                text,
                text_chars,
                truncated: cut || stored.extracted.truncated,
                warnings: stored.extracted.warnings,
            });
        }
        Ok(CorpusPreviewDto {
            homework_id: homework_id.map(str::to_owned),
            documents,
            excluded,
            total_chars: total,
            limit_chars: MAX_CORPUS_CHARS,
            truncated,
        })
    }
}

fn is_stale(info: &MaterialInfo, analyzed_sha: &str) -> bool {
    info.sha256
        .as_deref()
        .is_some_and(|current| current != analyzed_sha)
}

fn item(
    material: &MaterialRef,
    name: &str,
    outcome: BatchItemOutcome,
    status: Option<ExtractionStatus>,
    text_chars: u64,
    error: Option<AppError>,
) -> BatchItemReportDto {
    BatchItemReportDto {
        material: material.clone(),
        name: name.to_owned(),
        outcome,
        status,
        text_chars,
        error,
    }
}

fn to_dto(
    info: &MaterialInfo,
    stored: StoredExtraction,
    stale: bool,
    cached: bool,
) -> MaterialExtractionDto {
    let extracted = stored.extracted;
    MaterialExtractionDto {
        material: info.material.clone(),
        name: info.name.clone(),
        context: info.context.clone(),
        sha256: stored.sha256,
        extractor_version: stored.extractor_version,
        format: extracted.format,
        status: extracted.status,
        status_message: extracted.status_message,
        metadata: extracted.metadata,
        blocks: extracted.blocks,
        blocks_truncated: extracted.blocks_truncated,
        text: extracted.text,
        text_chars: extracted.text_chars as u64,
        truncated: extracted.truncated,
        warnings: extracted.warnings,
        ocr_used: extracted.ocr_used,
        source_size_bytes: stored.source_size_bytes,
        duration_ms: stored.duration_ms,
        extracted_at: stored.created_at,
        stale,
        cached,
    }
}
