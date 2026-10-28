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

    /// Проверяет fd-relative, что относительный путь существует и является
    /// обычным файлом или каталогом (без symlink в любом компоненте).
    pub fn ensure_existing_path(&self, relative: &PersistentPath) -> Result<(), AppError> {
        let parent = self
            .0
            .open_existing_parent(relative)?
            .ok_or_else(|| BackendError::not_found("Путь не найден в хранилище"))?;
        let stat = fd::stat_leaf(&parent, relative.file_name())?
            .ok_or_else(|| BackendError::not_found("Путь не найден в хранилище"))?;
        match fd::file_type(&stat) {
            rustix::fs::FileType::Directory | rustix::fs::FileType::RegularFile => {}
            _ => return Err(blocked("Путь не является обычным файлом или папкой")),
        }
        self.0.verify_parent(relative, &parent)
    }

    /// Создаёт каталог покомпонентно внутри vault, не следуя symlink.
    pub fn ensure_directory(&self, relative: &PersistentPath) -> Result<(), AppError> {
        fd::ensure_directory_path(self.0.fd(), relative)
            .map(|_| ())
            .map_err(|_| blocked("Не удалось безопасно создать папку в хранилище"))
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

/// Наблюдаемая папка загрузок: та же политика корня, только чтение.
///
/// Watcher читает лишь верхний уровень каталога через открытый fd
/// (`fdopendir` + `statat(AT_SYMLINK_NOFOLLOW)`), не спускается в подкаталоги и
/// не следует symlink. Файл открывается `openat(O_NOFOLLOW)` и сверяется с
/// identity, которую watcher видел при стабилизации.
#[derive(Debug, Clone)]
pub struct DownloadsRoot {
    root: ControlledRoot,
}

/// Наблюдение одного файла верхнего уровня папки загрузок.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadEntry {
    pub name: String,
    pub size_bytes: u64,
    /// Время изменения, наносекунды от эпохи (только для сравнения наблюдений).
    pub modified_ns: i128,
    pub identity: FileIdentity,
}

/// Непрозрачная идентичность inode (устройство + inode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    dev: u64,
    ino: u64,
}

impl FileIdentity {
    pub(crate) fn of(identity: Identity) -> Self {
        let (dev, ino) = identity.raw();
        Self { dev, ino }
    }

    fn as_identity(self) -> Identity {
        Identity::from_raw(self.dev, self.ino)
    }

    #[cfg(test)]
    pub fn for_tests(dev: u64, ino: u64) -> Self {
        Self { dev, ino }
    }
}

impl DownloadsRoot {
    pub fn new(path: &Path, home_dir: Option<&Path>) -> Result<Self, AppError> {
        ControlledRoot::new(path, home_dir, "Папка загрузок").map(|root| Self { root })
    }

    /// Перечисляет обычные файлы верхнего уровня. Каталоги, symlink, специальные
    /// файлы и имена не в UTF-8 пропускаются; содержимое файлов не читается.
    pub fn list_files(&self) -> Result<Vec<DownloadEntry>, AppError> {
        let entries = fd::list_regular_files(self.root.fd())?;
        Ok(entries
            .into_iter()
            .map(|(name, stat)| DownloadEntry {
                name,
                size_bytes: fd::stat_size(&stat),
                modified_ns: fd::stat_mtime_ns(&stat),
                identity: FileIdentity::of(Identity::of(&stat)),
            })
            .collect())
    }

    /// Открывает файл верхнего уровня по имени: `openat(O_NOFOLLOW)` в fd корня.
    /// Если inode отличается от наблюдавшегося, файл был подменён — `conflict`.
    pub fn open_file(&self, name: &str, expected: FileIdentity) -> Result<SourceFile, AppError> {
        if !is_plain_file_name(name) {
            return Err(blocked("Недопустимое имя файла в папке загрузок"));
        }
        match fd::open_leaf(self.root.fd(), OsStr::new(name))? {
            fd::Leaf::Missing => Err(BackendError::not_found("Файл исчез из папки загрузок")),
            fd::Leaf::NotRegular => {
                Err(blocked("Файл в папке загрузок не является обычным файлом"))
            }
            fd::Leaf::File(file, stat) => {
                let identity = Identity::of(&stat);
                if identity != expected.as_identity() {
                    return Err(BackendError::conflict(
                        "Файл в папке загрузок был заменён после обнаружения; проверьте его заново",
                    ));
                }
                Ok(SourceFile {
                    file: Arc::new(file),
                    identity,
                })
            }
        }
    }
}

/// Одноуровневое имя файла без разделителей, `.`/`..` и управляющих символов.
pub(crate) fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
}

/// Временный каталог распаковки (например, `tempfile::TempDir`), привязанный к fd.
///
/// Файлы создаются только `openat(O_CREAT | O_EXCL | O_NOFOLLOW)` внутри
/// подкаталогов, созданных `mkdirat`; перезапись и выход за пределы невозможны.
/// Удаление каталога целиком — забота владельца `TempDir`.
#[derive(Debug, Clone)]
pub struct ScratchRoot {
    root: ControlledRoot,
}

impl ScratchRoot {
    pub fn new(path: &Path) -> Result<Self, AppError> {
        ControlledRoot::new(path, None, "Временный каталог распаковки").map(|root| Self { root })
    }

