//! Production workflow, этап 2: подключение (adoption) существующего Obsidian vault.
//!
//! 1. `preview` — только чтение: fd-relative обход `Предметы/`, SHA-256 каждого
//!    файла, сопоставление с базой активного пространства и неизменяемый план в
//!    `vault_adoption_plans`. В vault ничего не пишется.
//! 2. `apply` — повторная проверка plan (sha плана, workspace, fingerprint vault,
//!    TTL), повторное хэширование каждого нового файла и одна SQLite-транзакция.
//!    `dry_run` выполняет те же проверки и ту же транзакцию, затем откатывает её.
//!
//! Идентификаторы импортированных строк стабильны: они выводятся из identity
//! пространства и относительного пути, поэтому повторный preview/apply
//! идемпотентен, а уже импортированные записи помечаются `existing`.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::Database,
    error::{AppError, BackendError, ErrorCode},
    file_service::{PersistentPath, ScanLimits, ScanOutcome, ScannedKind, VaultRoot},
    intake::RootsService,
    workspace,
};

pub const SUBJECTS_ROOT: &str = "Предметы";
const PLAN_TTL_MINUTES: i64 = 30;
const HOMEWORK_FOLDER: &str = "03 ДЗ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdoptionStatus {
    /// Будет создано при применении.
    New,
    /// Уже есть в базе с тем же путём (и тем же SHA-256 для файла).
    Existing,
    /// Файл уже импортирован, но его содержимое изменилось. Не применяется.
    Changed,
    /// Противоречит существующей записи базы. Не применяется.
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileCategory {
    Lecture,
    Seminar,
    Material,
    HomeworkTask,
    HomeworkData,
    HomeworkDraft,
    HomeworkFinal,
}

