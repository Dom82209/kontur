//! Production workflow, этап 1: граница рабочих пространств (workspace).
//!
//! Каждое рабочее пространство владеет отдельной SQLite. Глобальный каталог
//! (`workspace-catalog.sqlite3`) хранит только identity, тип, fingerprint,
//! относительный путь к базе workspace, активный выбор и журнал переходов.
//! Учебные строки (предметы, ДЗ, AI jobs, packages, receipts) физически не могут
//! попасть в чужое пространство: SQL всегда выполняется в одной базе.
//!
//! Переход между пространствами выполняется в два шага:
//! 1. `preview_transition` — проверяет vault и записывает только строку журнала в
//!    каталог (app data). Ни vault, ни базы workspace не изменяются.
//! 2. `confirm_transition` — повторно валидирует vault (canonical path, dev/ino,
//!    содержимое marker), применяет изменения и сохраняет активный выбор.
//!    Процесс перезапускается, поэтому база никогда не подменяется под
//!    выполняющейся командой.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    db::Database,
    error::{AppError, BackendError, ErrorCode},
    file_service::VaultRoot,
    intake::ManagedRootsConfig,
};

const CATALOG_FILE: &str = "workspace-catalog.sqlite3";
const CATALOG_SCHEMA_VERSION: i64 = 1;
const LEGACY_DATABASE: &str = "kontur.sqlite3";
const LEGACY_DEMO_DIR: &str = "demo-workspace";
const DEMO_DIR: &str = "demo-workspaces";
pub const MARKER_FILE: &str = ".kontur-workspace.json";
const ROOTS_KEY: &str = "managed_roots";
const MARKER_MAX_BYTES: u64 = 4096;
/// Preview перехода действует ограниченное время; после — только новый preview.
const PREVIEW_TTL_MINUTES: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    Demo,
    Real,
}

impl WorkspaceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Real => "real",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    Active,
    Archived,
}

