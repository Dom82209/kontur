use std::{
    ffi::OsStr,
    fs::{self, File},
    path::{Component, Path},
    sync::Arc,
};

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::error::{AppError, BackendError, ErrorCode};

use super::fd::{self, DirFd, Identity};

/// Контролируемый корень, привязанный к открытому directory fd.
///
/// Канонический путь используется только при создании для проверок политики
/// (не `/`, не home, не широкий системный каталог). После открытия все операции
/// выполняются относительно `fd`, а не через повторное разрешение строки пути.
#[derive(Debug, Clone)]
pub(crate) struct ControlledRoot {
    fd: DirFd,
}

impl ControlledRoot {
    fn new(path: &Path, home_dir: Option<&Path>, label: &str) -> Result<Self, AppError> {
        if !path.is_absolute() || path == Path::new(".") {
            return Err(blocked(format!(
                "{label} должен быть настроен абсолютным путём"
            )));
        }
        if is_too_broad(path) {
            return Err(blocked(format!("{label} слишком широкий")));
        }
        let metadata = fs::symlink_metadata(path)
            .map_err(|_| blocked(format!("{label} недоступен или не существует")))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(blocked(format!("{label} должен быть обычным каталогом")));
        }
        let canonical =
            fs::canonicalize(path).map_err(|_| blocked(format!("не удалось проверить {label}")))?;
        if is_too_broad(&canonical) {
            return Err(blocked(format!("{label} слишком широкий")));
        }
        if let Some(home) = home_dir {
            if home.is_absolute() {
                if let Ok(canonical_home) = fs::canonicalize(home) {
                    if canonical == canonical_home {
                        return Err(blocked(format!("{label} не может быть домашним каталогом")));
                    }
                }
            }
        }
        let fd = fd::open_root_directory(&canonical)
            .map_err(|_| blocked(format!("{label} должен быть обычным каталогом")))?;
        Ok(Self { fd })
    }

    pub(crate) fn fd(&self) -> &DirFd {
        &self.fd
    }

    /// Обходит существующих родителей без создания. `None` — родитель ещё не создан.
    pub(crate) fn open_existing_parent(
        &self,
        relative: &PersistentPath,
    ) -> Result<Option<DirFd>, AppError> {
        fd::walk_parent(&self.fd, relative, false)
    }

    /// Обходит и создаёт родителей по одному через `mkdirat` внутри корня.
    pub(crate) fn ensure_parent(&self, relative: &PersistentPath) -> Result<DirFd, AppError> {
        fd::walk_parent(&self.fd, relative, true)?
            .ok_or_else(|| io_error("не удалось создать целевую папку"))
    }

    pub(crate) fn verify_parent(
        &self,
        relative: &PersistentPath,
        parent: &DirFd,
    ) -> Result<(), AppError> {
        fd::verify_parent_identity(&self.fd, relative, parent)
    }
}

#[derive(Debug, Clone)]
pub struct VaultRoot(ControlledRoot);

impl VaultRoot {
    pub fn new(path: &Path, home_dir: Option<&Path>) -> Result<Self, AppError> {
        ControlledRoot::new(path, home_dir, "Корень vault").map(Self)
    }

