use tauri::State;

use crate::{
    app_state::AppState, domain::BackendHealth, error::BackendError, repositories::Repositories,
};

#[tauri::command]
pub fn backend_health(state: State<'_, AppState>) -> Result<BackendHealth, BackendError> {
    state.database().ping()?;
    let schema_version = Repositories::new(state.database()).schema_version()?;
    let _safe_state_probe = (
        state.app_data_dir().is_absolute(),
        state.events().registered_services(),
    );

    Ok(BackendHealth {
        ready: true,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_version,
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        database_exists: state.database().exists(),
    })
}