impl WorkspaceState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDto {
    pub id: String,
    pub kind: WorkspaceKind,
    pub vault_canonical: Option<String>,
    pub vault_fingerprint: Option<String>,
    pub created_at: String,
    pub state: WorkspaceState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceActivationDto {
    pub workspace: WorkspaceDto,
    /// Всегда `true`: новая база открывается только после перезапуска процесса.
    pub restart_required: bool,
}

/// Тип перехода. Каждый тип проходит preview → confirm/cancel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransitionRequest {
    /// Новое реальное пространство для vault, который ещё не зарегистрирован.
    CreateReal,
    /// Повторное подключение ранее зарегистрированного vault (по marker/fingerprint).
    ConnectExisting,
    /// Новое демо-пространство целиком внутри каталога данных приложения.
    CreateDemo,
    /// Переключение на пространство из каталога (после повторной проверки vault).
    Activate {
        #[serde(rename = "workspaceId")]
        workspace_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionAction {
    CreateReal,
    ConnectExisting,
    CreateDemo,
    Activate,
}

impl TransitionAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::CreateReal => "create_real",
            Self::ConnectExisting => "connect_existing",
            Self::CreateDemo => "create_demo",
            Self::Activate => "activate",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "create_real" => Self::CreateReal,
            "connect_existing" => Self::ConnectExisting,
            "create_demo" => Self::CreateDemo,
            "activate" => Self::Activate,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionState {
    Previewed,
    Applying,
    Applied,
    Cancelled,
    Failed,
}

impl TransitionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Previewed => "previewed",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "previewed" => Self::Previewed,
            "applying" => Self::Applying,
            "applied" => Self::Applied,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

/// Что именно произойдёт при подтверждении. Показывается пользователю дословно.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTransitionPreviewDto {
    pub id: String,
    pub action: TransitionAction,
    pub current_workspace: Option<WorkspaceDto>,
    /// Пространство, которое станет активным (для create — ещё не существует).
    pub target_workspace: Option<WorkspaceDto>,
    pub target_kind: WorkspaceKind,
    pub vault_display_path: Option<String>,
    /// Файлы и записи, которые будут созданы/изменены после подтверждения.
    pub effects: Vec<String>,
    pub warnings: Vec<String>,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTransitionDto {
    pub id: String,
    pub action: TransitionAction,
    pub state: TransitionState,
    pub from_workspace_id: Option<String>,
    pub target_workspace_id: Option<String>,
    pub message: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOverviewDto {
    pub active: Option<WorkspaceDto>,
    pub workspaces: Vec<WorkspaceDto>,
    pub recent_transitions: Vec<WorkspaceTransitionDto>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VaultMarker {
    workspace_id: String,
    nonce: String,
}

/// Снимок vault на момент preview: повторно сверяется перед apply.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultSnapshot {
    canonical: PathBuf,
    dev: u64,
    ino: u64,
    /// SHA-256 содержимого marker или `absent`.
    marker_state: String,
}

pub struct WorkspaceCatalog {
    app_data_dir: PathBuf,
    home: Option<PathBuf>,
    connection: Mutex<Connection>,
}

impl WorkspaceCatalog {
    /// Открывает (или создаёт) каталог и возвращает путь базы активного workspace.
    pub fn bootstrap(
        app_data_dir: &Path,
        home: Option<&Path>,
    ) -> Result<(Self, PathBuf), AppError> {
        fs::create_dir_all(app_data_dir.join("workspaces"))?;
        let connection = Connection::open(app_data_dir.join(CATALOG_FILE))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate_catalog(&connection)?;
        let catalog = Self {
            app_data_dir: app_data_dir.to_path_buf(),
            home: home.map(Path::to_path_buf),
            connection: Mutex::new(connection),
        };
        catalog.fail_interrupted_transitions()?;
        if catalog.workspace_count()? == 0 {
            catalog.import_legacy()?;
        }
        let active = catalog.active_record()?;
        let path = catalog.database_path(&active.database_relpath)?;
        Ok((catalog, path))
    }

    /// Проверяет, что открытая база принадлежит активной записи каталога.
    /// Несовпадение — повреждение или подмена файла: работа блокируется.
    pub fn verify_active_database(&self, database: &Database) -> Result<WorkspaceDto, AppError> {
        let record = self.active_record()?;
        let identity = read_identity(database)?;
        if identity.id != record.id || identity.kind != record.kind {
            return Err(BackendError::blocked(
                "База рабочего пространства не совпадает с каталогом. Работа остановлена, чтобы не смешать данные.",
            ));
        }
        match (&identity.vault_fingerprint, &record.vault_fingerprint) {
            (None, None) => {
                // Legacy-пространство, vault которого был недоступен при импорте:
                // привязываем его при первом успешном доступе к прежнему пути.
                self.bind_legacy_vault(database, &record)?;
            }
            (Some(_), None) => {
                // База уже привязана, а запись каталога — нет (прерванная привязка):
                // каталог восстанавливается из неизменяемой identity базы.
                self.lock()?.execute(
                    "UPDATE workspaces SET vault_canonical=?2, vault_fingerprint=?3
                     WHERE id=?1 AND vault_fingerprint IS NULL",
                    params![
                        record.id,
                        identity.vault_canonical,
                        identity.vault_fingerprint
                    ],
                )?;
            }
            (identity_fp, record_fp) if identity_fp != record_fp => {
                return Err(BackendError::blocked(
                    "Fingerprint vault в базе не совпадает с каталогом. Работа остановлена, чтобы не смешать данные.",
                ));
            }
            _ => {}
        }
        read_identity(database)
    }

    pub fn overview(&self) -> Result<WorkspaceOverviewDto, AppError> {
        let workspaces = self.list()?;
        let active = self
            .active_record()
            .ok()
            .and_then(|record| workspaces.iter().find(|item| item.id == record.id).cloned());
        Ok(WorkspaceOverviewDto {
            active,
            workspaces,
            recent_transitions: self.recent_transitions(20)?,
        })
    }

    pub fn list(&self) -> Result<Vec<WorkspaceDto>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id,kind,vault_canonical,vault_fingerprint,created_at,state
             FROM workspaces ORDER BY created_at,id",
        )?;
        let rows = statement.query_map([], row_workspace)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Шаг 1. Ничего не пишет ни в vault, ни в базы workspace — только журнал каталога.
    pub fn preview_transition(
        &self,
        request: &TransitionRequest,
        vault: Option<&Path>,
    ) -> Result<WorkspaceTransitionPreviewDto, AppError> {
        let current = self.active_workspace()?;
        let (action, plan) = match request {
            TransitionRequest::CreateReal => {
                let vault = vault.ok_or_else(|| BackendError::blocked("Vault не выбран"))?;
                (TransitionAction::CreateReal, self.plan_create_real(vault)?)
            }
            TransitionRequest::ConnectExisting => {
                let vault = vault.ok_or_else(|| BackendError::blocked("Vault не выбран"))?;
                (TransitionAction::ConnectExisting, self.plan_connect(vault)?)
            }
            TransitionRequest::CreateDemo => (TransitionAction::CreateDemo, self.plan_demo()),
            TransitionRequest::Activate { workspace_id } => (
                TransitionAction::Activate,
                self.plan_activate(workspace_id)?,
            ),
        };
        if let (Some(current), Some(target)) = (&current, &plan.target) {
            if current.id == target.id {
                return Err(BackendError::conflict(
                    "Это рабочее пространство уже активно",
                ));
            }
        }
        let id = Uuid::new_v4().to_string();
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO workspace_transitions(
               id,action,vault_canonical,vault_dev,vault_ino,marker_state,
               from_workspace_id,target_workspace_id,state,created_at,expires_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'previewed',
               strftime('%Y-%m-%dT%H:%M:%fZ','now'),
               strftime('%Y-%m-%dT%H:%M:%fZ','now',?9))",
            params![
                id,
                action.as_str(),
                plan.snapshot
                    .as_ref()
                    .map(|s| s.canonical.to_string_lossy().into_owned()),
                plan.snapshot.as_ref().map(|s| s.dev as i64),
                plan.snapshot.as_ref().map(|s| s.ino as i64),
                plan.snapshot.as_ref().map(|s| s.marker_state.clone()),
                current.as_ref().map(|w| w.id.clone()),
                plan.target.as_ref().map(|w| w.id.clone()),
                format!("+{PREVIEW_TTL_MINUTES} minutes"),
            ],
        )?;
        let expires_at: String = connection.query_row(
            "SELECT expires_at FROM workspace_transitions WHERE id=?1",
            [&id],
            |row| row.get(0),
        )?;
        drop(connection);
        Ok(WorkspaceTransitionPreviewDto {
            id,
            action,
            current_workspace: current,
            target_workspace: plan.target,
            target_kind: plan.target_kind,
            vault_display_path: plan.snapshot.as_ref().map(|s| self.display(&s.canonical)),
            effects: plan.effects,
            warnings: plan.warnings,
            expires_at,
        })
    }

    /// Шаг 2. Повторная валидация и применение. Возвращает новое активное пространство.
    pub fn confirm_transition(
        &self,
        transition_id: &str,
    ) -> Result<WorkspaceActivationDto, AppError> {
        let pending = self.claim_transition(transition_id)?;
        let result = self.apply_transition(&pending);
        match &result {
            Ok(activation) => self.finish_transition(
                transition_id,
                TransitionState::Applied,
                Some(&activation.workspace.id),
                None,
            )?,
            Err(error) => self.finish_transition(
                transition_id,
                TransitionState::Failed,
                None,
                Some(&error.message),
            )?,
        }
        result
    }

    pub fn cancel_transition(&self, transition_id: &str) -> Result<(), AppError> {
        let changed = self.lock()?.execute(
            "UPDATE workspace_transitions
             SET state='cancelled', finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id=?1 AND state='previewed'",
            [transition_id],
        )?;
        if changed == 1 {
            Ok(())
        } else {
            Err(BackendError::conflict(
                "Переход уже применён, отменён или не найден",
            ))
        }
    }

    fn apply_transition(
        &self,
        pending: &PendingTransition,
    ) -> Result<WorkspaceActivationDto, AppError> {
        match pending.action {
            TransitionAction::CreateReal => {
                let snapshot = self.revalidate_snapshot(pending)?;
                self.create_real(&snapshot)
            }
            TransitionAction::ConnectExisting => {
                let snapshot = self.revalidate_snapshot(pending)?;
                self.connect_existing(&snapshot, pending.target_workspace_id.as_deref())
            }
            TransitionAction::CreateDemo => self.create_demo(),
            TransitionAction::Activate => {
                let id = pending
                    .target_workspace_id
                    .as_deref()
                    .ok_or_else(|| BackendError::database("Переход без целевого пространства"))?;
                self.activate_existing(id)
            }
        }
    }

    // ----- планирование (без записи) -------------------------------------------------

    fn plan_create_real(&self, vault: &Path) -> Result<TransitionPlan, AppError> {
        let snapshot = self.snapshot_vault(vault)?;
        if snapshot.marker_state != "absent" {
            return Err(BackendError::conflict(
                "В vault уже есть идентификатор «Контура». Используйте «Подключить существующее».",
            ));
        }
        if self.record_by_canonical(&snapshot.canonical)?.is_some() {
            return Err(BackendError::conflict(
                "Этот vault уже зарегистрирован. Используйте «Подключить существующее».",
            ));
        }
        self.ensure_not_inside_app_data(&snapshot.canonical)?;
        self.ensure_not_nested_in_registered_vault(&snapshot.canonical)?;
        let warnings = vec![
            "Данные текущего пространства не переносятся: новое пространство начинается с пустой базы.".to_owned(),
        ];
        Ok(TransitionPlan {
            target: None,
            target_kind: WorkspaceKind::Real,
            effects: vec![
                format!("Создать файл {MARKER_FILE} в корне vault (только идентификатор, без учебных данных)."),
                "Создать отдельную базу SQLite в каталоге данных приложения.".to_owned(),
                "Сделать новое пространство активным и перезапустить приложение.".to_owned(),
            ],
            warnings,
            snapshot: Some(snapshot),
        })
    }

    fn plan_connect(&self, vault: &Path) -> Result<TransitionPlan, AppError> {
        let snapshot = self.snapshot_vault(vault)?;
        let (record, _) = self.resolve_known_vault(&snapshot)?;
        self.reject_duplicate_copy(&record, &snapshot)?;
        let mut warnings = Vec::new();
        if record.vault_canonical.as_deref() != snapshot.canonical.to_str() {
            warnings.push(
                "Vault перемещён: путь пространства будет обновлён после проверки fingerprint."
                    .to_owned(),
            );
        }
        Ok(TransitionPlan {
            target: Some(record.dto()),
            target_kind: record.kind,
            effects: vec![
                "Сделать это пространство активным и перезапустить приложение.".to_owned(),
                "Vault и учебные данные не изменяются.".to_owned(),
            ],
            warnings,
            snapshot: Some(snapshot),
        })
    }

    fn plan_demo(&self) -> TransitionPlan {
        TransitionPlan {
            target: None,
            target_kind: WorkspaceKind::Demo,
            effects: vec![
                "Создать демо-хранилище, загрузки и резервные копии только в каталоге данных приложения.".to_owned(),
                "Создать отдельную базу SQLite с «Демо-курсом».".to_owned(),
                "Сделать демо активным и перезапустить приложение.".to_owned(),
            ],
            warnings: vec!["Демо никогда не смешивается с реальными данными и не может стать реальным пространством.".to_owned()],
            snapshot: None,
        }
    }

    fn plan_activate(&self, workspace_id: &str) -> Result<TransitionPlan, AppError> {
        let record = self
            .record_by_id(workspace_id)?
            .ok_or_else(|| BackendError::not_found("Рабочее пространство не найдено"))?;
        let mut warnings = Vec::new();
        if let Some(path) = &record.vault_canonical {
            if self.verify_record_vault(&record, Path::new(path)).is_err() {
                return Err(BackendError::blocked(
                    "Vault этого пространства недоступен или его fingerprint изменился. Подключите vault через «Подключить существующее».",
                ));
            }
        } else {
            warnings.push("У пространства нет привязанного vault.".to_owned());
        }
        Ok(TransitionPlan {
            target: Some(record.dto()),
            target_kind: record.kind,
            effects: vec![
                "Сделать это пространство активным и перезапустить приложение.".to_owned(),
            ],
            warnings,
            snapshot: None,
        })
    }

    // ----- применение ----------------------------------------------------------------

    fn create_real(&self, snapshot: &VaultSnapshot) -> Result<WorkspaceActivationDto, AppError> {
        if self.record_by_canonical(&snapshot.canonical)?.is_some() {
            return Err(BackendError::conflict("Этот vault уже зарегистрирован"));
        }
        self.ensure_not_inside_app_data(&snapshot.canonical)?;
        self.ensure_not_nested_in_registered_vault(&snapshot.canonical)?;
        let id = Uuid::new_v4().to_string();
        let marker = VaultMarker {
            workspace_id: id.clone(),
            nonce: Uuid::new_v4().to_string(),
        };
        let fingerprint = marker_fingerprint(&marker)?;
        let relative = format!("workspaces/{id}.sqlite3");
        let database_path = self.database_path(&relative)?;
        write_marker(&snapshot.canonical, &marker)?;
        let result = self.create_database_and_activate(
            &id,
            WorkspaceKind::Real,
            &snapshot.canonical,
            &fingerprint,
            &relative,
            None,
        );
        if result.is_err() {
            remove_marker_if_ours(&snapshot.canonical, &marker);
            remove_database_files(&database_path);
        }
        result
    }

    fn create_demo(&self) -> Result<WorkspaceActivationDto, AppError> {
        let id = Uuid::new_v4().to_string();
        let demo_root = self.app_data_dir.join(DEMO_DIR);
        if fs::symlink_metadata(&demo_root).is_ok_and(|metadata| !metadata.file_type().is_dir()) {
            // Symlink или файл на месте каталога демо: ничего не создаём и не удаляем.
            return Err(BackendError::blocked(
                "Каталог демо-пространств не может быть символической ссылкой",
            ));
        }
        let base = demo_root.join(&id);
        let relative = format!("workspaces/{id}.sqlite3");
        let database_path = self.database_path(&relative)?;
        let result = (|| {
            let vault = base.join("Учебное хранилище");
            let backup = base.join("Резервные копии");
            let downloads = base.join("Загрузки");
            for directory in [&vault, &backup, &downloads] {
                fs::create_dir_all(directory).map_err(|_| {
                    BackendError::io("Не удалось создать безопасное демонстрационное пространство")
                })?;
            }
            let canonical = fs::canonicalize(&vault)?;
            self.ensure_inside_app_data(&canonical)?;
            let marker = VaultMarker {
                workspace_id: id.clone(),
                nonce: Uuid::new_v4().to_string(),
            };
            write_marker(&canonical, &marker)?;
            let fingerprint = marker_fingerprint(&marker)?;
            self.create_database_and_activate(
                &id,
                WorkspaceKind::Demo,
                &canonical,
                &fingerprint,
                &relative,
                Some((&backup, &downloads)),
            )
        })();
        if result.is_err() {
            remove_database_files(&database_path);
            // Каталог создан этим вызовом с новым UUID внутри app data — удаляется целиком.
            let _ = fs::remove_dir_all(&base);
        }
        result
    }

    fn connect_existing(
        &self,
        snapshot: &VaultSnapshot,
        expected_id: Option<&str>,
    ) -> Result<WorkspaceActivationDto, AppError> {
        let (record, fingerprint) = self.resolve_known_vault(snapshot)?;
        self.reject_duplicate_copy(&record, snapshot)?;
        if expected_id.is_some_and(|id| id != record.id) {
            return Err(BackendError::blocked(
                "Vault изменился после предпросмотра. Повторите подключение.",
            ));
        }
        let database = self.open_workspace_database(&record)?;
        let identity = read_identity(&database)?;
        if identity.id != record.id
            || identity.kind != record.kind
            || identity.vault_fingerprint.as_deref() != Some(fingerprint.as_str())
        {
            return Err(BackendError::blocked(
                "Идентификатор базы и fingerprint vault не совпадают",
            ));
        }
        let canonical_text = path_text(&snapshot.canonical)?;
        if record.vault_canonical.as_deref() != Some(canonical_text) {
            // Vault перемещён: обновляем путь в базе workspace, затем в каталоге.
            update_identity_vault(&database, canonical_text, &fingerprint)?;
            let mut roots = read_roots(&database)?;
            roots.vault_path = Some(canonical_text.to_owned());
            write_roots(&database, &roots)?;
            self.update_canonical(&record.id, canonical_text)?;
        }
        drop(database);
        self.activate(&record.id)?;
        Ok(WorkspaceActivationDto {
            workspace: self.require_record(&record.id)?.dto(),
            restart_required: true,
        })
    }

    fn activate_existing(&self, id: &str) -> Result<WorkspaceActivationDto, AppError> {
        let record = self.require_record(id)?;
        if let Some(path) = &record.vault_canonical {
            self.verify_record_vault(&record, Path::new(path))?;
        }
        let database = self.open_workspace_database(&record)?;
        let identity = read_identity(&database)?;
        if identity.id != record.id || identity.kind != record.kind {
            return Err(BackendError::blocked(
                "База пространства не совпадает с каталогом",
            ));
        }
        drop(database);
        self.activate(&record.id)?;
        Ok(WorkspaceActivationDto {
            workspace: self.require_record(id)?.dto(),
            restart_required: true,
        })
    }

    fn create_database_and_activate(
        &self,
        id: &str,
        kind: WorkspaceKind,
        canonical: &Path,
        fingerprint: &str,
        database_relpath: &str,
        demo_roots: Option<(&Path, &Path)>,
    ) -> Result<WorkspaceActivationDto, AppError> {
        let canonical_text = path_text(canonical)?;
        let database_path = self.database_path(database_relpath)?;
        if fs::symlink_metadata(&database_path).is_ok() {
            return Err(BackendError::conflict(
                "База нового пространства уже существует",
            ));
        }
        let database = Database::open_file(&database_path)?;
        database.migrate()?;
        let created_at = database.with_connection(timestamp)?;
        let workspace = WorkspaceDto {
            id: id.to_owned(),
            kind,
            vault_canonical: Some(canonical_text.to_owned()),
            vault_fingerprint: Some(fingerprint.to_owned()),
            created_at,
            state: WorkspaceState::Active,
        };
        write_identity(&database, &workspace)?;
        let mut roots = ManagedRootsConfig {
            vault_path: Some(canonical_text.to_owned()),
            backup_path: None,
            downloads_path: None,
        };
        if let Some((backup, downloads)) = demo_roots {
            roots.backup_path = Some(path_text(&fs::canonicalize(backup)?)?.to_owned());
            roots.downloads_path = Some(path_text(&fs::canonicalize(downloads)?)?.to_owned());
        }
        write_roots(&database, &roots)?;
        if demo_roots.is_some() {
            crate::intake::RootsService::new(&database, None).prepare_inbox()?;
            database.with_transaction(|transaction| {
                transaction.execute(
                    "INSERT INTO subjects(id,name,short_name,folder_path)
                     VALUES ('demo-course','Демо-курс','Демо','Предметы/Демо-курс')",
                    [],
                )?;
                Ok(())
            })?;
        }
        drop(database);
        self.insert_and_activate(&workspace, database_relpath)?;
        Ok(WorkspaceActivationDto {
            workspace,
            restart_required: true,
        })
    }

    /// Однократный перенос базы схемы ≤ 8 в каталог. Файл остаётся на месте.
    fn import_legacy(&self) -> Result<(), AppError> {
        let path = self.app_data_dir.join(LEGACY_DATABASE);
        let database = Database::open_file(&path)?;
        database.migrate()?;
        if let Some(existing) = try_read_identity(&database)? {
            // Каталог был потерян, а база уже имеет identity: восстанавливаем запись как есть.
            drop(database);
            return self.insert_and_activate(&existing, LEGACY_DATABASE);
        }
        let roots = read_roots(&database)?;
        let configured = roots.vault_path.as_deref().map(PathBuf::from);
        let legacy_demo_root = fs::canonicalize(self.app_data_dir.join(LEGACY_DEMO_DIR)).ok();
        let resolved = configured.as_deref().and_then(|path| {
            VaultRoot::new(path, self.home.as_deref()).ok()?;
            let canonical = fs::canonicalize(path).ok()?;
            let fingerprint = legacy_fingerprint(&canonical).ok()?;
            Some((canonical, fingerprint))
        });
        // Демо — только пустая конфигурация или прежнее демо внутри app data.
        // Любой другой выбранный пользователем vault считается реальным.
        let is_demo = match (&configured, &resolved) {
            (None, _) => true,
            (Some(_), Some((canonical, _))) => legacy_demo_root
                .as_deref()
                .is_some_and(|root| canonical.starts_with(root)),
            (Some(path), None) => path.starts_with(self.app_data_dir.join(LEGACY_DEMO_DIR)),
        };
        let created_at = database.with_connection(timestamp)?;
        let workspace = WorkspaceDto {
            id: Uuid::new_v4().to_string(),
            kind: if is_demo {
                WorkspaceKind::Demo
            } else {
                WorkspaceKind::Real
            },
            vault_canonical: resolved
                .as_ref()
                .map(|(canonical, _)| canonical.to_string_lossy().into_owned()),
            vault_fingerprint: resolved.map(|(_, fingerprint)| fingerprint),
            created_at,
            state: WorkspaceState::Active,
        };
        write_identity(&database, &workspace)?;
        drop(database);
        self.insert_and_activate(&workspace, LEGACY_DATABASE)
    }

    fn bind_legacy_vault(
        &self,
        database: &Database,
        record: &CatalogRecord,
    ) -> Result<(), AppError> {
        let roots = read_roots(database)?;
        let Some(path) = roots.vault_path.as_deref().map(PathBuf::from) else {
            return Ok(());
        };
        if VaultRoot::new(&path, self.home.as_deref()).is_err() {
            return Ok(());
        }
        let Ok(canonical) = fs::canonicalize(&path) else {
            return Ok(());
        };
        if let Some(other) = self.record_by_canonical(&canonical)? {
            if other.id != record.id {
                return Err(BackendError::blocked(
                    "Vault legacy-пространства уже принадлежит другому пространству",
                ));
            }
        }
        let fingerprint = legacy_fingerprint(&canonical)?;
        let text = path_text(&canonical)?;
        update_identity_vault(database, text, &fingerprint)?;
        self.lock()?.execute(
            "UPDATE workspaces SET vault_canonical=?2, vault_fingerprint=?3 WHERE id=?1",
            params![record.id, text, fingerprint],
        )?;
        Ok(())
    }

    // ----- vault: snapshot / проверка --------------------------------------------------

    fn snapshot_vault(&self, vault: &Path) -> Result<VaultSnapshot, AppError> {
        VaultRoot::new(vault, self.home.as_deref())?;
        let canonical = fs::canonicalize(vault)
            .map_err(|_| BackendError::blocked("Выбранный vault недоступен"))?;
        let metadata = fs::symlink_metadata(&canonical)?;
        if !metadata.file_type().is_dir() {
            return Err(BackendError::blocked("Vault должен быть папкой"));
        }
        Ok(VaultSnapshot {
            marker_state: marker_state(&canonical)?,
            dev: metadata.dev(),
            ino: metadata.ino(),
            canonical,
        })
    }

    fn revalidate_snapshot(&self, pending: &PendingTransition) -> Result<VaultSnapshot, AppError> {
        let expected = pending
            .snapshot
            .as_ref()
            .ok_or_else(|| BackendError::database("Переход без снимка vault"))?;
        let current = self.snapshot_vault(&expected.canonical)?;
        if &current != expected {
            return Err(BackendError::blocked(
                "Vault изменился после предпросмотра (путь, папка или идентификатор). Выполните предпросмотр заново.",
            ));
        }
        Ok(current)
    }

    /// Находит запись каталога по marker (или legacy fingerprint) и проверяет совпадение.
    fn resolve_known_vault(
        &self,
        snapshot: &VaultSnapshot,
    ) -> Result<(CatalogRecord, String), AppError> {
        let (record, fingerprint) = if snapshot.marker_state != "absent" {
            let marker = read_marker(&snapshot.canonical)?;
            let fingerprint = marker_fingerprint(&marker)?;
            let record = self.record_by_id(&marker.workspace_id)?.ok_or_else(|| {
                BackendError::blocked("Этот vault не зарегистрирован в «Контуре»")
            })?;
            (record, fingerprint)
        } else {
            let record = self
                .record_by_canonical(&snapshot.canonical)?
                .ok_or_else(|| {
                    BackendError::blocked(
                        "В vault нет проверенного идентификатора рабочего пространства",
                    )
                })?;
            (record, legacy_fingerprint(&snapshot.canonical)?)
        };
        if record.vault_fingerprint.as_deref() != Some(fingerprint.as_str()) {
            return Err(BackendError::blocked(
                "Fingerprint vault не совпадает. Подключение заблокировано.",
            ));
        }
        Ok((record, fingerprint))
    }

    /// Копия vault (например, резервная) содержит тот же marker. Если исходный vault
    /// всё ещё на зарегистрированном месте, подключение копии запрещено: иначе база
    /// незаметно начнёт работать с другой папкой.
    fn reject_duplicate_copy(
        &self,
        record: &CatalogRecord,
        snapshot: &VaultSnapshot,
    ) -> Result<(), AppError> {
        let Some(registered) = record.vault_canonical.as_deref().map(Path::new) else {
            return Ok(());
        };
        if registered == snapshot.canonical {
            return Ok(());
        }
        let Ok(original) = self.snapshot_vault(registered) else {
            return Ok(());
        };
        if original.dev == snapshot.dev && original.ino == snapshot.ino {
            return Ok(());
        }
        if self
            .resolve_known_vault(&original)
            .is_ok_and(|(found, _)| found.id == record.id)
        {
            return Err(BackendError::blocked(
                "Это копия vault: исходный vault этого пространства всё ещё на месте. Подключите исходную папку или удалите идентификатор из копии.",
            ));
        }
        Ok(())
    }

    fn verify_record_vault(&self, record: &CatalogRecord, path: &Path) -> Result<(), AppError> {
        let snapshot = self.snapshot_vault(path)?;
        let (found, _) = self.resolve_known_vault(&snapshot)?;
        if found.id != record.id {
            return Err(BackendError::blocked(
                "Vault принадлежит другому пространству",
            ));
        }
        Ok(())
    }

    fn ensure_not_inside_app_data(&self, canonical: &Path) -> Result<(), AppError> {
        if let Ok(app_data) = fs::canonicalize(&self.app_data_dir) {
            if canonical.starts_with(&app_data) || app_data.starts_with(canonical) {
                return Err(BackendError::blocked(
                    "Реальный vault не может находиться в каталоге данных приложения или содержать его",
                ));
            }
        }
        Ok(())
    }

    /// Вложенные друг в друга vault разных пространств делят файлы: сканер одного
    /// пространства увидел бы материалы другого. Такое пересечение запрещено.
    fn ensure_not_nested_in_registered_vault(&self, canonical: &Path) -> Result<(), AppError> {
        let overlaps = self.list()?.iter().any(|workspace| {
            workspace.vault_canonical.as_deref().is_some_and(|other| {
                let other = Path::new(other);
                canonical.starts_with(other) || other.starts_with(canonical)
            })
        });
        if overlaps {
            return Err(BackendError::blocked(
                "Выбранная папка находится внутри уже зарегистрированного vault или содержит его. Выберите отдельное хранилище.",
            ));
        }
        Ok(())
    }

    /// Демо-хранилище обязано физически находиться внутри каталога данных приложения:
    /// symlink `demo-workspaces` наружу (например, в реальный vault) запрещён.
    fn ensure_inside_app_data(&self, canonical: &Path) -> Result<(), AppError> {
        let app_data = fs::canonicalize(&self.app_data_dir)?;
        let demo_root = self.app_data_dir.join(DEMO_DIR);
        let demo_root_is_symlink = fs::symlink_metadata(&demo_root)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false);
        if demo_root_is_symlink || !canonical.starts_with(app_data.join(DEMO_DIR)) {
            return Err(BackendError::blocked(
                "Демо-пространство должно находиться внутри каталога данных приложения",
            ));
        }
        Ok(())
    }

