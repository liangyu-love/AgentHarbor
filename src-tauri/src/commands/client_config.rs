use crate::core::{
    client_config::{
        ClaudeOfficialPreview, ClientPaths, ClientStatus, ConfigApplied, ConfigManager,
        ConfigPreview, ConfigRequest, Network,
    },
    models::ApiProxyMode,
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
            applied
                .warnings
                .push("clients.errors.routerReconcile".into());
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

/// The manager for writing or checking direct configurations, which follow
/// the app proxy setting and the terminal proxy.
fn configuring(paths: &CodexPaths) -> ConfigManager {
    let proxy = crate::core::newapi::proxy_from_settings(&paths.settings_path);
    let system = proxy.mode == ApiProxyMode::System
        && !crate::platform::proxy::detect_system_proxy_candidates().is_empty();
    manager(paths).with_network(Network {
        proxy,
        terminal: crate::core::terminal_proxy::active_url(paths),
        system,
    })
}

/// Each client's key files for the dashboard. Existence checks only.
#[tauri::command]
pub async fn list_client_files(
    repo: State<'_, Mutex<Repository>>,
) -> Result<Vec<crate::core::client_files::ClientFiles>, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        let desktop = crate::core::claude_desktop::Manager::new(&paths.app_data_dir).current_paths();
        crate::core::client_files::collect(&paths, &ClientPaths::discover(), desktop.as_ref())
    })
    .await
    .map_err(|_| "clients.errors.storage".to_owned())
}

#[tauri::command]
pub fn list_client_configs(
    repo: State<'_, Mutex<Repository>>,
) -> Result<Vec<ClientStatus>, String> {
    let paths = paths(&repo)?;
    // An unreadable store only skips the check for changed relay cards.
    let store = router::load_store(&paths).ok();
    Ok(configuring(&paths).list_with(store.as_ref()))
}

#[tauri::command]
pub fn preview_client_config(
    repo: State<'_, Mutex<Repository>>,
    request: ConfigRequest,
    replace_conflicts: Option<bool>,
) -> Result<ConfigPreview, String> {
    let paths = paths(&repo)?;
    configuring(&paths).preview_with_conflicts(
        &router::load_store(&paths)?,
        &request,
        replace_conflicts.unwrap_or(false),
    )
}

#[tauri::command]
pub async fn apply_client_config(
    repo: State<'_, Mutex<Repository>>,
    request: ConfigRequest,
    revision: String,
    replace_conflicts: Option<bool>,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    if request.needs_router() {
        routing_runtime::ensure_available(&paths).await?;
    }
    let result = router::load_store(&paths).and_then(|store| {
        configuring(&paths).apply_with_conflicts(
            &store,
            &request,
            &revision,
            replace_conflicts.unwrap_or(false),
        )
    });
    attach_reconcile_warning(result, routing_runtime::reconcile(&paths).await)
}

#[tauri::command]
pub async fn restore_client_config(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
    replace_conflicts: Option<bool>,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let result = manager(&paths).restore_confirmed(target_id, replace_conflicts.unwrap_or(false));
    attach_reconcile_warning(result, routing_runtime::reconcile(&paths).await)
}

#[tauri::command]
pub fn preview_claude_code_official_config(
    repo: State<'_, Mutex<Repository>>,
) -> Result<ClaudeOfficialPreview, String> {
    manager(&paths(&repo)?).preview_claude_official()
}

/// Withdraw third-party CLI settings without replacing the saved official login.
#[tauri::command]
pub async fn switch_claude_code_official_config(
    repo: State<'_, Mutex<Repository>>,
    revision: String,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let result = manager(&paths).switch_claude_official(&revision);
    attach_reconcile_warning(result, routing_runtime::reconcile(&paths).await)
}

/// Re-inject the last applied configuration. With `rebaseline`, the file's
/// current managed values are adopted as the rollback point first.
#[tauri::command]
pub async fn reapply_client_config(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
    rebaseline: Option<bool>,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let rebaseline = rebaseline.unwrap_or(false);
    if manager(&paths)
        .stored_request(target_id)?
        .is_some_and(|request| request.needs_router())
    {
        routing_runtime::ensure_available(&paths).await?;
    }
    let result = router::load_store(&paths)
        .and_then(|store| configuring(&paths).reapply(&store, target_id, rebaseline));
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
                warnings: Vec::new(),
            }),
            Err("proxy unavailable".into()),
        )
        .unwrap();

        assert_eq!(result.path, "client.json");
        assert!(result
            .warnings
            .contains(&"clients.errors.routerReconcile".to_owned()));
    }

    #[test]
    fn write_error_is_preserved_when_reconcile_also_fails() {
        let result =
            attach_reconcile_warning(Err("write failed".into()), Err("proxy unavailable".into()));

        assert!(matches!(result, Err(error) if error == "write failed"));
    }
}
