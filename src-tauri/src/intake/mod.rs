//! Этап 5: управляемые каталоги, native picker/drop и импорт во «Входящие».
//!
//! Модуль не зависит от GUI: системный диалог скрыт за [`FilePicker`], события —
//! за callback прогресса, поэтому весь сценарий тестируется на `TempDir`.

mod grants;
mod importer;
mod naming;
mod roots;
mod store;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde::Serialize;

pub use grants::{IntakeGrants, GRANT_TTL};
pub use importer::{
    stored_import_report, validate_operation_id, ImportItemInput, ImportItemReport, ImportOrigin,
    ImportOutcome, ImportProgress, InboxImportReport, InboxImporter, MAX_IMPORT_BYTES,
    MAX_IMPORT_ITEMS,
};
pub use naming::{detect_kind, is_risky_to_open, numbered_name, safe_inbox_name};
pub use roots::{
    inbox_path, ImportRoots, ManagedRootsConfig, ManagedRootsDto, RootKind, RootSelectionDto,
    RootState, RootStatusDto, RootsService, INBOX_RELATIVE,
};
pub use store::inbox_relative_path;

/// Абстракция системного диалога. Production-реализация — официальный
/// `tauri-plugin-dialog`, вызываемый из Rust; в тестах — заглушка.
/// `None` означает, что пользователь закрыл диалог.
pub trait FilePicker {
    fn pick_files(&self, start: Option<&Path>) -> Option<Vec<PathBuf>>;
    fn pick_folder(&self, title: &str, start: Option<&Path>) -> Option<PathBuf>;
}

/// Элемент native intake для frontend (`DroppedItem`). `sourcePath` транзитный и не сохраняется.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DroppedItemDto {
    pub name: String,
    pub size_bytes: u64,
    pub source_path: String,
}

/// Регистрирует пути из native picker/drop и превращает их в `DroppedItem`.
/// Каталоги, symlink и неабсолютные пути отбрасываются сразу.
pub fn register_native_paths(grants: &IntakeGrants, paths: Vec<PathBuf>) -> Vec<DroppedItemDto> {
    let mut items = Vec::with_capacity(paths.len());
    let mut granted = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(source_path) = path.to_str().map(str::to_owned) else {
            continue;
        };
        if !path.is_absolute() {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        items.push(DroppedItemDto {
            name: naming::display_name(path.file_name()),
            size_bytes: metadata.len(),
            source_path,
        });
        granted.push(path);
    }
    grants.grant(granted);
    items
}

/// Открывает системный выбор нескольких файлов. Отмена — пустой список, без разрешений.
pub fn pick_files_for_import(
    picker: &dyn FilePicker,
    grants: &IntakeGrants,
    start: Option<&Path>,
) -> Vec<DroppedItemDto> {
    match picker.pick_files(start) {
        None => Vec::new(),
        Some(paths) => register_native_paths(grants, paths),
    }
}

/// Выбор управляемого корня. Отмена не меняет настройки.
pub fn choose_root(
    service: &RootsService<'_>,
    picker: &dyn FilePicker,
    kind: RootKind,
) -> Result<RootSelectionDto, crate::error::AppError> {
    let title = match kind {
        RootKind::Vault => "Выберите хранилище Obsidian",
        RootKind::Backup => "Выберите папку резервных копий",
        RootKind::Downloads => "Выберите папку загрузок",
        RootKind::Inbox => {
            return Err(crate::error::BackendError::blocked(format!(
                "«Входящие» находятся внутри хранилища: {INBOX_RELATIVE}"
            )))
        }
    };
    match picker.pick_folder(title, None) {
        None => Ok(RootSelectionDto {
            cancelled: true,
            roots: service.status()?,
        }),
        Some(path) => Ok(RootSelectionDto {
            cancelled: false,
            roots: service.set_root(kind, &path)?,
        }),
    }
}
