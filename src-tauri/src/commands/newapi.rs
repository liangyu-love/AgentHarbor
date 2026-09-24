use crate::{
    core::{
        newapi,
        repository::Repository,
        router::{
            self, ImportResult, ModelProtocol, NewApiState, ProviderInput, ProviderView,
            StateChange,
        },
        router_proxy, routing_runtime,
    },
    platform::paths::CodexPaths,
};
use std::sync::Mutex;
use tauri::State;

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "newapi.errors.storage")?
        .paths()
        .clone())
}

#[tauri::command]
pub async fn load_newapi_state(repo: State<'_, Mutex<Repository>>) -> Result<NewApiState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let store = router::load_store(&paths)?;
    match routing_runtime::reconcile(&paths).await {
        Ok(()) if store.router.enabled => {
            // Publish the Codex config only after the local route accepts requests.
            if let Err(error) = router::ensure_routing_config(&paths, &store) {
                eprintln!("[AgentHarbor] router config repair failed: {error}");
            }
        }
        Err(error) => {
            eprintln!("[AgentHarbor] router proxy startup failed: {error}");
            if store.router.enabled {
                let _ = router::remove_routing_config(&paths, &store);
            }
        }
        Ok(()) => {}
    }
    // Read the store again: `ensure_routing_config` may have just rewritten the
    // Codex config, and the drift flag must reflect the file's final state.
    let store = router::load_store(&paths).unwrap_or(store);
    Ok(router::state_from_store_at(
        &store,
        router_proxy::is_running(),
        Some(&paths),
    ))
}

#[tauri::command]
pub fn save_newapi_provider(
    repo: State<'_, Mutex<Repository>>,
    provider: ProviderInput,
) -> Result<StateChange, String> {
    router::save_provider(&paths(&repo)?, provider)
}

#[tauri::command]
pub fn delete_newapi_provider(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<StateChange, String> {
    router::delete_provider(&paths(&repo)?, &id)
}

#[tauri::command]
pub fn set_newapi_provider_enabled(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    enabled: bool,
) -> Result<StateChange, String> {
    router::set_enabled(&paths(&repo)?, &id, enabled)
}

#[tauri::command]
pub fn set_newapi_provider_network(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    mode: String,
) -> Result<StateChange, String> {
    router::set_network_mode(&paths(&repo)?, &id, &mode)
}

#[tauri::command]
pub async fn test_newapi_provider(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    model: String,
) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    tauri::async_runtime::spawn_blocking(move || router::test_provider(&paths, &id, &model, &proxy))
        .await
        .map_err(|_| "newapi.errors.network")?
}

#[tauri::command]
pub async fn fetch_newapi_models(
    repo: State<'_, Mutex<Repository>>,
    provider_id: Option<String>,
    base_url: String,
    api_key: String,
    protocol: Option<ModelProtocol>,
) -> Result<Vec<crate::core::newapi::Model>, String> {
    let paths = paths(&repo)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    tauri::async_runtime::spawn_blocking(move || {
        router::fetch_models(
            &paths,
            provider_id.as_deref(),
            &base_url,
            &api_key,
            protocol.unwrap_or_default(),
            &proxy,
        )
    })
    .await
    .map_err(|_| "newapi.errors.network".to_string())?
}

#[tauri::command]
pub async fn query_newapi_provider_balance(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    tauri::async_runtime::spawn_blocking(move || {
        router::query_provider_balance(&paths, &id, &proxy)
    })
    .await
    .map_err(|_| "newapi.errors.network")?
}

#[tauri::command]
pub async fn set_newapi_smart_routing(
    repo: State<'_, Mutex<Repository>>,
    enabled: bool,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    if enabled {
        routing_runtime::ensure_available(&paths).await?;
    }
    let mut change = router::set_smart_routing(&paths, enabled)?;
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        if enabled {
            let _ = router::set_smart_routing(&paths, false);
            let _ = routing_runtime::reconcile(&paths).await;
        }
        return Err(error);
    }
    change.state.proxy_running = router_proxy::is_running();
    Ok(change)
}

#[tauri::command]
pub fn set_newapi_accountless(
    repo: State<'_, Mutex<Repository>>,
    enabled: bool,
) -> Result<StateChange, String> {
    router::set_accountless(&paths(&repo)?, enabled)
}

#[tauri::command]
pub fn import_newapi_providers(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<ImportResult, String> {
    router::import_providers(&paths(&repo)?, std::path::Path::new(&path))
}

#[tauri::command]
pub fn export_newapi_providers(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<(), String> {
    router::export_providers(&paths(&repo)?, std::path::Path::new(&path))
}
