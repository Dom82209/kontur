//! Tauri-граница этапа 2: подключение существующего vault.
//!
//! Preview только читает vault (SHA-256 всех файлов), поэтому выполняется в
//! `spawn_blocking`. Apply и dry-run берут общий замок файловых мутаций, чтобы
//! раскладка или импорт не шли параллельно с проверкой плана.

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    adoption::{
        AdoptionService, CategoryOverride, VaultAdoptionPreviewDto, VaultAdoptionReportDto,
    },
    app_state::AppState,
    error::{AppError, ErrorCode},
};

async fn blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| AppError::new(ErrorCode::Unknown, "Фоновая задача прервана"))?
}

#[tauri::command]
pub async fn preview_vault_adoption<R: Runtime>(
    app: AppHandle<R>,
) -> Result<VaultAdoptionPreviewDto, AppError> {
    blocking(move || {
        let state = app.state::<AppState>();
        AdoptionService::new(state.database(), state.home_dir()).preview()
    })
    .await
}

#[tauri::command]
pub async fn apply_vault_adoption<R: Runtime>(
    app: AppHandle<R>,
    plan_id: String,
    overrides: Vec<CategoryOverride>,
    dry_run: bool,
) -> Result<VaultAdoptionReportDto, AppError> {
    blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.lock_file_mutations()?;
        AdoptionService::new(state.database(), state.home_dir())
            .apply(&plan_id, &overrides, dry_run)
    })
    .await
}

#[tauri::command]
pub async fn cancel_vault_adoption<R: Runtime>(
    app: AppHandle<R>,
    plan_id: String,
) -> Result<(), AppError> {
    blocking(move || {
        let state = app.state::<AppState>();
        AdoptionService::new(state.database(), state.home_dir()).cancel(&plan_id)
    })
    .await
}