impl FileCategory {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Lecture => "lecture",
            Self::Seminar => "seminar",
            Self::Material => "material",
            Self::HomeworkTask => "homework_task",
            Self::HomeworkData => "homework_data",
            Self::HomeworkDraft => "homework_draft",
            Self::HomeworkFinal => "homework_final",
        }
    }

    pub fn is_homework(self) -> bool {
        matches!(
            self,
            Self::HomeworkTask | Self::HomeworkData | Self::HomeworkDraft | Self::HomeworkFinal
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// `.md`-заметка Obsidian (обзор, README, карточка) — не учебный файл.
    ObsidianNote,
    /// Служебная папка (`00 Входящие`, `04 Отправлено`, `05 Служебные данные`).
    ServiceFolder,
    /// Файлы на уровне предмета (`00 Общие материалы`): схема требует неделю.
    SubjectLevel,
    /// `Неделя 00`: схема поддерживает номера недель от 1.
    WeekZero,
    /// Вне структуры `Предмет/Неделя NN`.
    OutsideStructure,
    Symlink,
    SpecialFile,
    InvalidName,
    Unreadable,
    /// Родительская запись (предмет, неделя, ДЗ) конфликтует с базой.
    ParentConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedSubject {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub folder_path: String,
    pub status: AdoptionStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedWeek {
    pub id: String,
    pub subject_id: String,
    pub number: i64,
    pub folder_path: String,
    pub status: AdoptionStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedHomework {
    pub id: String,
    pub week_id: String,
    pub title: String,
    pub folder_path: String,
    pub status: AdoptionStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    pub id: String,
    pub relative_path: String,
    pub name: String,
    pub kind: String,
    pub size_bytes: u64,
    pub modified_ns: i128,
    pub sha256: String,
    pub week_id: String,
    /// ДЗ, к которому относится файл из папки `03 ДЗ` (если ДЗ планируется/есть).
    pub homework_id: Option<String>,
    pub category: FileCategory,
    pub confidence: f64,
    pub reasons: Vec<String>,
    pub needs_review: bool,
    pub status: AdoptionStatus,
    pub existing_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedEntry {
    pub relative_path: String,
    pub reason: SkipReason,
    pub detail: String,
}

/// Неизменяемый план: сериализуется в `plan_json`, его SHA-256 сверяется при apply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdoptionPlan {
    pub subjects: Vec<PlannedSubject>,
    pub weeks: Vec<PlannedWeek>,
    pub homeworks: Vec<PlannedHomework>,
    pub files: Vec<PlannedFile>,
    pub skipped: Vec<SkippedEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdoptionSummaryDto {
    pub subjects_new: usize,
    pub subjects_existing: usize,
    pub weeks_new: usize,
    pub weeks_existing: usize,
    pub homeworks_new: usize,
    pub homeworks_existing: usize,
    pub files_new: usize,
    pub files_existing: usize,
    pub files_changed: usize,
    pub conflicts: usize,
    pub skipped: usize,
    pub needs_review: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultAdoptionPreviewDto {
    pub id: String,
    pub workspace_id: String,
    pub created_at: String,
    pub expires_at: String,
    pub summary: AdoptionSummaryDto,
    pub plan: AdoptionPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryOverride {
    pub relative_path: String,
    pub category: FileCategory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Previewed,
    Applied,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdoptionCountsDto {
    pub subjects: usize,
    pub weeks: usize,
    pub homeworks: usize,
    pub files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultAdoptionReportDto {
    pub plan_id: String,
    pub dry_run: bool,
    pub status: PlanStatus,
    pub created: AdoptionCountsDto,
    pub already_present: usize,
    pub changed_not_applied: usize,
    pub conflicts_not_applied: usize,
    pub skipped: usize,
    pub overrides_applied: usize,
}

/// Строки базы, с которыми сопоставляется vault.
#[derive(Debug, Default)]
struct Existing {
    subjects_by_folder: HashMap<String, String>,
    subjects_by_name: HashMap<String, String>,
    weeks_by_folder: HashMap<String, String>,
    weeks_by_subject_number: HashMap<(String, i64), String>,
    homeworks_by_folder: HashMap<String, String>,
    homework_by_week: HashMap<String, String>,
    files_by_path: HashMap<String, Option<String>>,
}

pub struct AdoptionService<'a> {
    database: &'a Database,
    home: Option<PathBuf>,
}

impl<'a> AdoptionService<'a> {
    pub fn new(database: &'a Database, home: Option<PathBuf>) -> Self {
        Self { database, home }
    }

    /// Только чтение vault. В базу пишется лишь неизменяемый план.
    pub fn preview(&self) -> Result<VaultAdoptionPreviewDto, AppError> {
        let (identity, fingerprint) = self.bound_identity()?;
        let vault = RootsService::new(self.database, self.home.clone()).open_vault()?;
        let root = PersistentPath::new(SUBJECTS_ROOT)?;
        let scan = vault.scan_tree(Some(&root), ScanLimits::default())?;
        let existing = self.existing()?;
        let rules = crate::repositories::ReadRepository::new(self.database).automation_rules()?;
        let hints = StudentHints::new(&rules.student_name.last_name);
        let plan = build_plan(&identity.id, &scan, &existing, &hints, |relative| {
            let hashed = vault.hash_file(&PersistentPath::new(relative)?)?;
            Ok((hashed.sha256.as_str().to_owned(), hashed.size_bytes))
        })?;
        let plan_json = serde_json::to_string(&plan)
            .map_err(|error| BackendError::database(error.to_string()))?;
        let plan_sha = hex(&Sha256::digest(plan_json.as_bytes()));
        let id = Uuid::new_v4().to_string();
        let (created_at, expires_at) = self.database.with_transaction(|transaction| {
            transaction.execute(
                "INSERT INTO vault_adoption_plans(
                   id,workspace_id,vault_fingerprint,status,plan_json,plan_sha256,created_at,expires_at)
                 VALUES (?1,?2,?3,'previewed',?4,?5,
                   strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                   strftime('%Y-%m-%dT%H:%M:%fZ','now',?6))",
                params![
                    id,
                    identity.id,
                    fingerprint,
                    plan_json,
                    plan_sha,
                    format!("+{PLAN_TTL_MINUTES} minutes")
                ],
            )?;
            Ok(transaction.query_row(
                "SELECT created_at, expires_at FROM vault_adoption_plans WHERE id=?1",
                [&id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?)
        })?;
        Ok(VaultAdoptionPreviewDto {
            summary: summarize(&plan),
            id,
            workspace_id: identity.id,
            created_at,
            expires_at,
            plan,
        })
    }

    pub fn cancel(&self, plan_id: &str) -> Result<(), AppError> {
        let changed = self.database.with_connection(|connection| {
            Ok(connection.execute(
                "UPDATE vault_adoption_plans
                 SET status='cancelled', finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE id=?1 AND status='previewed'",
                [plan_id],
            )?)
        })?;
        if changed == 1 {
            Ok(())
        } else {
            Err(BackendError::conflict(
                "План уже применён, отменён или не найден",
            ))
        }
    }

    pub fn apply(
        &self,
        plan_id: &str,
        overrides: &[CategoryOverride],
        dry_run: bool,
    ) -> Result<VaultAdoptionReportDto, AppError> {
        let (identity, fingerprint) = self.bound_identity()?;
        let stored = self.load_plan(plan_id)?;
        if stored.status != PlanStatus::Previewed {
            return Err(BackendError::conflict(
                "План уже применён, отменён или завершился ошибкой. Выполните предпросмотр заново.",
            ));
        }
        if !stored.fresh {
            if !dry_run {
                self.finish(plan_id, PlanStatus::Failed, Some("План устарел"), None)?;
            }
            return Err(BackendError::new(
                ErrorCode::ConfirmationRequired,
                "План устарел. Выполните предпросмотр заново.",
            ));
        }
        if stored.workspace_id != identity.id || stored.vault_fingerprint != fingerprint {
            return Err(BackendError::blocked(
                "План создан для другого рабочего пространства или vault",
            ));
        }
        if hex(&Sha256::digest(stored.plan_json.as_bytes())) != stored.plan_sha256 {
            return Err(BackendError::blocked(
                "План повреждён: SHA-256 не совпадает",
            ));
        }
        let plan: AdoptionPlan = serde_json::from_str(&stored.plan_json)
            .map_err(|_| BackendError::database("План повреждён"))?;
        // Ошибка ручных категорий — ошибка ввода: план остаётся доступным.
        let overrides = validate_overrides(&plan, overrides)?;
        let outcome = self.execute(plan_id, &plan, &overrides, dry_run);
        if let Err(error) = &outcome {
            if !dry_run {
                // Изменение vault или базы после предпросмотра завершает план.
                let _ = self.finish(plan_id, PlanStatus::Failed, Some(&error.message), None);
            }
        }
        outcome
    }

    fn execute(
        &self,
        plan_id: &str,
        plan: &AdoptionPlan,
        overrides: &BTreeMap<String, FileCategory>,
        dry_run: bool,
    ) -> Result<VaultAdoptionReportDto, AppError> {
        // Повторная проверка каждого нового файла: путь без symlink, размер и SHA-256.
        let vault = RootsService::new(self.database, self.home.clone()).open_vault()?;
        for file in plan
            .files
            .iter()
            .filter(|f| f.status == AdoptionStatus::New)
        {
            verify_file(&vault, file)?;
        }
        self.database.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let report = insert_plan(&transaction, plan_id, plan, overrides, dry_run)?;
            if dry_run {
                transaction.rollback()?;
            } else {
                let json = serde_json::to_string(&report)
                    .map_err(|error| BackendError::database(error.to_string()))?;
                finish_in(
                    &transaction,
                    plan_id,
                    PlanStatus::Applied,
                    None,
                    Some(&json),
                )?;
                transaction.commit()?;
            }
            Ok(report)
        })
    }

    fn bound_identity(&self) -> Result<(workspace::WorkspaceDto, String), AppError> {
        let identity = workspace::read_identity(self.database)?;
        let fingerprint = identity
            .vault_fingerprint
            .clone()
            .ok_or_else(|| BackendError::blocked("Vault рабочего пространства ещё не привязан"))?;
        Ok((identity, fingerprint))
    }

    fn existing(&self) -> Result<Existing, AppError> {
        self.database.with_connection(|connection| {
            let mut existing = Existing::default();
            for (id, name, folder) in
                query3(connection, "SELECT id,name,folder_path FROM subjects")?
            {
                existing.subjects_by_folder.insert(folder, id.clone());
                existing.subjects_by_name.insert(name, id);
            }
            let mut statement =
                connection.prepare("SELECT id,subject_id,number,folder_path FROM weeks")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                let (id, subject, number, folder) = row?;
                existing.weeks_by_folder.insert(folder, id.clone());
                existing
                    .weeks_by_subject_number
                    .insert((subject, number), id);
            }
            for (id, week, folder) in
                query3(connection, "SELECT id,week_id,folder_path FROM homeworks")?
            {
                existing.homeworks_by_folder.insert(folder, id.clone());
                existing.homework_by_week.insert(week, id);
            }
            let mut statement =
                connection.prepare("SELECT relative_path,content_hash FROM files")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (path, hash) = row?;
                existing.files_by_path.insert(path, hash);
            }
            Ok(existing)
        })
    }

    fn load_plan(&self, plan_id: &str) -> Result<StoredPlan, AppError> {
        self.database.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT workspace_id,vault_fingerprint,status,plan_json,plan_sha256,
                            expires_at > strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     FROM vault_adoption_plans WHERE id=?1",
                    [plan_id],
                    |row| {
                        Ok(StoredPlan {
                            workspace_id: row.get(0)?,
                            vault_fingerprint: row.get(1)?,
                            status: parse_status(&row.get::<_, String>(2)?),
                            plan_json: row.get(3)?,
                            plan_sha256: row.get(4)?,
                            fresh: row.get(5)?,
                        })
                    },
                )
                .optional()?
                .ok_or_else(|| BackendError::not_found("План подключения vault не найден"))
        })
    }

    fn finish(
        &self,
        plan_id: &str,
        status: PlanStatus,
        message: Option<&str>,
        report: Option<&str>,
    ) -> Result<(), AppError> {
        self.database.with_transaction(|transaction| {
            finish_in(transaction, plan_id, status, message, report)
        })
    }
}

struct StoredPlan {
    workspace_id: String,
    vault_fingerprint: String,
    status: PlanStatus,
    plan_json: String,
    plan_sha256: String,
    fresh: bool,
}

fn parse_status(value: &str) -> PlanStatus {
    match value {
        "previewed" => PlanStatus::Previewed,
        "applied" => PlanStatus::Applied,
        "cancelled" => PlanStatus::Cancelled,
        _ => PlanStatus::Failed,
    }
}

fn status_db(status: PlanStatus) -> &'static str {
    match status {
        PlanStatus::Previewed => "previewed",
        PlanStatus::Applied => "applied",
        PlanStatus::Cancelled => "cancelled",
        PlanStatus::Failed => "failed",
    }
}

fn finish_in(
    connection: &Connection,
    plan_id: &str,
    status: PlanStatus,
    message: Option<&str>,
    report: Option<&str>,
) -> Result<(), AppError> {
    connection.execute(
        "UPDATE vault_adoption_plans
         SET status=?2, message=?3, report_json=?4, finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id=?1 AND status='previewed'",
        params![plan_id, status_db(status), message, report],
    )?;
    Ok(())
}

fn query3(connection: &Connection, sql: &str) -> Result<Vec<(String, String, String)>, AppError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn verify_file(vault: &VaultRoot, file: &PlannedFile) -> Result<(), AppError> {
    let changed = || {
        BackendError::conflict(format!(
            "Файл изменился после предпросмотра: {}. Выполните предпросмотр заново.",
            file.relative_path
        ))
    };
    let hashed = vault
        .hash_file(&PersistentPath::new(&file.relative_path)?)
        .map_err(|_| changed())?;
    if hashed.sha256.as_str() != file.sha256 || hashed.size_bytes != file.size_bytes {
        return Err(changed());
    }
    Ok(())
}

fn validate_overrides(
    plan: &AdoptionPlan,
    overrides: &[CategoryOverride],
) -> Result<BTreeMap<String, FileCategory>, AppError> {
    let mut output = BTreeMap::new();
    for item in overrides {
        let file = plan
            .files
            .iter()
            .find(|file| file.relative_path == item.relative_path)
            .ok_or_else(|| {
                BackendError::not_found("Файл для ручной категории не найден в плане")
            })?;
        if file.status != AdoptionStatus::New {
            return Err(BackendError::blocked(
                "Категорию можно задать только новому файлу плана",
            ));
        }
        if item.category.is_homework() && file.homework_id.is_none() {
            return Err(BackendError::blocked(format!(
                "Файл {} не находится в папке ДЗ: категория ДЗ недоступна",
                file.name
            )));
        }
        if output
            .insert(item.relative_path.clone(), item.category)
            .is_some()
        {
            return Err(BackendError::blocked(
                "Повторная ручная категория для файла",
            ));
        }
    }
    Ok(output)
}

fn insert_plan(
    connection: &Connection,
    plan_id: &str,
    plan: &AdoptionPlan,
    overrides: &BTreeMap<String, FileCategory>,
    dry_run: bool,
) -> Result<VaultAdoptionReportDto, AppError> {
    let mut created = AdoptionCountsDto::default();
    let stale = |what: &str| {
        BackendError::conflict(format!(
            "База изменилась после предпросмотра ({what}). Выполните предпросмотр заново."
        ))
    };
    let used_homeworks: BTreeSet<&str> = plan
        .files
        .iter()
        .filter(|file| file.status == AdoptionStatus::New)
        .filter(|file| {
            overrides
                .get(&file.relative_path)
                .copied()
                .unwrap_or(file.category)
                .is_homework()
        })
        .filter_map(|file| file.homework_id.as_deref())
        .collect();

    for subject in plan
        .subjects
        .iter()
        .filter(|s| s.status == AdoptionStatus::New)
    {
        let taken: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM subjects WHERE id=?1 OR name=?2 OR folder_path=?3)",
            params![subject.id, subject.name, subject.folder_path],
            |row| row.get(0),
        )?;
        if taken {
            return Err(stale(&subject.folder_path));
        }
        connection.execute(
            "INSERT INTO subjects(id,name,short_name,folder_path) VALUES (?1,?2,?3,?4)",
            params![
                subject.id,
                subject.name,
                subject.short_name,
                subject.folder_path
            ],
        )?;
        record(
            connection,
            plan_id,
            "subject",
            &subject.id,
            &subject.folder_path,
            None,
        )?;
        created.subjects += 1;
    }
    for week in plan
        .weeks
        .iter()
        .filter(|w| w.status == AdoptionStatus::New)
    {
        let taken: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM weeks WHERE id=?1 OR folder_path=?2
                             OR (subject_id=?3 AND number=?4))",
            params![week.id, week.folder_path, week.subject_id, week.number],
            |row| row.get(0),
        )?;
        if taken {
            return Err(stale(&week.folder_path));
        }
        // Даты недели неизвестны до настройки семестра (этап 5): пустая строка = не задано.
        connection.execute(
            "INSERT INTO weeks(id,subject_id,number,starts_at,ends_at,folder_path)
             VALUES (?1,?2,?3,'','',?4)",
            params![week.id, week.subject_id, week.number, week.folder_path],
        )?;
        record(
            connection,
            plan_id,
            "week",
            &week.id,
            &week.folder_path,
            None,
        )?;
        created.weeks += 1;
    }
    for homework in plan
        .homeworks
        .iter()
        .filter(|h| h.status == AdoptionStatus::New)
    {
        if !used_homeworks.contains(homework.id.as_str()) {
            // Все файлы папки ДЗ вручную переведены в другие категории: ДЗ не создаётся.
            continue;
        }
        let taken: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM homeworks WHERE id=?1 OR folder_path=?2 OR week_id=?3)",
            params![homework.id, homework.folder_path, homework.week_id],
            |row| row.get(0),
        )?;
        if taken {
            return Err(stale(&homework.folder_path));
        }
        // Срок неизвестен: NULL, а не выдуманная дата.
        connection.execute(
            "INSERT INTO homeworks(id,week_id,title,deadline,stage,workflow_stage,folder_path)
             VALUES (?1,?2,?3,NULL,'task','materials',?4)",
            params![
                homework.id,
                homework.week_id,
                homework.title,
                homework.folder_path
            ],
        )?;
        record(
            connection,
            plan_id,
            "homework",
            &homework.id,
            &homework.folder_path,
            None,
        )?;
        created.homeworks += 1;
    }
    let mut overrides_applied = 0;
    for file in plan
        .files
        .iter()
        .filter(|f| f.status == AdoptionStatus::New)
    {
        let category = match overrides.get(&file.relative_path) {
            Some(category) => {
                overrides_applied += 1;
                *category
            }
            None => file.category,
        };
        let homework_id = if category.is_homework() {
            file.homework_id.as_deref()
        } else {
            None
        };
        let taken: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE id=?1 OR relative_path=?2)",
            params![file.id, file.relative_path],
            |row| row.get(0),
        )?;
        if taken {
            return Err(stale(&file.relative_path));
        }
        connection.execute(
            "INSERT INTO files(id,week_id,homework_id,category,name,kind,size_bytes,modified_at,relative_path,content_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                file.id,
                file.week_id,
                homework_id,
                category.as_db(),
                file.name,
                file.kind,
                i64::try_from(file.size_bytes).unwrap_or(i64::MAX),
                iso_from_ns(file.modified_ns),
                file.relative_path,
                file.sha256
            ],
        )?;
        record(
            connection,
            plan_id,
            "file",
            &file.id,
            &file.relative_path,
            Some(&file.sha256),
        )?;
        created.files += 1;
    }
    let count = |status: AdoptionStatus| {
        plan.subjects.iter().filter(|s| s.status == status).count()
            + plan.weeks.iter().filter(|w| w.status == status).count()
            + plan.homeworks.iter().filter(|h| h.status == status).count()
            + plan.files.iter().filter(|f| f.status == status).count()
    };
    Ok(VaultAdoptionReportDto {
        plan_id: plan_id.to_owned(),
        dry_run,
        status: if dry_run {
            PlanStatus::Previewed
        } else {
            PlanStatus::Applied
        },
        created,
        already_present: count(AdoptionStatus::Existing),
        changed_not_applied: count(AdoptionStatus::Changed),
        conflicts_not_applied: count(AdoptionStatus::Conflict),
        skipped: plan.skipped.len(),
        overrides_applied,
    })
}

