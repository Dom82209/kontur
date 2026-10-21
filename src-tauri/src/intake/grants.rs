//! Краткоживущие разрешения на чтение внешних файлов.
//!
//! WebView не может попросить backend прочитать произвольный путь: импорт
//! принимает только пути, которые backend сам получил из системного диалога
//! или из native drag-and-drop события окна. Разрешение живёт в памяти
//! процесса, не сохраняется в SQLite и истекает через [`GRANT_TTL`].

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

pub const GRANT_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_GRANTS: usize = 4096;

#[derive(Debug, Default)]
pub struct IntakeGrants {
    entries: Mutex<HashMap<PathBuf, Instant>>,
}

impl IntakeGrants {
    pub fn new() -> Self {
        Self::default()
    }

    /// Регистрирует пути, полученные из native picker или drop.
    pub fn grant<I, P>(&self, paths: I)
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let now = Instant::now();
        entries.retain(|_, granted| now.duration_since(*granted) < GRANT_TTL);
        for path in paths {
            let path = path.into();
            if path.is_absolute() {
                entries.insert(path, now);
            }
        }
        if entries.len() > MAX_GRANTS {
            let mut oldest: Vec<_> = entries.iter().map(|(p, t)| (p.clone(), *t)).collect();
            oldest.sort_by_key(|(_, granted)| *granted);
            for (path, _) in oldest.into_iter().take(entries.len() - MAX_GRANTS) {
                entries.remove(&path);
            }
        }
    }

    /// Повторное использование до истечения TTL разрешено: это делает повтор
    /// команды импорта идемпотентным.
    pub fn is_granted(&self, path: &Path) -> bool {
        let Ok(entries) = self.entries.lock() else {
            return false;
        };
        entries
            .get(path)
            .is_some_and(|granted| granted.elapsed() < GRANT_TTL)
    }

    #[cfg(test)]
    pub fn granted_count(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::IntakeGrants;

    #[test]
    fn only_absolute_granted_paths_are_accepted() {
        let grants = IntakeGrants::new();
        grants.grant(["/tmp/a.pdf", "relative.pdf"]);
        assert!(grants.is_granted(Path::new("/tmp/a.pdf")));
        assert!(!grants.is_granted(Path::new("relative.pdf")));
        assert!(!grants.is_granted(Path::new("/tmp/b.pdf")));
        assert_eq!(grants.granted_count(), 1);
    }
}
