//! Поиск официального CLI провайдера. Исполняемый файл ищется только в
//! фиксированных каталогах установки (не через `PATH`), его канонический путь
//! обязан остаться внутри разрешённых префиксов, быть обычным исполняемым
//! файлом, не доступным на запись всем и принадлежащим пользователю или root.

use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executable {
    /// Канонический путь — именно он запускается (без повторного поиска).
    pub path: PathBuf,
    /// Отображаемое имя без домашнего каталога.
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    Found(Executable),
    Missing,
    /// Файл найден, но отклонён проверкой безопасности.
    Rejected(String),
}

#[derive(Debug, Clone)]
pub struct CliLocator {
    dirs: Vec<PathBuf>,
    prefixes: Vec<PathBuf>,
    home: Option<PathBuf>,
}

impl CliLocator {
    /// Каталоги официальных способов установки на macOS: Homebrew (Apple
    /// Silicon и Intel), MacPorts, нативный установщик Claude Code
    /// (`~/.local/bin`) и глобальный npm-префикс пользователя.
    pub fn system(home: Option<&Path>) -> Self {
        let mut dirs = vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/local/bin"),
        ];
        let mut prefixes = vec![
            PathBuf::from("/opt/homebrew"),
            PathBuf::from("/usr/local"),
            PathBuf::from("/opt/local"),
        ];
        if let Some(home) = home {
            dirs.push(home.join(".local/bin"));
            dirs.push(home.join(".npm-global/bin"));
            prefixes.push(home.join(".local"));
            prefixes.push(home.join(".npm-global"));
        }
        Self {
            dirs,
            prefixes,
            home: home.map(Path::to_path_buf),
        }
    }

    /// Произвольные каталоги — только для тестов в TempDir.
    pub fn with_dirs(dirs: Vec<PathBuf>, prefixes: Vec<PathBuf>, home: Option<PathBuf>) -> Self {
        Self {
            dirs,
            prefixes,
            home,
        }
    }

    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    fn display(&self, candidate: &Path) -> String {
        if let Some(home) = &self.home {
            if let Ok(rest) = candidate.strip_prefix(home) {
                return format!("~/{}", rest.display());
            }
        }
        candidate
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn find(&self, name: &str) -> Located {
        let mut rejection = None;
        for dir in &self.dirs {
            let candidate = dir.join(name);
            if std::fs::symlink_metadata(&candidate).is_err() {
                continue;
            }
            match self.verify(&candidate) {
                Ok(path) => {
                    return Located::Found(Executable {
                        path,
                        display: self.display(&candidate),
                    })
                }
                Err(reason) => rejection = Some(reason),
            }
        }
        rejection.map_or(Located::Missing, Located::Rejected)
    }

    fn verify(&self, candidate: &Path) -> Result<PathBuf, String> {
        let canonical = std::fs::canonicalize(candidate)
            .map_err(|_| "исполняемый файл недоступен (битая ссылка)".to_owned())?;
        let allowed = self.prefixes.iter().any(|prefix| {
            std::fs::canonicalize(prefix).is_ok_and(|prefix| canonical.starts_with(prefix))
        });
        if !allowed {
            return Err("исполняемый файл указывает за пределы каталогов установки".to_owned());
        }
        let metadata = std::fs::metadata(&canonical)
            .map_err(|_| "не удалось прочитать атрибуты исполняемого файла".to_owned())?;
        if !metadata.is_file() {
            return Err("найден не файл".to_owned());
        }
        let mode = metadata.permissions().mode();
        if mode & 0o111 == 0 {
            return Err("файл не исполняемый".to_owned());
        }
        if mode & 0o002 != 0 {
            return Err("файл доступен на запись всем пользователям".to_owned());
        }
        let uid = rustix::process::getuid().as_raw();
        if metadata.uid() != uid && metadata.uid() != 0 {
            return Err("файл принадлежит другому пользователю".to_owned());
        }
        if let Some(parent) = canonical.parent() {
            let parent_mode = std::fs::metadata(parent)
                .map(|item| item.permissions().mode())
                .unwrap_or(0o002);
            if parent_mode & 0o002 != 0 && parent_mode & 0o1000 == 0 {
                return Err("каталог исполняемого файла доступен на запись всем".to_owned());
            }
        }
        Ok(canonical)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::{CliLocator, Located};

    fn script(path: &std::path::Path, mode: u32) {
        fs::write(path, "#!/bin/sh\necho ok\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[test]
    fn finds_only_safe_executables_inside_allowed_prefixes() {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let bin = root.join("prefix/bin");
        fs::create_dir_all(&bin).unwrap();
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let locator = CliLocator::with_dirs(
            vec![bin.clone()],
            vec![root.join("prefix")],
            Some(root.clone()),
        );

        assert_eq!(locator.find("codex"), Located::Missing);

        script(&bin.join("codex"), 0o755);
        match locator.find("codex") {
            Located::Found(exe) => {
                assert_eq!(exe.path, bin.join("codex"));
                assert_eq!(exe.display, "~/prefix/bin/codex");
            }
            other => panic!("unexpected {other:?}"),
        }

        script(&bin.join("noexec"), 0o644);
        assert!(matches!(locator.find("noexec"), Located::Rejected(_)));

        script(&bin.join("writable"), 0o777);
        assert!(matches!(locator.find("writable"), Located::Rejected(_)));

        script(&outside.join("claude"), 0o755);
        std::os::unix::fs::symlink(outside.join("claude"), bin.join("claude")).unwrap();
        assert!(
            matches!(locator.find("claude"), Located::Rejected(_)),
            "symlink out of prefix"
        );

        fs::create_dir(bin.join("dir")).unwrap();
        assert!(matches!(locator.find("dir"), Located::Rejected(_)));
    }
}
