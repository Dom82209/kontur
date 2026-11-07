//! Этап 10: родословная версий, структурное сравнение и план исправлений.
//!
//! - Версии неизменяемы; родитель, происхождение и причина записываются при
//!   создании (см. `workflow::service::create_version`), решения пользователя —
//!   отдельным журналом `version_decisions`.
//! - Сравнение хранит только типизированные изменения и короткие фрагменты;
//!   оно устаревает, если файлы версий или версия алгоритма изменились.
//! - План исправлений строится только из текущей проверки итоговой версии и
//!   действующих требований преподавателя. Пункты из проверки AI остаются
//!   предложениями, пока пользователь их не подтвердит.

pub mod diff;
#[cfg(test)]
mod tests;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::Database,
    domain::{HomeworkStage, RequirementStatus},
    dto::HomeworkVersionDto,
    error::{AppError, BackendError},
    file_service::{read_vault_file, PersistentPath},
    intake::RootsService,
    workflow::{
        dto::{ReviewRunStatus, RunKind},
        fingerprint, repository,
    },
};

use diff::{ChangeDto, ComparisonStatus, FileInput, EXTRACTOR_VERSION};

const MAX_COMPARE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionComparisonDto {
    pub id: String,
    pub homework_id: String,
    pub base_version_id: String,
    pub target_version_id: String,
    pub base_fingerprint: String,
    pub target_fingerprint: String,
    pub extractor_version: i64,
    pub status: ComparisonStatus,
    pub summary: String,
    pub changes: Vec<ChangeDto>,
    pub warnings: Vec<String>,
    pub manual_checks: Vec<String>,
    pub created_at: String,
    /// Файлы версий или алгоритм сравнения изменились после сравнения.
    pub stale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionStatus {
    Proposed,
    Open,
    Resolved,
    Dismissed,
}

impl CorrectionStatus {
    fn as_db(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }
    fn from_db(value: &str) -> Self {
        match value {
            "open" => Self::Open,
            "resolved" => Self::Resolved,
            "dismissed" => Self::Dismissed,
            _ => Self::Proposed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionItemDto {
    pub id: String,
    pub position: i64,
    pub category: String,
    pub issue: String,
    pub source_requirement: String,
    pub evidence: String,
    pub file: Option<String>,
    pub location: Option<String>,
    pub expected_change: String,
    pub acceptance_criterion: String,
    pub priority: String,
    pub origin: String,
    pub status: CorrectionStatus,
    pub linked_review_run_id: Option<String>,
    pub linked_comment_id: Option<String>,
    pub linked_requirement_id: Option<String>,
    pub resolved_in_version_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionPlanDto {
    pub id: String,
    pub homework_id: String,
    pub version_id: String,
    pub review_run_id: Option<String>,
    pub created_at: String,
    pub items: Vec<CorrectionItemDto>,
    /// Требования, проверка или итоговая версия изменились после построения плана.
    pub stale: bool,
}

/// Отпечаток набора файлов версии: путь и SHA-256 каждого файла.
pub fn set_fingerprint(files: &[FileInput]) -> String {
    let mut lines: Vec<String> = files
        .iter()
        .map(|f| {
            format!(
                "{}|{}",
                f.relative_path,
                f.sha256.as_deref().unwrap_or("unread")
            )
        })
        .collect();
    lines.sort();
    let mut hasher = Sha256::new();
    hasher.update(lines.join("\n").as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Сохранённое сравнение устарело: файлы или алгоритм не те же.
pub fn is_stale(stored: &VersionComparisonDto, base_fp: &str, target_fp: &str) -> bool {
    stored.base_fingerprint != base_fp
        || stored.target_fingerprint != target_fp
        || stored.extractor_version != EXTRACTOR_VERSION
}

fn version(
    connection: &Connection,
    homework_id: &str,
    id: &str,
) -> Result<HomeworkVersionDto, AppError> {
    repository::homework_dto(connection, homework_id)?
        .versions
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| BackendError::conflict("Версия не относится к этому ДЗ"))
}

/// Цепочка предков версии от корня до неё; защита от циклов — по множеству id.
pub fn lineage(
    connection: &Connection,
    homework_id: &str,
    version_id: &str,
) -> Result<Vec<HomeworkVersionDto>, AppError> {
    let versions = repository::homework_dto(connection, homework_id)?.versions;
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = Some(version_id.to_owned());
    while let Some(id) = cursor {
        if !seen.insert(id.clone()) {
            return Err(BackendError::conflict(
                "В родословной версий обнаружен цикл",
            ));
        }
        let Some(v) = versions.iter().find(|v| v.id == id) else {
            return Err(BackendError::not_found("Версия не найдена в этом ДЗ"));
        };
        cursor = v.lineage.parent_version_id.clone();
        chain.push(v.clone());
    }
    chain.reverse();
    Ok(chain)
}

const COMPARISON_SELECT: &str = "SELECT id, homework_id, base_version_id, target_version_id,
    base_fingerprint, target_fingerprint, extractor_version, status, summary, changes_json,
    warnings_json, manual_checks_json, created_at FROM version_comparisons";

fn comparison_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<VersionComparisonDto> {
    let json = |index: usize| -> rusqlite::Result<String> { row.get(index) };
    Ok(VersionComparisonDto {
        id: row.get(0)?,
        homework_id: row.get(1)?,
        base_version_id: row.get(2)?,
        target_version_id: row.get(3)?,
        base_fingerprint: row.get(4)?,
        target_fingerprint: row.get(5)?,
        extractor_version: row.get(6)?,
        status: ComparisonStatus::from_db(&row.get::<_, String>(7)?),
        summary: row.get(8)?,
        changes: serde_json::from_str(&json(9)?).unwrap_or_default(),
        warnings: serde_json::from_str(&json(10)?).unwrap_or_default(),
        manual_checks: serde_json::from_str(&json(11)?).unwrap_or_default(),
        created_at: row.get(12)?,
        stale: false,
    })
}

const ITEM_SELECT: &str =
    "SELECT id, position, category, issue, source_requirement, evidence, file,
    location, expected_change, acceptance_criterion, priority, origin, status, linked_review_run_id,
    linked_comment_id, linked_requirement_id, resolved_in_version_id, updated_at
    FROM correction_items";

fn item_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CorrectionItemDto> {
    Ok(CorrectionItemDto {
        id: row.get(0)?,
        position: row.get(1)?,
        category: row.get(2)?,
        issue: row.get(3)?,
        source_requirement: row.get(4)?,
        evidence: row.get(5)?,
        file: row.get(6)?,
        location: row.get(7)?,
        expected_change: row.get(8)?,
        acceptance_criterion: row.get(9)?,
        priority: row.get(10)?,
        origin: row.get(11)?,
        status: CorrectionStatus::from_db(&row.get::<_, String>(12)?),
        linked_review_run_id: row.get(13)?,
        linked_comment_id: row.get(14)?,
        linked_requirement_id: row.get(15)?,
        resolved_in_version_id: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

/// Новый пункт плана (без id и статуса).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub category: String,
    pub issue: String,
    pub source_requirement: String,
    pub evidence: String,
    pub file: Option<String>,
    pub location: Option<String>,
    pub expected_change: String,
    pub acceptance_criterion: String,
    pub priority: &'static str,
    pub origin: &'static str,
    pub status: CorrectionStatus,
    pub linked_review_run_id: Option<String>,
    pub linked_comment_id: Option<String>,
    pub linked_requirement_id: Option<String>,
}

/// Текущая проверка итоговой версии: последний завершённый запуск с результатом.
fn current_review(
    snapshot: &repository::WorkflowSnapshot,
    version_id: &str,
) -> Option<crate::workflow::dto::ReviewRunDto> {
    snapshot
        .runs
        .iter()
        .filter(|run| {
            matches!(run.kind, RunKind::ReviewVersion | RunKind::ReviewTeacher)
                && run.status == ReviewRunStatus::Completed
                && run.version_id.as_deref() == Some(version_id)
                && run.result.is_some()
        })
        .max_by(|a, b| a.started_at.cmp(&b.started_at))
        .cloned()
}

/// Пункты плана — только из текущей проверки и действующих требований.
pub fn plan_items(
    snapshot: &repository::WorkflowSnapshot,
    version_id: &str,
) -> (Option<String>, Vec<NewItem>) {
    let mut items = Vec::new();
    let file = snapshot
        .homework
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .map(|v| v.file_name.clone());
    let review = current_review(snapshot, version_id);
    if let Some(run) = &review {
        let result = run.result.as_ref().expect("filtered");
        let findings = result
            .critical
            .iter()
            .map(|f| (f, "high", "Критическое замечание"))
            .chain(result.remarks.iter().map(|f| (f, "medium", "Замечание")));
        for (finding, priority, category) in findings {
            items.push(NewItem {
                category: category.to_owned(),
                issue: finding.text.clone(),
                source_requirement: result
                    .criteria
                    .iter()
                    .find(|c| c.id == finding.criterion)
                    .map(|c| c.label.clone())
                    .unwrap_or_else(|| finding.criterion.as_db_str().to_owned()),
                evidence: finding
                    .evidence
                    .iter()
                    .map(|e| format!("{} — {}", e.file_name, e.locator))
                    .collect::<Vec<_>>()
                    .join("; "),
                file: file.clone(),
                location: finding.location.clone(),
                expected_change: format!("Исправить: {}", finding.text),
                acceptance_criterion: "Повторная проверка новой версии не повторяет это замечание"
                    .to_owned(),
                priority,
                origin: "review",
                status: CorrectionStatus::Proposed,
                linked_review_run_id: Some(run.id.clone()),
                linked_comment_id: None,
                linked_requirement_id: None,
            });
        }
        for claim in &result.unsupported_claims {
            items.push(NewItem {
                category: "Неподтверждённое утверждение".to_owned(),
                issue: claim.text.clone(),
                source_requirement: "Опора на материалы курса".to_owned(),
                evidence: String::new(),
                file: file.clone(),
                location: Some(claim.location.clone()),
                expected_change: "Подтвердить ссылкой на материалы курса или убрать".to_owned(),
                acceptance_criterion: "У утверждения есть источник из материалов".to_owned(),
                priority: "medium",
                origin: "review",
                status: CorrectionStatus::Proposed,
                linked_review_run_id: Some(run.id.clone()),
                linked_comment_id: None,
                linked_requirement_id: None,
            });
        }
    }
    for requirement in snapshot.teacher_requirements.iter().filter(|r| {
        r.withdrawn_at.is_none()
            && matches!(
                r.status,
                RequirementStatus::NotDone | RequirementStatus::Manual
            )
    }) {
        let not_done = requirement.status == RequirementStatus::NotDone;
        items.push(NewItem {
            category: "Требование преподавателя".to_owned(),
            issue: if not_done {
                format!("Не выполнено: {}", requirement.text)
            } else {
                format!("Требует ручной проверки: {}", requirement.text)
            },
            source_requirement: requirement.text.clone(),
            evidence: requirement.origin.clone(),
            file: file.clone(),
            location: None,
            expected_change: format!("Выполнить требование: {}", requirement.text),
            acceptance_criterion: "Требование отмечено выполненным для новой версии".to_owned(),
            priority: if not_done { "high" } else { "medium" },
            origin: "requirement",
            // Требование уже подтверждено пользователем — пункт сразу открыт.
            status: CorrectionStatus::Open,
            linked_review_run_id: None,
            linked_comment_id: requirement.comment_id.clone(),
            linked_requirement_id: Some(requirement.id.clone()),
        });
    }
    (review.map(|r| r.id), items)
}

pub struct VersionService<'a> {
    database: &'a Database,
    roots: RootsService<'a>,
}

impl<'a> VersionService<'a> {
    pub fn new(database: &'a Database, home: Option<std::path::PathBuf>) -> Self {
        Self {
            database,
            roots: RootsService::new(database, home),
        }
    }

    pub fn lineage(
        &self,
        homework_id: &str,
        version_id: &str,
    ) -> Result<Vec<HomeworkVersionDto>, AppError> {
        self.database
            .with_connection(|c| lineage(c, homework_id, version_id))
    }

    /// Файлы версии с SHA-256 и содержимым (если помещается в лимит).
    fn read_files(&self, version: &HomeworkVersionDto) -> Result<Vec<FileInput>, AppError> {
        let vault = self.roots.open_vault()?;
        Ok(version
            .files
            .iter()
            .map(|file| {
                let snapshot = PersistentPath::new(&file.relative_path)
                    .and_then(|path| read_vault_file(&vault, path, MAX_COMPARE_BYTES))
                    .ok();
                FileInput {
                    name: file.name.clone(),
                    relative_path: file.relative_path.clone(),
                    size_bytes: snapshot
                        .as_ref()
                        .map_or(u64::try_from(file.size_bytes).unwrap_or(0), |s| {
                            s.size_bytes
                        }),
                    sha256: snapshot.as_ref().map(|s| s.sha256.as_str().to_owned()),
                    bytes: snapshot.and_then(|s| s.bytes),
                    main: file.id == version.source_file_id,
                }
            })
            .collect())
    }

    fn pair(
        &self,
        homework_id: &str,
        base: &str,
        target: &str,
    ) -> Result<(HomeworkVersionDto, HomeworkVersionDto), AppError> {
        if base == target {
            return Err(BackendError::conflict("Выберите две разные версии"));
        }
        self.database.with_connection(|c| {
            Ok((
                version(c, homework_id, base)?,
                version(c, homework_id, target)?,
            ))
        })
    }

    /// Сравнить две версии одного ДЗ. Свежий результат переиспользуется.
    pub fn compare(
        &self,
        homework_id: &str,
        base_id: &str,
        target_id: &str,
    ) -> Result<VersionComparisonDto, AppError> {
        let (base, target) = self.pair(homework_id, base_id, target_id)?;
        let (base_files, target_files) = (self.read_files(&base)?, self.read_files(&target)?);
        let (base_fp, target_fp) = (set_fingerprint(&base_files), set_fingerprint(&target_files));
        let existing = self.database.with_connection(|c| {
            Ok(c.query_row(
                &format!("{COMPARISON_SELECT} WHERE base_version_id = ?1 AND target_version_id = ?2 ORDER BY created_at DESC, rowid DESC LIMIT 1"),
                [base_id, target_id],
                comparison_row,
            )
            .optional()?)
        })?;
        if let Some(stored) = existing {
            if !is_stale(&stored, &base_fp, &target_fp) {
                return Ok(stored);
            }
        }
        let body = diff::compare_sets(&base_files, &target_files);
        let id = Uuid::new_v4().to_string();
        self.database.with_transaction(|tx| {
            tx.execute(
                "INSERT INTO version_comparisons(id, homework_id, base_version_id, target_version_id,
                    base_fingerprint, target_fingerprint, extractor_version, status, summary,
                    changes_json, warnings_json, manual_checks_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    homework_id,
                    base_id,
                    target_id,
                    base_fp,
                    target_fp,
                    EXTRACTOR_VERSION,
                    body.status().as_db(),
                    body.summary(),
                    serde_json::to_string(&body.changes).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&body.warnings).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&body.manual_checks).unwrap_or_else(|_| "[]".into()),
                ],
            )?;
            Ok(())
        })?;
        self.comparison(&id)
    }

    /// Сохранённое сравнение с пересчитанным признаком устаревания.
    pub fn comparison(&self, id: &str) -> Result<VersionComparisonDto, AppError> {
        let mut stored = self
            .database
            .with_connection(|c| {
                Ok(c.query_row(
                    &format!("{COMPARISON_SELECT} WHERE id = ?1"),
                    [id],
                    comparison_row,
                )
                .optional()?)
            })?
            .ok_or_else(|| BackendError::not_found("Сравнение не найдено"))?;
        let (base, target) = self.pair(
            &stored.homework_id,
            &stored.base_version_id,
            &stored.target_version_id,
        )?;
        let base_fp = set_fingerprint(&self.read_files(&base)?);
        let target_fp = set_fingerprint(&self.read_files(&target)?);
        stored.stale = is_stale(&stored, &base_fp, &target_fp);
        Ok(stored)
    }

    // ── План исправлений ──

    fn plan_stale(
        connection: &Connection,
        plan: &CorrectionPlanDto,
        requirements_fp: &str,
    ) -> Result<bool, AppError> {
        let snapshot = repository::load_snapshot(connection, &plan.homework_id)?;
        let current = current_review(&snapshot, &plan.version_id).map(|r| r.id);
        Ok(
            snapshot.homework.final_version_id.as_deref() != Some(plan.version_id.as_str())
                || current != plan.review_run_id
                || fingerprint::current(connection, &plan.homework_id)?.requirements
                    != requirements_fp,
        )
    }

    pub fn current_plan(&self, homework_id: &str) -> Result<Option<CorrectionPlanDto>, AppError> {
        self.database.with_connection(|c| {
            let row = c
                .query_row(
                    "SELECT id, homework_id, version_id, review_run_id, created_at, requirements_fingerprint
                       FROM correction_plans WHERE homework_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                    [homework_id],
                    |r| {
                        Ok((
                            CorrectionPlanDto {
                                id: r.get(0)?,
                                homework_id: r.get(1)?,
                                version_id: r.get(2)?,
                                review_run_id: r.get(3)?,
                                created_at: r.get(4)?,
                                items: Vec::new(),
                                stale: false,
                            },
                            r.get::<_, String>(5)?,
                        ))
                    },
                )
                .optional()?;
            let Some((mut plan, requirements_fp)) = row else { return Ok(None) };
            let mut statement = c.prepare(&format!("{ITEM_SELECT} WHERE plan_id = ?1 ORDER BY position"))?;
            plan.items = statement
                .query_map([&plan.id], item_row)?
                .collect::<Result<_, _>>()?;
            plan.stale = Self::plan_stale(c, &plan, &requirements_fp)?;
            Ok(Some(plan))
        })
    }

    /// Построить план для итоговой версии по текущей проверке и требованиям.
    pub fn build_plan(&self, homework_id: &str) -> Result<CorrectionPlanDto, AppError> {
        self.database.with_transaction(|tx| {
            let snapshot = repository::load_snapshot(tx, homework_id)?;
            if snapshot.homework.stage == HomeworkStage::Submitted {
                return Err(BackendError::immutable("Отправленная работа неизменяема"));
            }
            let version_id = snapshot
                .homework
                .final_version_id
                .clone()
                .ok_or_else(|| BackendError::blocked("Сначала выберите итоговую версию"))?;
            let (review_run_id, items) = plan_items(&snapshot, &version_id);
            if items.is_empty() {
                return Err(BackendError::blocked(
                    "Нет замечаний текущей проверки и невыполненных требований — план не нужен",
                ));
            }
            let id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO correction_plans(id, homework_id, version_id, review_run_id, requirements_fingerprint)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, homework_id, version_id, review_run_id, snapshot.inputs.requirements],
            )?;
            for (position, item) in items.iter().enumerate() {
                tx.execute(
                    "INSERT INTO correction_items(id, plan_id, position, category, issue, source_requirement,
                        evidence, file, location, expected_change, acceptance_criterion, priority, origin,
                        status, linked_review_run_id, linked_comment_id, linked_requirement_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                    params![
                        Uuid::new_v4().to_string(),
                        id,
                        position as i64 + 1,
                        item.category,
                        diff::fragment(&item.issue),
                        item.source_requirement,
                        item.evidence,
                        item.file,
                        item.location,
                        item.expected_change,
                        item.acceptance_criterion,
                        item.priority,
                        item.origin,
                        item.status.as_db(),
                        item.linked_review_run_id,
                        item.linked_comment_id,
                        item.linked_requirement_id,
                    ],
                )?;
            }
            Ok(())
        })?;
        self.current_plan(homework_id)?
            .ok_or_else(|| BackendError::not_found("План не найден"))
    }

    /// Пользователь меняет статус пункта: подтверждает предложение, отклоняет его
    /// или связывает исправление с новой версией.
    pub fn set_item_status(
        &self,
        item_id: &str,
        status: CorrectionStatus,
        resolved_in_version_id: Option<&str>,
    ) -> Result<CorrectionPlanDto, AppError> {
        let homework_id = self.database.with_transaction(|tx| {
            let (homework_id, plan_version, current): (String, String, String) = tx
                .query_row(
                    "SELECT p.homework_id, p.version_id, i.status FROM correction_items i
                       JOIN correction_plans p ON p.id = i.plan_id WHERE i.id = ?1",
                    [item_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?
                .ok_or_else(|| BackendError::not_found("Пункт плана не найден"))?;
            if repository::homework_core(tx, &homework_id)?.stage == HomeworkStage::Submitted {
                return Err(BackendError::immutable("Отправленная работа неизменяема"));
            }
            let current = CorrectionStatus::from_db(&current);
            let resolved = match status {
                CorrectionStatus::Proposed => {
                    return Err(BackendError::conflict("Пункт нельзя вернуть в предложения"))
                }
                CorrectionStatus::Open | CorrectionStatus::Dismissed => None,
                CorrectionStatus::Resolved => {
                    if current == CorrectionStatus::Proposed {
                        return Err(BackendError::blocked(
                            "Сначала подтвердите предложенный пункт",
                        ));
                    }
                    let fixed = resolved_in_version_id.ok_or_else(|| {
                        BackendError::conflict("Укажите версию, в которой исправлено")
                    })?;
                    let versions = lineage(tx, &homework_id, fixed)?;
                    if !versions.iter().any(|v| v.id == plan_version) || fixed == plan_version {
                        return Err(BackendError::conflict(
                            "Исправление связывается только с версией-потомком той, для которой строился план",
                        ));
                    }
                    Some(fixed.to_owned())
                }
            };
            tx.execute(
                "UPDATE correction_items SET status = ?2, resolved_in_version_id = ?3,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![item_id, status.as_db(), resolved],
            )?;
            Ok(homework_id)
        })?;
        self.current_plan(&homework_id)?
            .ok_or_else(|| BackendError::not_found("План не найден"))
    }
}