fn record(
    connection: &Connection,
    plan_id: &str,
    kind: &str,
    entity_id: &str,
    relative_path: &str,
    sha256: Option<&str>,
) -> Result<(), AppError> {
    connection.execute(
        "INSERT INTO vault_adoption_items(plan_id,entity_kind,entity_id,relative_path,content_sha256)
         VALUES (?1,?2,?3,?4,?5)",
        params![plan_id, kind, entity_id, relative_path, sha256],
    )?;
    Ok(())
}

fn iso_from_ns(ns: i128) -> Option<String> {
    let value = time::OffsetDateTime::from_unix_timestamp_nanos(ns).ok()?;
    value
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

pub fn summarize(plan: &AdoptionPlan) -> AdoptionSummaryDto {
    let subjects = |status| plan.subjects.iter().filter(|s| s.status == status).count();
    let weeks = |status| plan.weeks.iter().filter(|w| w.status == status).count();
    let homeworks = |status| plan.homeworks.iter().filter(|h| h.status == status).count();
    let files = |status| plan.files.iter().filter(|f| f.status == status).count();
    AdoptionSummaryDto {
        subjects_new: subjects(AdoptionStatus::New),
        subjects_existing: subjects(AdoptionStatus::Existing),
        weeks_new: weeks(AdoptionStatus::New),
        weeks_existing: weeks(AdoptionStatus::Existing),
        homeworks_new: homeworks(AdoptionStatus::New),
        homeworks_existing: homeworks(AdoptionStatus::Existing),
        files_new: files(AdoptionStatus::New),
        files_existing: files(AdoptionStatus::Existing),
        files_changed: files(AdoptionStatus::Changed),
        conflicts: subjects(AdoptionStatus::Conflict)
            + weeks(AdoptionStatus::Conflict)
            + homeworks(AdoptionStatus::Conflict),
        skipped: plan.skipped.len(),
        needs_review: plan
            .files
            .iter()
            .filter(|f| f.status == AdoptionStatus::New && f.needs_review)
            .count(),
        truncated: plan.truncated,
    }
}

// ----- построение плана (чистая функция над результатом обхода) ------------------------

fn build_plan(
    workspace_id: &str,
    scan: &ScanOutcome,
    existing: &Existing,
    hints: &StudentHints,
    mut hash: impl FnMut(&str) -> Result<(String, u64), AppError>,
) -> Result<AdoptionPlan, AppError> {
    let mut builder = PlanBuilder {
        workspace_id,
        existing,
        hints,
        plan: AdoptionPlan {
            truncated: scan.truncated,
            ..AdoptionPlan::default()
        },
        subjects: BTreeMap::new(),
        weeks: BTreeMap::new(),
        homeworks: BTreeMap::new(),
    };
    for path in &scan.non_utf8 {
        builder.skip(path, SkipReason::InvalidName, "Имя не в UTF-8");
    }
    for path in &scan.unreadable {
        builder.skip(path, SkipReason::Unreadable, "Папка недоступна для чтения");
    }
    // Сначала каталоги: предметы и недели появляются в плане даже без файлов.
    for entry in scan
        .entries
        .iter()
        .filter(|e| e.kind == ScannedKind::Directory)
    {
        builder.directory(&entry.relative);
    }
    for entry in &scan.entries {
        match entry.kind {
            ScannedKind::Directory => {}
            ScannedKind::Symlink => builder.skip(
                &entry.relative,
                SkipReason::Symlink,
                "Символическая ссылка не открывается",
            ),
            ScannedKind::Other => {
                builder.skip(&entry.relative, SkipReason::SpecialFile, "Не обычный файл")
            }
            ScannedKind::File => builder.file(entry, &mut hash)?,
        }
    }
    builder.finish();
    Ok(builder.plan)
}

struct PlanBuilder<'p> {
    workspace_id: &'p str,
    existing: &'p Existing,
    hints: &'p StudentHints,
    plan: AdoptionPlan,
    /// folder_path → индекс в plan.subjects
    subjects: BTreeMap<String, usize>,
    weeks: BTreeMap<String, usize>,
    homeworks: BTreeMap<String, usize>,
}

