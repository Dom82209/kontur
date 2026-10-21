use std::{
    ffi::OsStr,
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::error::{AppError, BackendError, ErrorCode};

#[derive(Debug, Clone)]
pub(crate) struct ControlledRoot {
    canonical: PathBuf,
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
        Ok(Self { canonical })
    }

    fn validate_existing_parents(&self, relative: &PersistentPath) -> Result<(), AppError> {
        let mut current = self.canonical.clone();
        let components: Vec<_> = relative.as_path().components().collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            let Component::Normal(component) = component else {
                return Err(blocked("небезопасный компонент целевого пути"));
            };
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(blocked("целевая папка небезопасна"));
                    }
                    let canonical = fs::canonicalize(&current)
                        .map_err(|_| blocked("не удалось проверить целевую папку"))?;
                    if !canonical.starts_with(&self.canonical) {
                        return Err(blocked("целевая папка выходит за разрешённый корень"));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => return Err(io_error("не удалось проверить целевую папку")),
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_parent(&self, relative: &PersistentPath) -> Result<PathBuf, AppError> {
        let mut current = self.canonical.clone();
        let components: Vec<_> = relative.as_path().components().collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            let Component::Normal(component) = component else {
                return Err(blocked("небезопасный компонент целевого пути"));
            };
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(blocked("целевая папка небезопасна"));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&current)
                        .map_err(|_| io_error("не удалось создать целевую папку"))?;
                }
                Err(_) => return Err(io_error("не удалось проверить целевую папку")),
            }
            let canonical = fs::canonicalize(&current)
                .map_err(|_| blocked("не удалось проверить целевую папку"))?;
            if !canonical.starts_with(&self.canonical) {
                return Err(blocked("целевая папка выходит за разрешённый корень"));
            }
        }
        self.validate_existing_parents(relative)?;
        Ok(current)
    }

    pub(crate) fn absolute(&self, relative: &PersistentPath) -> PathBuf {
        self.canonical.join(relative.as_path())
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
    pub fn new(vault: &VaultRoot, path: PersistentPath) -> Result<Self, AppError> {
        let absolute = vault.0.absolute(&path);
        vault.0.validate_existing_parents(&path)?;
        let metadata = fs::symlink_metadata(&absolute)
            .map_err(|_| blocked("Контролируемая папка входящих не существует"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(blocked("Контролируемая папка входящих небезопасна"));
        }
        let canonical = fs::canonicalize(&absolute)
            .map_err(|_| blocked("Не удалось проверить папку входящих"))?;
        if canonical == vault.0.canonical || !canonical.starts_with(&vault.0.canonical) {
            return Err(blocked("Папка входящих должна находиться внутри vault"));
        }
        Ok(Self {
            root: ControlledRoot { canonical },
        })
    }

    pub(crate) fn controlled(&self) -> &ControlledRoot {
        &self.root
    }
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    canonical: PathBuf,
}

impl SourceFile {
    pub fn new(path: &Path) -> Result<Self, AppError> {
        if !path.is_absolute() {
            return Err(blocked("Исходный файл должен иметь абсолютный путь"));
        }
        let metadata = fs::symlink_metadata(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => BackendError::not_found("Исходный файл не найден"),
            _ => io_error("Не удалось проверить исходный файл"),
        })?;
        if metadata.file_type().is_symlink() {
            return Err(blocked(
                "Символические ссылки нельзя использовать как источник",
            ));
        }
        if !metadata.is_file() {
            return Err(blocked("Источник должен быть обычным файлом"));
        }
        let canonical =
            fs::canonicalize(path).map_err(|_| io_error("Не удалось проверить исходный файл"))?;
        Ok(Self { canonical })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.canonical
    }

    pub(crate) fn revalidate(&self) -> Result<u64, AppError> {
        let metadata =
            fs::symlink_metadata(&self.canonical).map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => BackendError::not_found("Исходный файл не найден"),
                _ => io_error("Не удалось повторно проверить исходный файл"),
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(blocked("Исходный путь больше не является обычным файлом"));
        }
        Ok(metadata.len())
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
        root.validate_existing_parents(&relative)?;
        Ok(Self {
            root: root.clone(),
            relative,
        })
    }

    pub fn relative(&self) -> &PersistentPath {
        &self.relative
    }

    pub(crate) fn absolute(&self) -> PathBuf {
        self.root.absolute(&self.relative)
    }

    pub(crate) fn ensure_parent(&self) -> Result<PathBuf, AppError> {
        self.root.ensure_parent(&self.relative)
    }

    pub(crate) fn revalidate_parents(&self) -> Result<(), AppError> {
        self.root.validate_existing_parents(&self.relative)
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
