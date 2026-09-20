use crate::{core::{newapi, repository::Repository, router::{self, ImportResult, NewApiState, ProviderInput, ProviderView, StateChange}, router_proxy}, platform::paths::CodexPaths};
use std::sync::Mutex;
use tauri::State;

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo.lock().map_err(|_| "newapi.errors.storage")?.paths().clone())
}

#[tauri::command]
pub async fn load_newapi_state(repo: State<'_, Mutex<Repository>>) -> Result<NewApiState, String> {
    let paths = paths(&repo)?;
    let store = router::load_store(&paths)?;
    if store.router.enabled && !router_proxy::is_running() {
        // A busy port (e.g. the original app still running) must not blank out the page.
        if let Err(error) = start_proxy(&paths, store.router.port).await {
            eprintln!("[AiMaMi] router proxy startup failed: {error}");
        }
    }
    router::state(&paths, router_proxy::is_running())
}

async fn start_proxy(paths: &CodexPaths, port: u16) -> Result<(), String> {
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    router_proxy::ensure_running(port, paths.clone(), proxy).await.map(|_| ())
}

#[tauri::command]
pub fn save_newapi_provider(repo: State<'_, Mutex<Repository>>, provider: ProviderInput) -> Result<StateChange, String> {
    router::save_provider(&paths(&repo)?, provider)
}

#[tauri::command]
pub fn delete_newapi_provider(repo: State<'_, Mutex<Repository>>, id: String) -> Result<StateChange, String> {
    router::delete_provider(&paths(&repo)?, &id)
}

#[tauri::command]
pub fn set_newapi_provider_enabled(repo: State<'_, Mutex<Repository>>, id: String, enabled: bool) -> Result<StateChange, String> {
    router::set_enabled(&paths(&repo)?, &id, enabled)
}

#[tauri::command]
pub fn set_newapi_active_provider(repo: State<'_, Mutex<Repository>>, id: String) -> Result<StateChange, String> {
    router::set_active(&paths(&repo)?, &id)
}

#[tauri::command]
pub fn set_newapi_direct(repo: State<'_, Mutex<Repository>>, id: String) -> Result<StateChange, String> {
    let change = router::set_direct(&paths(&repo)?, &id)?;
    router_proxy::stop();
    Ok(change)
}

#[tauri::command]
pub async fn test_newapi_provider(repo: State<'_, Mutex<Repository>>, id: String) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    tauri::async_runtime::spawn_blocking(move || router::test_provider(&paths, &id, &proxy))
        .await
        .map_err(|_| "newapi.errors.network")?
}

#[tauri::command]
pub async fn fetch_newapi_models(
    repo: State<'_, Mutex<Repository>>,
    provider_id: Option<String>,
    base_url: String,
    api_key: String,
) -> Result<Vec<crate::core::newapi::Model>, String> {
    let paths = paths(&repo)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    tauri::async_runtime::spawn_blocking(move || {
        router::fetch_models(&paths, provider_id.as_deref(), &base_url, &api_key, &proxy)
    })
    .await
    .map_err(|_| "newapi.errors.network".to_string())?
}

#[tauri::command]
pub async fn query_newapi_provider_balance(repo: State<'_, Mutex<Repository>>, id: String) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    tauri::async_runtime::spawn_blocking(move || router::query_provider_balance(&paths, &id, &proxy))
        .await
        .map_err(|_| "newapi.errors.network")?
}

#[tauri::command]
pub async fn set_newapi_smart_routing(repo: State<'_, Mutex<Repository>>, enabled: bool) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let was_running = router_proxy::is_running();
    if enabled {
        let port = router::load_store(&paths)?.router.port;
        if let Err(error) = start_proxy(&paths, port).await {
            if !was_running {
                router_proxy::stop();
            }
            return Err(error);
        }
    }
    match router::set_smart_routing(&paths, enabled) {
        Ok(change) => {
            if !enabled {
                router_proxy::stop();
            }
            Ok(change)
        }
        Err(error) => {
            if enabled && !was_running {
                router_proxy::stop();
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub fn set_newapi_accountless(repo: State<'_, Mutex<Repository>>, enabled: bool) -> Result<StateChange, String> {
    router::set_accountless(&paths(&repo)?, enabled)
}

#[tauri::command]
pub fn import_newapi_providers(repo: State<'_, Mutex<Repository>>, path: String) -> Result<ImportResult, String> {
    router::import_providers(&paths(&repo)?, std::path::Path::new(&path))
}

#[tauri::command]
pub fn export_newapi_providers(repo: State<'_, Mutex<Repository>>, path: String) -> Result<(), String> {
    router::export_providers(&paths(&repo)?, std::path::Path::new(&path))
}