impl PlanBuilder<'_> {
    fn skip(&mut self, path: &str, reason: SkipReason, detail: &str) {
        self.plan.skipped.push(SkippedEntry {
            relative_path: path.to_owned(),
            reason,
            detail: detail.to_owned(),
        });
    }

    fn directory(&mut self, relative: &str) {
        let parts: Vec<&str> = relative.split('/').collect();
        match parts.as_slice() {
            [_, subject] if !is_service_name(subject) => {
                self.ensure_subject(subject);
            }
            [_, subject, week] if !is_service_name(subject) => {
                if let Some(number) = parse_week(week).filter(|n| *n >= 1) {
                    self.ensure_week(subject, week, number);
                }
            }
            _ => {}
        }
    }

    fn ensure_subject(&mut self, name: &str) -> usize {
        let folder = format!("{SUBJECTS_ROOT}/{name}");
        if let Some(index) = self.subjects.get(&folder) {
            return *index;
        }
        let (id, status, reason) = if let Some(id) = self.existing.subjects_by_folder.get(&folder) {
            (id.clone(), AdoptionStatus::Existing, None)
        } else if let Some(id) = self.existing.subjects_by_name.get(name) {
            (
                id.clone(),
                AdoptionStatus::Conflict,
                Some("Предмет с таким названием уже связан с другой папкой".to_owned()),
            )
        } else {
            (
                stable_id(self.workspace_id, "subject", &folder),
                AdoptionStatus::New,
                None,
            )
        };
        self.plan.subjects.push(PlannedSubject {
            id,
            name: name.to_owned(),
            short_name: short_name(name),
            folder_path: folder.clone(),
            status,
            reason,
        });
        let index = self.plan.subjects.len() - 1;
        self.subjects.insert(folder, index);
        index
    }

    fn ensure_week(&mut self, subject: &str, week_folder: &str, number: i64) -> usize {
        let subject_index = self.ensure_subject(subject);
        let folder = format!("{SUBJECTS_ROOT}/{subject}/{week_folder}");
        if let Some(index) = self.weeks.get(&folder) {
            return *index;
        }
        let parent = &self.plan.subjects[subject_index];
        let subject_id = parent.id.clone();
        let (id, status, reason) = if parent.status == AdoptionStatus::Conflict {
            (
                stable_id(self.workspace_id, "week", &folder),
                AdoptionStatus::Conflict,
                Some("Предмет конфликтует с базой".to_owned()),
            )
        } else if let Some(id) = self.existing.weeks_by_folder.get(&folder) {
            (id.clone(), AdoptionStatus::Existing, None)
        } else if self
            .existing
            .weeks_by_subject_number
            .contains_key(&(subject_id.clone(), number))
        {
            (
                stable_id(self.workspace_id, "week", &folder),
                AdoptionStatus::Conflict,
                Some(format!(
                    "Неделя {number} предмета уже связана с другой папкой"
                )),
            )
        } else {
            (
                stable_id(self.workspace_id, "week", &folder),
                AdoptionStatus::New,
                None,
            )
        };
        self.plan.weeks.push(PlannedWeek {
            id,
            subject_id,
            number,
            folder_path: folder.clone(),
            status,
            reason,
        });
        let index = self.plan.weeks.len() - 1;
        self.weeks.insert(folder, index);
        index
    }

    fn ensure_homework(&mut self, week_index: usize) -> usize {
        let week = &self.plan.weeks[week_index];
        let folder = format!("{}/{HOMEWORK_FOLDER}", week.folder_path);
        if let Some(index) = self.homeworks.get(&folder) {
            return *index;
        }
        let week_id = week.id.clone();
        let title = format!("Домашнее задание — неделя {}", week.number);
        let (id, status, reason) = if week.status == AdoptionStatus::Conflict {
            (
                stable_id(self.workspace_id, "homework", &folder),
                AdoptionStatus::Conflict,
                Some("Неделя конфликтует с базой".to_owned()),
            )
        } else if let Some(id) = self.existing.homeworks_by_folder.get(&folder) {
            (id.clone(), AdoptionStatus::Existing, None)
        } else if self.existing.homework_by_week.contains_key(&week_id) {
            (
                stable_id(self.workspace_id, "homework", &folder),
                AdoptionStatus::Conflict,
                Some("У недели уже есть ДЗ в другой папке".to_owned()),
            )
        } else {
            (
                stable_id(self.workspace_id, "homework", &folder),
                AdoptionStatus::New,
                None,
            )
        };
        self.plan.homeworks.push(PlannedHomework {
            id,
            week_id,
            title,
            folder_path: folder.clone(),
            status,
            reason,
        });
        let index = self.plan.homeworks.len() - 1;
        self.homeworks.insert(folder, index);
        index
    }

    fn file(
        &mut self,
        entry: &crate::file_service::ScannedEntry,
        hash: &mut impl FnMut(&str) -> Result<(String, u64), AppError>,
    ) -> Result<(), AppError> {
        let relative = entry.relative.as_str();
        let parts: Vec<&str> = relative.split('/').collect();
        let name = *parts.last().unwrap_or(&"");
        if PersistentPath::new(relative).is_err() {
            self.skip(relative, SkipReason::InvalidName, "Недопустимый путь");
            return Ok(());
        }
        // parts[0] = «Предметы»
        let (subject, week_folder, rest) = match parts.as_slice() {
            [_, _] => {
                self.skip(
                    relative,
                    SkipReason::OutsideStructure,
                    "Файл вне папки предмета",
                );
                return Ok(());
            }
            [_, subject, ..] if is_service_name(subject) => {
                self.skip(relative, SkipReason::ServiceFolder, "Служебная папка");
                return Ok(());
            }
            [_, _, _] if is_note(name) => {
                self.skip(relative, SkipReason::ObsidianNote, "Заметка Obsidian");
                return Ok(());
            }
            [_, _, _] => {
                self.skip(
                    relative,
                    SkipReason::OutsideStructure,
                    "Файл на уровне предмета",
                );
                return Ok(());
            }
            [_, subject, week, rest @ ..] => (*subject, *week, rest),
            _ => return Ok(()),
        };
        let number = match parse_week(week_folder) {
            Some(0) => {
                self.skip(
                    relative,
                    SkipReason::WeekZero,
                    "«Неделя 00» пока не поддерживается: номер недели должен быть от 1",
                );
                return Ok(());
            }
            Some(number) => number,
            None if week_folder.starts_with("00 ") => {
                self.skip(
                    relative,
                    SkipReason::SubjectLevel,
                    "Общие материалы предмета пока не поддерживаются: файл должен относиться к неделе",
                );
                return Ok(());
            }
            None => {
                self.skip(
                    relative,
                    SkipReason::OutsideStructure,
                    "Папка вне структуры недель",
                );
                return Ok(());
            }
        };
        if is_note(name) {
            self.skip(
                relative,
                SkipReason::ObsidianNote,
                "Служебная заметка Obsidian",
            );
            return Ok(());
        }
        let Some(classified) = classify(rest, name, self.hints) else {
            self.skip(
                relative,
                SkipReason::ServiceFolder,
                "Служебная папка «Контура» (снимки сдачи и служебные данные не импортируются)",
            );
            return Ok(());
        };
        let week_index = self.ensure_week(subject, week_folder, number);
        let week_status = self.plan.weeks[week_index].status;
        if week_status == AdoptionStatus::Conflict {
            self.skip(
                relative,
                SkipReason::ParentConflict,
                "Неделя конфликтует с базой",
            );
            return Ok(());
        }
        let week_id = self.plan.weeks[week_index].id.clone();
        let homework_id = if rest.first() == Some(&HOMEWORK_FOLDER) {
            let index = self.ensure_homework(week_index);
            if self.plan.homeworks[index].status == AdoptionStatus::Conflict {
                self.skip(
                    relative,
                    SkipReason::ParentConflict,
                    "ДЗ недели конфликтует с базой",
                );
                return Ok(());
            }
            Some(self.plan.homeworks[index].id.clone())
        } else {
            None
        };
        let (sha256, size_bytes) = hash(relative)?;
        let (status, existing_sha256) = match self.existing.files_by_path.get(relative) {
            None => (AdoptionStatus::New, None),
            Some(Some(previous)) if *previous == sha256 => {
                (AdoptionStatus::Existing, Some(previous.clone()))
            }
            Some(previous) => (AdoptionStatus::Changed, previous.clone()),
        };
        self.plan.files.push(PlannedFile {
            id: stable_id(self.workspace_id, "file", relative),
            relative_path: relative.to_owned(),
            name: name.to_owned(),
            kind: extension(name),
            size_bytes,
            modified_ns: entry.modified_ns,
            sha256,
            week_id,
            homework_id,
            category: classified.category,
            confidence: classified.confidence,
            needs_review: classified.confidence < 0.7,
            reasons: classified.reasons,
            status,
            existing_sha256,
        });
        Ok(())
    }

    /// ДЗ без единого файла в плане не создаётся (пустая папка `03 ДЗ` с README).
    fn finish(&mut self) {
        let used: BTreeSet<String> = self
            .plan
            .files
            .iter()
            .filter_map(|file| file.homework_id.clone())
            .collect();
        self.plan.homeworks.retain(|homework| {
            homework.status != AdoptionStatus::New || used.contains(&homework.id)
        });
    }
}