    pub(crate) fn controlled(&self) -> &ControlledRoot {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct BackupRoot(ControlledRoot);

impl BackupRoot {
    pub fn new(path: &Path, home_dir: Option<&Path>) -> Result<Self, AppError> {
        ControlledRoot::new(path, home_dir, "Корень резервных копий").map(Self)
    }

    pub(crate) fn controlled(&self) -> &ControlledRoot {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct InboxRoot {
    root: ControlledRoot,
}

impl InboxRoot {
    /// Открывает папку входящих покомпонентно от fd vault; symlink в любом компоненте отклоняется.
    pub fn new(vault: &VaultRoot, path: PersistentPath) -> Result<Self, AppError> {
        let fd = fd::walk_directory(vault.0.fd(), &path)
            .map_err(|_| blocked("Контролируемая папка входящих небезопасна"))?
            .ok_or_else(|| blocked("Контролируемая папка входящих не существует"))?;
        Ok(Self {
            root: ControlledRoot { fd },
        })
    }

    pub(crate) fn controlled(&self) -> &ControlledRoot {
        &self.root
    }
}

/// Недоверенный source, открытый ровно один раз.
///
/// Хэш, размер и копирование читают один и тот же открытый inode через `pread`;
/// повторного разрешения строкового пути нет.
#[derive(Debug, Clone)]
pub struct SourceFile {
    file: Arc<File>,
    identity: Identity,
}

impl SourceFile {
    pub fn new(path: &Path) -> Result<Self, AppError> {
        if !path.is_absolute() {
            return Err(blocked("Исходный файл должен иметь абсолютный путь"));
        }
        let (file, stat) = fd::open_source_path(path)?;
        Ok(Self {
            file: Arc::new(file),
            identity: Identity::of(&stat),
        })
    }

    /// Открывает файл внутри контролируемого корня покомпонентно (для backup).
    pub(crate) fn open_in(destination: &VerifiedDestination) -> Result<Self, AppError> {
        let parent = destination
            .open_existing_parent()?
            .ok_or_else(|| BackendError::not_found("Исходный файл не найден"))?;
        match fd::open_leaf(&parent, destination.file_name())? {
            fd::Leaf::Missing => Err(BackendError::not_found("Исходный файл не найден")),
            fd::Leaf::NotRegular => Err(blocked("Источник должен быть обычным файлом")),
            fd::Leaf::File(file, stat) => {
                destination.verify_parent(&parent)?;
                Ok(Self {
                    file: Arc::new(file),
                    identity: Identity::of(&stat),
                })
            }
        }
    }

    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    /// Проверяет через `fstat` открытого дескриптора, что это всё ещё тот же regular inode.
    pub(crate) fn revalidate(&self) -> Result<u64, AppError> {
        let stat = rustix::fs::fstat(self.file.as_ref())
            .map_err(|_| io_error("Не удалось повторно проверить исходный файл"))?;
        if fd::file_type(&stat) != rustix::fs::FileType::RegularFile
            || Identity::of(&stat) != self.identity
        {
            return Err(blocked("Исходный путь больше не является обычным файлом"));
        }
        Ok(fd::stat_size(&stat))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct PersistentPath(String);

impl PersistentPath {
    pub fn new(value: impl AsRef<str>) -> Result<Self, AppError> {
        let value = value.as_ref();
        if value.is_empty()
            || value.starts_with('/')
            || value.starts_with('\\')
            || value.to_ascii_lowercase().starts_with("file://")
            || value.contains('\\')
            || value.as_bytes().get(1) == Some(&b':')
            || value.contains('\0')
        {
            return Err(blocked(
                "Persistent path должен быть безопасным относительным путём",
            ));
        }
        let mut normalized = Vec::new();
        for component in value.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(blocked("Persistent path содержит небезопасный компонент"));
            }
            if component.len() > 255 {
                return Err(blocked("Компонент persistent path слишком длинный"));
            }
            normalized.push(component);
        }
        let joined = normalized.join("/");
        let path = Path::new(&joined);
        if path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(blocked("Persistent path содержит небезопасный префикс"));
        }
        Ok(Self(joined))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    pub(crate) fn file_name(&self) -> &OsStr {
        self.as_path()
            .file_name()
            .expect("validated non-empty path")
    }

    pub(crate) fn with_file_name(&self, name: &str) -> Result<Self, AppError> {
        let parent = self
            .as_path()
            .parent()
            .filter(|path| !path.as_os_str().is_empty());
        let value = match parent {
            Some(parent) => format!("{}/{}", parent.to_string_lossy(), name),
            None => name.to_owned(),
        };
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for PersistentPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(D::Error::custom)
    }
}

/// Проверенный root + относительный путь. Абсолютный `PathBuf` наружу не выдаётся.
#[derive(Debug, Clone)]
pub struct VerifiedDestination {
    root: ControlledRoot,
    relative: PersistentPath,
}

impl VerifiedDestination {
    pub fn in_vault(root: &VaultRoot, relative: PersistentPath) -> Result<Self, AppError> {
        Self::new(root.controlled(), relative)
    }

    pub fn in_inbox(root: &InboxRoot, relative: PersistentPath) -> Result<Self, AppError> {
        Self::new(root.controlled(), relative)
    }

    pub fn in_backup(root: &BackupRoot, relative: PersistentPath) -> Result<Self, AppError> {
        Self::new(root.controlled(), relative)
    }

    fn new(root: &ControlledRoot, relative: PersistentPath) -> Result<Self, AppError> {
        root.open_existing_parent(&relative)?;
        Ok(Self {
            root: root.clone(),
            relative,
        })
    }

    pub fn relative(&self) -> &PersistentPath {
        &self.relative
    }

    pub(crate) fn file_name(&self) -> &OsStr {
        self.relative.file_name()
    }

    pub(crate) fn open_existing_parent(&self) -> Result<Option<DirFd>, AppError> {
        self.root.open_existing_parent(&self.relative)
    }

    pub(crate) fn ensure_parent(&self) -> Result<DirFd, AppError> {
        self.root.ensure_parent(&self.relative)
    }

    pub(crate) fn verify_parent(&self, parent: &DirFd) -> Result<(), AppError> {
        self.root.verify_parent(&self.relative, parent)
    }
}

fn is_too_broad(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }
    path.parent()
        .is_some_and(|parent| parent.parent().is_none())
}

pub(crate) fn blocked(message: impl Into<String>) -> AppError {
    BackendError::new(ErrorCode::Blocked, message)
}

pub(crate) fn io_error(message: impl Into<String>) -> AppError {
    BackendError::new(ErrorCode::IoError, message)
}