    // ----- журнал переходов ------------------------------------------------------------

    fn claim_transition(&self, id: &str) -> Result<PendingTransition, AppError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let row = transaction
            .query_row(
                "SELECT action,vault_canonical,vault_dev,vault_ino,marker_state,target_workspace_id,
                        from_workspace_id,state,
                        expires_at > strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 FROM workspace_transitions WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, bool>(8)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| BackendError::not_found("Предпросмотр перехода не найден"))?;
        let (action, canonical, dev, ino, marker, target, from, state, fresh) = row;
        if state != TransitionState::Previewed.as_str() {
            return Err(BackendError::conflict(
                "Переход уже применён, отменён или выполняется",
            ));
        }
        if !fresh {
            finish_in(
                &transaction,
                id,
                TransitionState::Failed,
                None,
                Some("Предпросмотр устарел"),
            )?;
            transaction.commit()?;
            return Err(BackendError::new(
                ErrorCode::ConfirmationRequired,
                "Предпросмотр устарел. Выполните его заново.",
            ));
        }
        let active: Option<String> = active_id(&transaction)?;
        if active != from {
            finish_in(
                &transaction,
                id,
                TransitionState::Failed,
                None,
                Some("Активное пространство изменилось после предпросмотра"),
            )?;
            transaction.commit()?;
            return Err(BackendError::conflict(
                "Активное пространство изменилось после предпросмотра. Выполните его заново.",
            ));
        }
        transaction.execute(
            "UPDATE workspace_transitions SET state='applying' WHERE id=?1 AND state='previewed'",
            [id],
        )?;
        transaction.commit()?;
        let action = TransitionAction::parse(&action)
            .ok_or_else(|| BackendError::database("Неизвестный тип перехода"))?;
        let snapshot = match (canonical, dev, ino, marker) {
            (Some(canonical), Some(dev), Some(ino), Some(marker_state)) => Some(VaultSnapshot {
                canonical: PathBuf::from(canonical),
                dev: dev as u64,
                ino: ino as u64,
                marker_state,
            }),
            _ => None,
        };
        Ok(PendingTransition {
            action,
            snapshot,
            target_workspace_id: target,
        })
    }

    fn finish_transition(
        &self,
        id: &str,
        state: TransitionState,
        target: Option<&str>,
        message: Option<&str>,
    ) -> Result<(), AppError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        finish_in(&transaction, id, state, target, message)?;
        transaction.commit()?;
        Ok(())
    }

    fn fail_interrupted_transitions(&self) -> Result<(), AppError> {
        self.lock()?.execute(
            "UPDATE workspace_transitions
             SET state='failed', message='Переход прерван закрытием приложения',
                 finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE state='applying'",
            [],
        )?;
        Ok(())
    }

    pub fn recent_transitions(&self, limit: u32) -> Result<Vec<WorkspaceTransitionDto>, AppError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id,action,state,from_workspace_id,target_workspace_id,message,created_at,finished_at
             FROM workspace_transitions ORDER BY created_at DESC, id LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            let action: String = row.get(1)?;
            let state: String = row.get(2)?;
            Ok(WorkspaceTransitionDto {
                id: row.get(0)?,
                action: TransitionAction::parse(&action).ok_or(rusqlite::Error::InvalidQuery)?,
                state: TransitionState::parse(&state).ok_or(rusqlite::Error::InvalidQuery)?,
                from_workspace_id: row.get(3)?,
                target_workspace_id: row.get(4)?,
                message: row.get(5)?,
                created_at: row.get(6)?,
                finished_at: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // ----- каталог -----------------------------------------------------------------------

    fn insert_and_activate(
        &self,
        workspace: &WorkspaceDto,
        database_relpath: &str,
    ) -> Result<(), AppError> {
        self.database_path(database_relpath)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        transaction.execute("UPDATE workspaces SET state='archived'", [])?;
        transaction.execute(
            "INSERT INTO workspaces(id,kind,vault_canonical,vault_fingerprint,database_relpath,created_at,state)
             VALUES (?1,?2,?3,?4,?5,?6,'active')",
            params![
                workspace.id,
                workspace.kind.as_str(),
                workspace.vault_canonical,
                workspace.vault_fingerprint,
                database_relpath,
                workspace.created_at
            ],
        )?;
        set_active_in(&transaction, &workspace.id)?;
        transaction.commit()?;
        Ok(())
    }

    fn activate(&self, id: &str) -> Result<(), AppError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=?1)",
            [id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(BackendError::not_found("Рабочее пространство не найдено"));
        }
        transaction.execute("UPDATE workspaces SET state='archived'", [])?;
        transaction.execute("UPDATE workspaces SET state='active' WHERE id=?1", [id])?;
        set_active_in(&transaction, id)?;
        transaction.commit()?;
        Ok(())
    }

    fn update_canonical(&self, id: &str, canonical: &str) -> Result<(), AppError> {
        self.lock()?.execute(
            "UPDATE workspaces SET vault_canonical=?2 WHERE id=?1",
            params![id, canonical],
        )?;
        Ok(())
    }

    fn workspace_count(&self) -> Result<u64, AppError> {
        let count: i64 = self
            .lock()?
            .query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0))?;
        u64::try_from(count).map_err(|_| BackendError::database("Некорректный каталог"))
    }

    fn active_workspace(&self) -> Result<Option<WorkspaceDto>, AppError> {
        match self.active_record() {
            Ok(record) => Ok(Some(record.dto())),
            Err(error) if error.code == ErrorCode::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn active_record(&self) -> Result<CatalogRecord, AppError> {
        self.lock()?
            .query_row(
                &format!(
                    "SELECT {RECORD_COLUMNS} FROM workspaces w JOIN workspace_catalog_settings s
                       ON s.key='active_workspace_id' AND s.value=w.id"
                ),
                [],
                row_record,
            )
            .optional()?
            .ok_or_else(|| BackendError::not_found("Активное рабочее пространство не найдено"))
    }

    fn require_record(&self, id: &str) -> Result<CatalogRecord, AppError> {
        self.record_by_id(id)?
            .ok_or_else(|| BackendError::not_found("Рабочее пространство не найдено"))
    }

    fn record_by_id(&self, id: &str) -> Result<Option<CatalogRecord>, AppError> {
        Ok(self
            .lock()?
            .query_row(
                &format!("SELECT {RECORD_COLUMNS} FROM workspaces w WHERE w.id=?1"),
                [id],
                row_record,
            )
            .optional()?)
    }

    fn record_by_canonical(&self, path: &Path) -> Result<Option<CatalogRecord>, AppError> {
        let path = path_text(path)?;
        Ok(self
            .lock()?
            .query_row(
                &format!("SELECT {RECORD_COLUMNS} FROM workspaces w WHERE w.vault_canonical=?1"),
                [path],
                row_record,
            )
            .optional()?)
    }

    fn open_workspace_database(&self, record: &CatalogRecord) -> Result<Database, AppError> {
        let path = self.database_path(&record.database_relpath)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|_| BackendError::blocked("База рабочего пространства отсутствует"))?;
        if !metadata.file_type().is_file() {
            return Err(BackendError::blocked(
                "База рабочего пространства не является обычным файлом",
            ));
        }
        let database = Database::open_file(&path)?;
        database.migrate()?;
        Ok(database)
    }

    /// Только `kontur.sqlite3` или `workspaces/<uuid>.sqlite3` внутри app data.
    fn database_path(&self, relative: &str) -> Result<PathBuf, AppError> {
        let valid = relative == LEGACY_DATABASE
            || relative
                .strip_prefix("workspaces/")
                .and_then(|name| name.strip_suffix(".sqlite3"))
                .is_some_and(|stem| Uuid::parse_str(stem).is_ok() && !stem.contains('/'));
        let path = Path::new(relative);
        if !valid
            || path.is_absolute()
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(BackendError::database("Некорректный путь базы workspace"));
        }
        let full = self.app_data_dir.join(path);
        // Каталог `workspaces` не должен быть symlink наружу app data.
        if let Some(parent) = full.parent() {
            if let Ok(metadata) = fs::symlink_metadata(parent) {
                if metadata.file_type().is_symlink() {
                    return Err(BackendError::blocked(
                        "Каталог баз workspace не может быть символической ссылкой",
                    ));
                }
            }
        }
        if let Ok(metadata) = fs::symlink_metadata(&full) {
            if metadata.file_type().is_symlink() {
                return Err(BackendError::blocked(
                    "База workspace не может быть символической ссылкой",
                ));
            }
        }
        Ok(full)
    }

    fn display(&self, path: &Path) -> String {
        if let Some(home) = &self.home {
            if let Ok(rest) = path.strip_prefix(home) {
                return if rest.as_os_str().is_empty() {
                    "~".to_owned()
                } else {
                    format!("~/{}", rest.to_string_lossy())
                };
            }
        }
        path.to_string_lossy().into_owned()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.connection
            .lock()
            .map_err(|_| BackendError::database("Каталог workspace повреждён"))
    }
}

