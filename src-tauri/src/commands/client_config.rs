use crate::core::{
    client_config::{
        ClientPaths, ClientStatus, ConfigApplied, ConfigManager, ConfigPreview, ConfigRequest,
    },
    repository::Repository,
    router, routing_runtime,
    targets::TargetId,
};
use crate::platform::paths::CodexPaths;
use std::sync::Mutex;
use tauri::State;

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "clients.errors.storage")?
        .paths()
        .clone())
}

fn manager(paths: &CodexPaths) -> ConfigManager {
    ConfigManager::new(&paths.app_data_dir, ClientPaths::discover())
}

#[tauri::command]
pub fn list_client_configs(
    repo: State<'_, Mutex<Repository>>,
) -> Result<Vec<ClientStatus>, String> {
    Ok(manager(&paths(&repo)?).list())
}

#[tauri::command]
pub fn preview_client_config(
    repo: State<'_, Mutex<Repository>>,
    request: ConfigRequest,
) -> Result<ConfigPreview, String> {
    let paths = paths(&repo)?;
    manager(&paths).preview(&router::load_store(&paths)?, &request)
}

#[tauri::command]
pub async fn apply_client_config(
    repo: State<'_, Mutex<Repository>>,
    request: ConfigRequest,
    revision: String,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    routing_runtime::ensure_available(&paths).await?;
    let result = router::load_store(&paths)
        .and_then(|store| manager(&paths).apply(&store, &request, &revision));
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        if result.is_ok() {
            return Err(error);
        }
    }
    result
}

#[tauri::command]
pub async fn restore_client_config(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let result = manager(&paths).restore(target_id);
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        if result.is_ok() {
            return Err(error);
        }
    }
    result
}