struct Classified {
    category: FileCategory,
    confidence: f64,
    reasons: Vec<String>,
}

/// Категория по папке внутри недели и имени файла. `None` — служебная папка.
fn classify(rest: &[&str], name: &str, hints: &StudentHints) -> Option<Classified> {
    let folder = if rest.len() > 1 { rest[0] } else { "" };
    let by_folder = |category, reason: &str| Classified {
        category,
        confidence: 0.95,
        reasons: vec![reason.to_owned()],
    };
    match folder {
        "01 Лекции" => Some(by_folder(FileCategory::Lecture, "Папка «01 Лекции»")),
        "02 Семинары" => Some(by_folder(FileCategory::Seminar, "Папка «02 Семинары»")),
        "04 Дополнительно" => Some(by_folder(
            FileCategory::Material,
            "Папка «04 Дополнительно»",
        )),
        HOMEWORK_FOLDER => classify_homework(&rest[1..], name, hints),
        "" => Some(Classified {
            category: FileCategory::Material,
            confidence: 0.5,
            reasons: vec!["Файл в корне недели: категория не определена".to_owned()],
        }),
        other => Some(Classified {
            category: FileCategory::Material,
            confidence: 0.5,
            reasons: vec![format!("Неизвестная папка «{other}»")],
        }),
    }
}