struct TransitionPlan {
    target: Option<WorkspaceDto>,
    target_kind: WorkspaceKind,
    effects: Vec<String>,
    warnings: Vec<String>,
    snapshot: Option<VaultSnapshot>,
}

struct PendingTransition {
    action: TransitionAction,
    snapshot: Option<VaultSnapshot>,
    target_workspace_id: Option<String>,
}

const RECORD_COLUMNS: &str =
    "w.id,w.kind,w.vault_canonical,w.vault_fingerprint,w.database_relpath,w.created_at,w.state";

#[derive(Debug, Clone)]
struct CatalogRecord {
    id: String,
    kind: WorkspaceKind,
    vault_canonical: Option<String>,
    vault_fingerprint: Option<String>,
    database_relpath: String,
    created_at: String,
    state: WorkspaceState,
}

impl CatalogRecord {
    fn dto(&self) -> WorkspaceDto {
        WorkspaceDto {
            id: self.id.clone(),
            kind: self.kind,
            vault_canonical: self.vault_canonical.clone(),
            vault_fingerprint: self.vault_fingerprint.clone(),
            created_at: self.created_at.clone(),
            state: self.state,
        }
    }
}

fn migrate_catalog(connection: &Connection) -> Result<(), AppError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > CATALOG_SCHEMA_VERSION {
        return Err(BackendError::database(
            "Каталог рабочих пространств создан более новой версией приложения",
        ));
    }
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS workspaces (
           id TEXT PRIMARY KEY NOT NULL,
           kind TEXT NOT NULL CHECK (kind IN ('demo','real')),
           vault_canonical TEXT,
           vault_fingerprint TEXT,
           database_relpath TEXT NOT NULL UNIQUE,
           created_at TEXT NOT NULL,
           state TEXT NOT NULL CHECK (state IN ('active','archived')),
           CHECK ((vault_canonical IS NULL) = (vault_fingerprint IS NULL))
         ) STRICT;
         CREATE UNIQUE INDEX IF NOT EXISTS workspace_vault_fingerprint
           ON workspaces(vault_fingerprint) WHERE vault_fingerprint IS NOT NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS workspace_vault_canonical
           ON workspaces(vault_canonical) WHERE vault_canonical IS NOT NULL;
         CREATE TRIGGER IF NOT EXISTS workspaces_identity_stable
         BEFORE UPDATE OF id, kind, database_relpath, created_at ON workspaces
         BEGIN SELECT RAISE(ABORT, 'workspace identity is immutable'); END;
         CREATE TRIGGER IF NOT EXISTS workspaces_fingerprint_bound_once
         BEFORE UPDATE OF vault_fingerprint ON workspaces
         WHEN OLD.vault_fingerprint IS NOT NULL
           AND NEW.vault_fingerprint IS NOT OLD.vault_fingerprint
         BEGIN SELECT RAISE(ABORT, 'workspace vault fingerprint is immutable'); END;
         CREATE TRIGGER IF NOT EXISTS workspaces_no_delete
         BEFORE DELETE ON workspaces
         BEGIN SELECT RAISE(ABORT, 'workspace records are append-only'); END;
         CREATE TABLE IF NOT EXISTS workspace_catalog_settings (
           key TEXT PRIMARY KEY NOT NULL,
           value TEXT NOT NULL
         ) STRICT;
         CREATE TABLE IF NOT EXISTS workspace_transitions (
           id TEXT PRIMARY KEY NOT NULL,
           action TEXT NOT NULL CHECK (action IN ('create_real','connect_existing','create_demo','activate')),
           vault_canonical TEXT,
           vault_dev INTEGER,
           vault_ino INTEGER,
           marker_state TEXT,
           from_workspace_id TEXT,
           target_workspace_id TEXT,
           state TEXT NOT NULL CHECK (state IN ('previewed','applying','applied','cancelled','failed')),
           message TEXT,
           created_at TEXT NOT NULL,
           expires_at TEXT NOT NULL,
           finished_at TEXT
         ) STRICT;
         CREATE INDEX IF NOT EXISTS workspace_transitions_created
           ON workspace_transitions(created_at);
         PRAGMA user_version = 1;
         COMMIT;",
    )?;
    Ok(())
}

