//! Управляемые каталоги: хранилище Obsidian, «Входящие», резервные копии и
//! (необязательно) наблюдаемая папка загрузок.
//!
//! В SQLite (`app_settings.managed_roots`) хранятся только канонические абсолютные
//! пути трёх выбранных пользователем корней. «Входящие» — фиксированный
//! относительный путь внутри хранилища. Security-scoped bookmarks, пароли,
//! пути исходных файлов и другие абсолютные пути не сохраняются.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    db::Database,
    error::{AppError, BackendError},
    file_service::{
        BackupRoot, DownloadsRoot, InboxRoot, PersistentPath, VaultRoot, VerifiedDestination,
    },
};

use super::store;

/// Относительный путь «Входящих» внутри хранилища (ограничен также схемой SQLite).
pub const INBOX_RELATIVE: &str = "Предметы/00 Входящие";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedRootsConfig {
    pub vault_path: Option<String>,
    pub backup_path: Option<String>,
    pub downloads_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootKind {
    Vault,
    Inbox,
    Backup,
    Downloads,
}

impl RootKind {
    fn label(self) -> &'static str {
        match self {
            Self::Vault => "Хранилище Obsidian",
            Self::Inbox => "Папка «Входящие»",
            Self::Backup => "Папка резервных копий",
            Self::Downloads => "Папка загрузок",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootState {
    Ok,
    NotConfigured,
    Missing,
    Unsafe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootStatusDto {
    pub kind: RootKind,
    pub state: RootState,
    /// Только для показа пользователю: домашний каталог сокращён до `~`.
    pub display_path: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRootsDto {
    pub vault: RootStatusDto,
    pub inbox: RootStatusDto,
    pub backup: RootStatusDto,
    pub downloads: RootStatusDto,
    /// Импорт во «Входящие» возможен: хранилище доступно, а «Входящие» есть или будут созданы.
    pub import_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootSelectionDto {
    pub cancelled: bool,
    pub roots: ManagedRootsDto,
}

/// Открытые для импорта корни.
pub struct ImportRoots {
    pub vault: VaultRoot,
    pub inbox: InboxRoot,
}

pub struct RootsService<'a> {
    database: &'a Database,
    home: Option<PathBuf>,
}

impl<'a> RootsService<'a> {
    pub fn new(database: &'a Database, home: Option<PathBuf>) -> Self {
        Self { database, home }
    }

    pub fn config(&self) -> Result<ManagedRootsConfig, AppError> {
        store::read_managed_roots(self.database)
    }

    pub fn status(&self) -> Result<ManagedRootsDto, AppError> {
        let config = self.config()?;
        let vault = self.root_status(RootKind::Vault, config.vault_path.as_deref());
        let backup = self.root_status(RootKind::Backup, config.backup_path.as_deref());
        let downloads = self.root_status(RootKind::Downloads, config.downloads_path.as_deref());
        let inbox = self.inbox_status(&config, vault.state);
        let import_ready = vault.state == RootState::Ok
            && matches!(inbox.state, RootState::Ok | RootState::Missing);
        Ok(ManagedRootsDto {
            vault,
            inbox,
            backup,
            downloads,
            import_ready,
        })
    }

    /// Проверяет и сохраняет корень, выбранный в системном диалоге.
    pub fn set_root(&self, kind: RootKind, path: &Path) -> Result<ManagedRootsDto, AppError> {
        if kind == RootKind::Inbox {
            return Err(BackendError::blocked(format!(
                "«Входящие» всегда находятся внутри хранилища: {INBOX_RELATIVE}. Создайте папку кнопкой «Подготовить»."
            )));
        }
        let home = self.home.as_deref();
        match kind {
            RootKind::Vault => VaultRoot::new(path, home).map(|_| ()),
            RootKind::Backup => BackupRoot::new(path, home).map(|_| ()),
            RootKind::Downloads => DownloadsRoot::new(path, home).map(|_| ()),
            RootKind::Inbox => unreachable!("handled above"),
        }?;
        let canonical = std::fs::canonicalize(path)
            .map_err(|_| BackendError::blocked(format!("{} недоступна", kind.label())))?;
        let canonical_string = canonical
            .to_str()
            .ok_or_else(|| BackendError::blocked("Путь содержит недопустимые символы"))?
            .to_owned();

        let mut config = self.config()?;
        match kind {
            RootKind::Vault => config.vault_path = Some(canonical_string),
            RootKind::Backup => config.backup_path = Some(canonical_string),
            RootKind::Downloads => config.downloads_path = Some(canonical_string),
            RootKind::Inbox => {}
        }
        validate_relationships(&config)?;
        store::write_managed_roots(self.database, &config)?;
        self.status()
    }

    /// Создаёт «Входящие» внутри хранилища fd-relative (`mkdirat`), без symlink.
    pub fn prepare_inbox(&self) -> Result<ManagedRootsDto, AppError> {
        self.open_import_roots()?;
        self.status()
    }

    /// Открывает хранилище и «Входящие» для импорта; «Входящие» создаются при отсутствии.
    pub fn open_import_roots(&self) -> Result<ImportRoots, AppError> {
        let vault = self.open_vault()?;
        let inbox = InboxRoot::open_or_create(&vault, inbox_path())?;
        Ok(ImportRoots { vault, inbox })
    }

    pub fn open_vault(&self) -> Result<VaultRoot, AppError> {
        let path = self.vault_path()?;
        VaultRoot::new(&path, self.home.as_deref())
    }

    /// Абсолютный путь файла внутри хранилища для действий ОС (Finder/приложение).
    ///
    /// Перед этим путь проходит fd-relative проверку: каждый компонент открывается
    /// с `O_NOFOLLOW`, конечный элемент должен существовать. Сам путь не сохраняется.
    pub fn resolve_for_os(
        &self,
        relative: &PersistentPath,
        expect_file: bool,
    ) -> Result<PathBuf, AppError> {
        let vault = self.open_vault()?;
        let destination = VerifiedDestination::in_vault(&vault, relative.clone())?;
        if expect_file {
            match crate::file_service::inspect_stored_file(&destination)? {
                crate::file_service::StoredFileState::Present { .. } => {}
                crate::file_service::StoredFileState::Missing => {
                    return Err(BackendError::not_found("Файл не найден на диске"))
                }
                crate::file_service::StoredFileState::NotRegular => {
                    return Err(BackendError::blocked("Путь не является обычным файлом"))
                }
            }
        } else {
            vault.ensure_existing_path(relative)?;
        }
        Ok(self.vault_path()?.join(relative.as_str()))
    }

    fn vault_path(&self) -> Result<PathBuf, AppError> {
        self.config()?.vault_path.map(PathBuf::from).ok_or_else(|| {
            BackendError::blocked(
                "Хранилище Obsidian не выбрано. Откройте «Настройки → Управляемые каталоги».",
            )
        })
    }

    fn root_status(&self, kind: RootKind, path: Option<&str>) -> RootStatusDto {
        let Some(path) = path else {
            return RootStatusDto {
                kind,
                state: RootState::NotConfigured,
                display_path: None,
                message: Some(match kind {
                    RootKind::Downloads => {
                        "Не выбрана (наблюдение подключается на следующем этапе)".to_owned()
                    }
                    _ => "Не выбрана".to_owned(),
                }),
            };
        };
        let path_buf = PathBuf::from(path);
        let display_path = Some(self.display(&path_buf));
        if std::fs::symlink_metadata(&path_buf).is_err() {
            return RootStatusDto {
                kind,
                state: RootState::Missing,
                display_path,
                message: Some("Папка не найдена — выберите её заново".to_owned()),
            };
        }
        let home = self.home.as_deref();
        let opened = match kind {
            RootKind::Vault => VaultRoot::new(&path_buf, home).map(|_| ()),
            RootKind::Backup => BackupRoot::new(&path_buf, home).map(|_| ()),
            RootKind::Downloads => DownloadsRoot::new(&path_buf, home).map(|_| ()),
            RootKind::Inbox => Ok(()),
        };
        match opened {
            Ok(()) => RootStatusDto {
                kind,
                state: RootState::Ok,
                display_path,
                message: None,
            },
            Err(error) => RootStatusDto {
                kind,
                state: RootState::Unsafe,
                display_path,
                message: Some(error.message),
            },
        }
    }

    fn inbox_status(&self, config: &ManagedRootsConfig, vault_state: RootState) -> RootStatusDto {
        let display_path = config
            .vault_path
            .as_deref()
            .map(|vault| self.display(&PathBuf::from(vault).join(INBOX_RELATIVE)));
        if vault_state != RootState::Ok {
            return RootStatusDto {
                kind: RootKind::Inbox,
                state: RootState::NotConfigured,
                display_path,
                message: Some("Сначала выберите хранилище Obsidian".to_owned()),
            };
        }
        let state = self
            .open_vault()
            .and_then(|vault| InboxRoot::new(&vault, inbox_path()).map(|_| ()));
        match state {
            Ok(()) => RootStatusDto {
                kind: RootKind::Inbox,
                state: RootState::Ok,
                display_path,
                message: None,
            },
            Err(error) if error.message.contains("не существует") => RootStatusDto {
                kind: RootKind::Inbox,
                state: RootState::Missing,
                display_path,
                message: Some("Папка будет создана при первом импорте".to_owned()),
            },
            Err(error) => RootStatusDto {
                kind: RootKind::Inbox,
                state: RootState::Unsafe,
                display_path,
                message: Some(error.message),
            },
        }
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
}

pub fn inbox_path() -> PersistentPath {
    PersistentPath::new(INBOX_RELATIVE).expect("constant inbox path is valid")
}

/// Корни не могут совпадать или быть вложены друг в друга: резервные копии и
/// загрузки не должны оказаться внутри хранилища, а хранилище — внутри них.
fn validate_relationships(config: &ManagedRootsConfig) -> Result<(), AppError> {
    let roots = [
        (RootKind::Vault, config.vault_path.as_deref()),
        (RootKind::Backup, config.backup_path.as_deref()),
        (RootKind::Downloads, config.downloads_path.as_deref()),
    ];
    for (index, (left_kind, left)) in roots.iter().enumerate() {
        for (right_kind, right) in roots.iter().skip(index + 1) {
            let (Some(left), Some(right)) = (left, right) else {
                continue;
            };
            let (left_path, right_path) = (Path::new(left), Path::new(right));
            if left_path.starts_with(right_path) || right_path.starts_with(left_path) {
                return Err(BackendError::conflict(format!(
                    "{} и {} не могут совпадать или находиться друг в друге",
                    left_kind.label(),
                    right_kind.label().to_lowercase()
                )));
            }
        }
    }
    Ok(())
}