fn classify_homework(rest: &[&str], name: &str, hints: &StudentHints) -> Option<Classified> {
    let lower = name.to_lowercase();
    let ext = extension(name);
    let subfolder = if rest.len() > 1 { rest[0] } else { "" };
    let fixed = |category, reason: &str| Classified {
        category,
        confidence: 0.9,
        reasons: vec![reason.to_owned()],
    };
    match subfolder {
        "01 Условие и данные" => {
            return Some(if is_table(&ext) {
                fixed(
                    FileCategory::HomeworkData,
                    "Таблица в «01 Условие и данные»",
                )
            } else {
                fixed(FileCategory::HomeworkTask, "Файл в «01 Условие и данные»")
            })
        }
        "02 Работа" => return Some(fixed(FileCategory::HomeworkDraft, "Папка «02 Работа»")),
        "03 На сдачу" => {
            return Some(fixed(FileCategory::HomeworkFinal, "Папка «03 На сдачу»"))
        }
        "04 Отправлено" | "05 Служебные данные" => return None,
        _ => {}
    }
    let contains_any = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    let mut reasons = Vec::new();
    if !subfolder.is_empty() {
        reasons.push(format!("Вложенная папка «{subfolder}»"));
    }
    let task_words = [
        "дз",
        "задани",
        "памятка",
        "условие",
        "homework",
        "task",
        "hw",
    ];
    let (category, confidence, reason) =
        if contains_any(&["приложение", "данные", "data", "dataset"]) {
            (
                FileCategory::HomeworkData,
                0.8,
                "Имя похоже на данные к заданию",
            )
        } else if hints.matches(&lower) {
            (
                FileCategory::HomeworkDraft,
                0.9,
                "В имени ваша фамилия из настроек — это ваша работа",
            )
        } else if has_lms_suffix(name) && matches!(ext.as_str(), "pdf" | "docx" | "doc" | "pptx") {
            (
                FileCategory::HomeworkTask,
                0.9,
                "Файл из LMS (хэш-суффикс в имени)",
            )
        } else if contains_any(&[
            "решени",
            "solution",
            "финал",
            "исправл",
            "черновик",
            "draft",
            "final",
            "кейс",
        ]) || has_version_suffix(name)
            || looks_like_personal_name(name)
            || ext == "tex"
        {
            (
                FileCategory::HomeworkDraft,
                0.75,
                "Имя похоже на работу студента (версия, решение, черновик)",
            )
        } else if ext == "pdf" && contains_any(&task_words) {
            (FileCategory::HomeworkTask, 0.75, "PDF с условием задания")
        } else if matches!(ext.as_str(), "docx" | "doc" | "pptx") && contains_any(&task_words) {
            // Без LMS-суффикса документ с «ДЗ» в имени часто оказывается собственным текстом.
            (
                FileCategory::HomeworkTask,
                0.6,
                "Документ упоминает ДЗ, но не из LMS: проверьте, условие это или ваша работа",
            )
        } else if is_table(&ext) || is_image(&ext) || ext == "ipynb" {
            (
                FileCategory::HomeworkDraft,
                0.5,
                "Тип файла похож на работу, проверьте категорию",
            )
        } else {
            (
                FileCategory::HomeworkTask,
                0.5,
                "Категория не определена, проверьте вручную",
            )
        };
    reasons.push(reason.to_owned());
    if !subfolder.is_empty() && confidence >= 0.7 {
        // Вложенные папки внутри ДЗ всегда требуют подтверждения.
        return Some(Classified {
            category,
            confidence: 0.6,
            reasons,
        });
    }
    Some(Classified {
        category,
        confidence,
        reasons,
    })
}