fn active_id(transaction: &Transaction<'_>) -> Result<Option<String>, AppError> {
    Ok(transaction
        .query_row(
            "SELECT value FROM workspace_catalog_settings WHERE key='active_workspace_id'",
            [],
            |row| row.get(0),
        )
        .optional()?)
}

fn set_active_in(transaction: &Transaction<'_>, id: &str) -> Result<(), AppError> {
    transaction.execute(
        "INSERT INTO workspace_catalog_settings(key,value) VALUES ('active_workspace_id',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        [id],
    )?;
    Ok(())
}

fn finish_in(
    transaction: &Transaction<'_>,
    id: &str,
    state: TransitionState,
    target: Option<&str>,
    message: Option<&str>,
) -> Result<(), AppError> {
    transaction.execute(
        "UPDATE workspace_transitions
         SET state=?2, target_workspace_id=COALESCE(?3,target_workspace_id), message=?4,
             finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id=?1",
        params![id, state.as_str(), target, message],
    )?;
    Ok(())
}

fn row_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<CatalogRecord> {
    Ok(CatalogRecord {
        id: row.get(0)?,
        kind: parse_kind(row.get::<_, String>(1)?)?,
        vault_canonical: row.get(2)?,
        vault_fingerprint: row.get(3)?,
        database_relpath: row.get(4)?,
        created_at: row.get(5)?,
        state: parse_state(row.get::<_, String>(6)?)?,
    })
}

