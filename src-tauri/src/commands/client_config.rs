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

fn attach_reconcile_warning(
    result: Result<ConfigApplied, String>,
    reconcile: Result<(), String>,
) -> Result<ConfigApplied, String> {
    match (result, reconcile) {
        (Ok(mut applied), Err(error)) => {
            log::warn!("client config was applied but router reconciliation failed: {error}");
            applied.warning = Some("clients.errors.routerReconcile".into());
            Ok(applied)
        }
        (result, _) => result,
    }
}

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "clients.errors.storage")?
        .paths()
        .clone())
}

fn manager(paths: &CodexPaths) -> ConfigManager {
    ConfigManager::new(&paths.app_data_dir, ClientPaths::discover())
        .with_official_home(&paths.codex_home)
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
    if request.needs_router() {
        routing_runtime::ensure_available(&paths).await?;
    }
    let result = router::load_store(&paths)
        .and_then(|store| manager(&paths).apply(&store, &request, &revision));
    attach_reconcile_warning(result, routing_runtime::reconcile(&paths).await)
}

#[tauri::command]
pub async fn restore_client_config(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let result = manager(&paths).restore(target_id);
    attach_reconcile_warning(result, routing_runtime::reconcile(&paths).await)
}

/// Drop AgentHarbor's management record, leaving the client file as it is.
#[tauri::command]
pub async fn release_client_config(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    manager(&paths).release(target_id)?;
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        log::warn!("client config was released but router reconciliation failed: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_write_remains_successful_when_reconcile_fails() {
        let result = attach_reconcile_warning(
            Ok(ConfigApplied {
                path: "client.json".into(),
                backup_path: "backup.dat".into(),
                warning: None,
            }),
            Err("proxy unavailable".into()),
        )
        .unwrap();

        assert_eq!(result.path, "client.json");
        assert_eq!(
            result.warning.as_deref(),
            Some("clients.errors.routerReconcile")
        );
    }

    #[test]
    fn write_error_is_preserved_when_reconcile_also_fails() {
        let result =
            attach_reconcile_warning(Err("write failed".into()), Err("proxy unavailable".into()));

        assert!(matches!(result, Err(error) if error == "write failed"));
    }
}