/// Фамилия студента из настроек (кириллица и латиница) для распознавания своих работ.
#[derive(Debug, Default)]
pub(crate) struct StudentHints {
    needles: Vec<String>,
}

impl StudentHints {
    pub(crate) fn new(last_name: &str) -> Self {
        let last = last_name.trim().to_lowercase();
        if last.chars().count() < 3 || last == "не задано" {
            return Self::default();
        }
        let latin = transliterate(&last);
        let mut needles = vec![last];
        if latin.chars().count() >= 3 && !needles.contains(&latin) {
            needles.push(latin);
        }
        Self { needles }
    }

    fn matches(&self, lower_name: &str) -> bool {
        self.needles
            .iter()
            .any(|needle| lower_name.contains(needle.as_str()))
    }
}

fn transliterate(text: &str) -> String {
    let mut output = String::new();
    for c in text.chars() {
        let mapped = match c {
            'а' => "a",
            'б' => "b",
            'в' => "v",
            'г' => "g",
            'д' => "d",
            'е' | 'ё' | 'э' => "e",
            'ж' => "zh",
            'з' => "z",
            'и' | 'й' | 'ы' => "i",
            'к' => "k",
            'л' => "l",
            'м' => "m",
            'н' => "n",
            'о' => "o",
            'п' => "p",
            'р' => "r",
            'с' => "s",
            'т' => "t",
            'у' => "u",
            'ф' => "f",
            'х' => "kh",
            'ц' => "ts",
            'ч' => "ch",
            'ш' => "sh",
            'щ' => "shch",
            'ю' => "yu",
            'я' => "ya",
            _ if c.is_ascii_alphanumeric() => {
                output.push(c);
                continue;
            }
            _ => "",
        };
        output.push_str(mapped);
    }
    output
}