fn row_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceDto> {
    Ok(WorkspaceDto {
        id: row.get(0)?,
        kind: parse_kind(row.get::<_, String>(1)?)?,
        vault_canonical: row.get(2)?,
        vault_fingerprint: row.get(3)?,
        created_at: row.get(4)?,
        state: parse_state(row.get::<_, String>(5)?)?,
    })
}

fn parse_kind(value: String) -> rusqlite::Result<WorkspaceKind> {
    match value.as_str() {
        "demo" => Ok(WorkspaceKind::Demo),
        "real" => Ok(WorkspaceKind::Real),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn parse_state(value: String) -> rusqlite::Result<WorkspaceState> {
    match value.as_str() {
        "active" => Ok(WorkspaceState::Active),
        "archived" => Ok(WorkspaceState::Archived),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn timestamp(connection: &Connection) -> Result<String, BackendError> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .map_err(Into::into)
}

fn path_text(path: &Path) -> Result<&str, AppError> {
    path.to_str()
        .ok_or_else(|| BackendError::blocked("Путь vault содержит недопустимые символы"))
}

fn write_identity(database: &Database, workspace: &WorkspaceDto) -> Result<(), AppError> {
    database.with_transaction(|transaction| {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT workspace_id FROM workspace_identity WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != workspace.id {
                return Err(BackendError::immutable(
                    "Идентификатор workspace базы неизменяем",
                ));
            }
            return Ok(());
        }
        transaction.execute(
            "INSERT INTO workspace_identity(singleton,workspace_id,kind,vault_canonical,vault_fingerprint,created_at,state)
             VALUES (1,?1,?2,?3,?4,?5,?6)",
            params![
                workspace.id,
                workspace.kind.as_str(),
                workspace.vault_canonical,
                workspace.vault_fingerprint,
                workspace.created_at,
                workspace.state.as_str()
            ],
        )?;
        Ok(())
    })
}

const IDENTITY_SELECT: &str =
    "SELECT workspace_id,kind,vault_canonical,vault_fingerprint,created_at,state
     FROM workspace_identity WHERE singleton=1";

pub fn read_identity(database: &Database) -> Result<WorkspaceDto, AppError> {
    try_read_identity(database)?
        .ok_or_else(|| BackendError::database("Идентификатор workspace отсутствует"))
}

pub fn try_read_identity(database: &Database) -> Result<Option<WorkspaceDto>, AppError> {
    database.with_connection(|connection| {
        Ok(connection
            .query_row(IDENTITY_SELECT, [], row_workspace)
            .optional()?)
    })
}

/// Проверяет, что путь vault из настроек всё ещё указывает на vault этой базы.
/// Используется перед любой файловой операцией через `RootsService`.
pub fn verify_vault_binding(database: &Database, vault: &Path) -> Result<(), AppError> {
    let Some(identity) = try_read_identity(database)? else {
        // База без identity (in-memory тесты, до bootstrap) — граница не задана.
        return Ok(());
    };
    let Some(expected) = identity.vault_fingerprint.as_deref() else {
        return Ok(());
    };
    let canonical = fs::canonicalize(vault)
        .map_err(|_| BackendError::not_found("Хранилище Obsidian не найдено"))?;
    let actual = match marker_state(&canonical)?.as_str() {
        "absent" => legacy_fingerprint(&canonical)?,
        _ => marker_fingerprint(&read_marker(&canonical)?)?,
    };
    if actual != expected {
        return Err(BackendError::blocked(
            "Папка vault не совпадает с рабочим пространством (fingerprint изменился). Подключите правильный vault или создайте новое пространство.",
        ));
    }
    Ok(())
}

fn update_identity_vault(
    database: &Database,
    canonical: &str,
    fingerprint: &str,
) -> Result<(), AppError> {
    database.with_transaction(|transaction| {
        transaction.execute(
            "UPDATE workspace_identity SET vault_canonical=?1,vault_fingerprint=?2 WHERE singleton=1",
            params![canonical, fingerprint],
        )?;
        Ok(())
    })
}

fn read_roots(database: &Database) -> Result<ManagedRootsConfig, AppError> {
    database.with_connection(|connection| {
        let raw: Option<String> = connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE key=?1",
                [ROOTS_KEY],
                |row| row.get(0),
            )
            .optional()?;
        raw.map_or(Ok(ManagedRootsConfig::default()), |value| {
            serde_json::from_str(&value)
                .map_err(|_| BackendError::database("Настройки корней повреждены"))
        })
    })
}

fn write_roots(database: &Database, roots: &ManagedRootsConfig) -> Result<(), AppError> {
    let json =
        serde_json::to_string(roots).map_err(|error| BackendError::database(error.to_string()))?;
    database.with_transaction(|transaction| {
        transaction.execute(
            "INSERT INTO app_settings(key,value_json,created_at,updated_at)
             VALUES (?1,?2,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,updated_at=CURRENT_TIMESTAMP",
            params![ROOTS_KEY, json],
        )?;
        Ok(())
    })
}

/// `absent` или SHA-256 содержимого marker. Symlink и не-файлы отклоняются.
fn marker_state(vault: &Path) -> Result<String, AppError> {
    match fs::symlink_metadata(vault.join(MARKER_FILE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("absent".to_owned()),
        Err(_) => Err(BackendError::blocked(
            "Идентификатор workspace в vault недоступен",
        )),
        Ok(_) => Ok(hex_hash(&read_marker_bytes(vault)?)),
    }
}

fn write_marker(vault: &Path, marker: &VaultMarker) -> Result<(), AppError> {
    let path = vault.join(MARKER_FILE);
    let bytes =
        serde_json::to_vec(marker).map_err(|error| BackendError::database(error.to_string()))?;
    // create_new = O_CREAT|O_EXCL: не следует symlink и не перезаписывает файл.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc_nofollow())
        .open(path)
        .map_err(|error| {
            BackendError::io(format!(
                "Не удалось создать идентификатор vault: {}",
                error.kind()
            ))
        })?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_marker_bytes(vault: &Path) -> Result<Vec<u8>, AppError> {
    let path = vault.join(MARKER_FILE);
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc_nofollow())
        .open(&path)
        .map_err(|_| BackendError::blocked("Некорректный идентификатор workspace в vault"))?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() || metadata.len() > MARKER_MAX_BYTES {
        return Err(BackendError::blocked(
            "Некорректный идентификатор workspace в vault",
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MARKER_MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MARKER_MAX_BYTES {
        return Err(BackendError::blocked(
            "Некорректный идентификатор workspace в vault",
        ));
    }
    Ok(bytes)
}

fn read_marker(vault: &Path) -> Result<VaultMarker, AppError> {
    serde_json::from_slice(&read_marker_bytes(vault)?)
        .map_err(|_| BackendError::blocked("Идентификатор workspace в vault повреждён"))
}

/// Удаляет marker только если он всё ещё содержит именно наш идентификатор.
fn remove_marker_if_ours(vault: &Path, marker: &VaultMarker) {
    if let Ok(current) = read_marker(vault) {
        if current.workspace_id == marker.workspace_id && current.nonce == marker.nonce {
            let _ = fs::remove_file(vault.join(MARKER_FILE));
        }
    }
}

fn libc_nofollow() -> i32 {
    rustix::fs::OFlags::NOFOLLOW.bits() as i32
}

fn marker_fingerprint(marker: &VaultMarker) -> Result<String, AppError> {
    if Uuid::parse_str(&marker.workspace_id).is_err() || Uuid::parse_str(&marker.nonce).is_err() {
        return Err(BackendError::blocked(
            "Идентификатор workspace в vault повреждён",
        ));
    }
    Ok(hex_hash(
        format!("kontur:v1:{}:{}", marker.workspace_id, marker.nonce).as_bytes(),
    ))
}

/// Fingerprint legacy-vault без marker: путь + устройство + inode корня.
/// Подмена папки тем же именем меняет inode и блокирует подключение.
fn legacy_fingerprint(vault: &Path) -> Result<String, AppError> {
    let metadata = fs::symlink_metadata(vault)?;
    Ok(hex_hash(
        format!(
            "kontur:legacy:v1:{}:{}:{}",
            vault.to_string_lossy(),
            metadata.dev(),
            metadata.ino()
        )
        .as_bytes(),
    ))
}

fn hex_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn remove_database_files(database: &Path) {
    // Только файлы с UUID-именем внутри app data (проверено `database_path`).
    let _ = fs::remove_file(database);
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = database.as_os_str().to_owned();
        name.push(suffix);
        let _ = fs::remove_file(PathBuf::from(name));
    }
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