    /// Создаёт новый файл по относительному пути. Существующий файл → `conflict`.
    pub fn create_file(&self, relative: &PersistentPath) -> Result<File, AppError> {
        let parent = self.root.ensure_parent(relative)?;
        let file = fd::create_exclusive(&parent, relative.file_name())?;
        self.root.verify_parent(relative, &parent)?;
        Ok(file)
    }

    /// Открывает ранее созданный файл как source для импорта (fd-relative, без symlink).
    pub fn open_source(&self, relative: &PersistentPath) -> Result<SourceFile, AppError> {
        let destination = VerifiedDestination {
            root: self.root.clone(),
            relative: relative.clone(),
        };
        SourceFile::open_in(&destination)
    }
}

#[derive(Debug, Clone)]
pub struct InboxRoot {
    root: ControlledRoot,
}

impl InboxRoot {
    /// Открывает или создаёт папку входящих покомпонентно (`openat`/`mkdirat`) от fd vault.
    /// Symlink или файл в любом компоненте отклоняется; за пределы vault выйти нельзя.
    pub fn open_or_create(vault: &VaultRoot, path: PersistentPath) -> Result<Self, AppError> {
        let fd = fd::ensure_directory_path(vault.0.fd(), &path)
            .map_err(|_| blocked("Папку «Входящие» нельзя безопасно открыть или создать"))?;
        Ok(Self {
            root: ControlledRoot { fd },
        })
    }

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

    /// Открывает недоверенный внешний файл из native picker/drop.
    ///
    /// Строже `new`: отклоняются `.`/`..`, symlink в любом компоненте родителя
    /// (кроме системных алиасов macOS `/var`, `/tmp`, `/etc` → `/private/...`),
    /// symlink в последнем компоненте, каталоги и специальные файлы. Канонический
    /// родитель открывается покомпонентно от `/` с `O_NOFOLLOW`, затем файл —
    /// `openat(O_NOFOLLOW)`. После открытия путь повторно сверяется с inode,
    /// чтобы подмена между проверкой и открытием дала `blocked`.
    pub fn open_external(path: &Path) -> Result<Self, AppError> {
        if !path.is_absolute() {
            return Err(blocked("Исходный файл должен иметь абсолютный путь"));
        }
        if path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        {
            return Err(blocked("Путь к исходному файлу содержит «.» или «..»"));
        }
        let name = path
            .file_name()
            .ok_or_else(|| blocked("Не указано имя исходного файла"))?;
        let parent = path
            .parent()
            .ok_or_else(|| blocked("Не указана папка исходного файла"))?;
        for ancestor in parent.ancestors() {
            if ancestor.parent().is_none() {
                break;
            }
            let metadata = fs::symlink_metadata(ancestor)
                .map_err(|_| BackendError::not_found("Папка исходного файла не найдена"))?;
            if metadata.file_type().is_symlink() && !is_system_alias(ancestor) {
                return Err(blocked(
                    "Путь к исходному файлу проходит через символическую ссылку",
                ));
            }
        }
        let canonical_parent = fs::canonicalize(parent)
            .map_err(|_| BackendError::not_found("Папка исходного файла не найдена"))?;
        let directory = fd::open_absolute_directory(&canonical_parent)?;
        let (file, stat) = match fd::open_leaf(&directory, name)? {
            fd::Leaf::Missing => return Err(BackendError::not_found("Исходный файл не найден")),
            fd::Leaf::NotRegular => {
                return Err(blocked(
                    "Источник должен быть обычным файлом, а не ссылкой, папкой или устройством",
                ))
            }
            fd::Leaf::File(file, stat) => (file, stat),
        };
        let identity = Identity::of(&stat);
        let current = fs::metadata(path)
            .map_err(|_| BackendError::not_found("Исходный файл исчез во время проверки"))?;
        if !same_identity(&current, identity) {
            return Err(blocked("Исходный файл был подменён во время проверки"));
        }
        Ok(Self {
            file: Arc::new(file),
            identity,
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

    /// Читает небольшой UTF-8 текст из уже проверенного inode. Используется
    /// только для явно выбранных пользователем текстовых импортов.
    pub fn read_utf8_limited(&self, limit: u64) -> Result<String, AppError> {
        use std::io::Read;
        let size = self.revalidate()?;
        if size > limit {
            return Err(blocked("Текстовый файл слишком большой"));
        }
        let mut file = self
            .file
            .try_clone()
            .map_err(|_| io_error("Не удалось прочитать файл"))?;
        let mut bytes = Vec::with_capacity(size as usize);
        file.read_to_end(&mut bytes)
            .map_err(|_| io_error("Не удалось прочитать файл"))?;
        String::from_utf8(bytes).map_err(|_| blocked("Файл должен быть текстом UTF-8"))
    }

    /// Время изменения открытого inode (нс), чтобы сверить с наблюдением watcher.
    pub fn modified_ns(&self) -> Result<i128, AppError> {
        let stat = rustix::fs::fstat(self.file.as_ref())
            .map_err(|_| io_error("Не удалось повторно проверить исходный файл"))?;
        Ok(fd::stat_mtime_ns(&stat))
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

/// Системные алиасы macOS: `/var`, `/tmp`, `/etc` — symlink на `/private/...`.
fn is_system_alias(path: &Path) -> bool {
    cfg!(target_os = "macos")
        && [Path::new("/var"), Path::new("/tmp"), Path::new("/etc")].contains(&path)
}

fn same_identity(metadata: &fs::Metadata, identity: Identity) -> bool {
    use std::os::unix::fs::MetadataExt;
    Identity::from_raw(metadata.dev(), metadata.ino()) == identity
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