/// Файлы, скачанные из LMS, имеют суффикс `_<8 hex>` перед расширением.
fn has_lms_suffix(name: &str) -> bool {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    stem.rsplit_once('_').is_some_and(|(_, tail)| {
        tail.len() == 8
            && tail
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            && tail.chars().any(|c| c.is_ascii_digit())
    })
}

/// `…_v01`, `… v2`, `… — v03` в конце имени — версия собственной работы.
fn has_version_suffix(name: &str) -> bool {
    let stem = name
        .rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_lowercase();
    let Some(position) = stem.rfind('v') else {
        return false;
    };
    let digits = &stem[position + 1..];
    let before = stem[..position].chars().last();
    (1..=3).contains(&digits.len())
        && digits.chars().all(|c| c.is_ascii_digit())
        && before.is_some_and(|c| c == '_' || c == ' ' || c == '-')
}

/// `Фамилия_Имя_…` — типичное имя файла студенческой работы.
fn looks_like_personal_name(name: &str) -> bool {
    let mut parts = name.split('_');
    let capitalized = |part: Option<&str>| {
        part.and_then(|value| value.chars().next())
            .is_some_and(|first| first.is_uppercase() && first.is_alphabetic())
            && part.is_some_and(|value| {
                value.chars().count() >= 2 && value.chars().all(char::is_alphabetic)
            })
    };
    capitalized(parts.next()) && capitalized(parts.next())
}

fn is_table(ext: &str) -> bool {
    matches!(ext, "xlsx" | "xlsm" | "xls" | "csv" | "ods")
}

fn is_image(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "heic" | "webp")
}

fn is_note(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".md")
        && (lower == "readme.md"
            || name.starts_with("00 ")
            || name == "Обзор недели.md"
            || name == "Карточка ДЗ.md")
}

fn is_service_name(name: &str) -> bool {
    name.starts_with("00 ")
}

fn parse_week(name: &str) -> Option<i64> {
    let digits = name.strip_prefix("Неделя ")?.trim();
    if digits.is_empty() || digits.len() > 2 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty() && ext.len() <= 8)
        .map(|(_, ext)| ext.to_lowercase())
        .unwrap_or_else(|| "file".to_owned())
}

/// «Основы бизнес-аналитики» → «ОБА»; одно слово — первые четыре буквы.
fn short_name(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '-' || c == ',')
        .filter(|word| !word.is_empty())
        .collect();
    if words.len() <= 1 {
        let chars: Vec<char> = name.chars().collect();
        return if chars.len() <= 5 {
            name.to_owned()
        } else {
            chars[..4].iter().collect()
        };
    }
    words
        .iter()
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Детерминированный UUID-формат из identity пространства и относительного пути.
pub fn stable_id(workspace_id: &str, kind: &str, path: &str) -> String {
    let digest = Sha256::digest(format!("kontur:adopt:v1:{workspace_id}:{kind}:{path}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80; // версия 8: пользовательский формат (RFC 9562)
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[path = "adoption_tests.rs"]
mod tests;
