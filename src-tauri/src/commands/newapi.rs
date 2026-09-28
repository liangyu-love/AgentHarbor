use crate::{
    core::{
        newapi,
        repository::Repository,
        router::{
            self, CodexAccessMode, ImportResult, ModelProtocol, NewApiState, ProviderInput,
            ProviderView, StateChange,
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
    // Opening the page only observes Codex's live configuration. Injection is
    // reserved for the explicit enable action.
    let store = router::load_store(&paths)?;
    Ok(router::state_from_store_at(
        &store,
        router_proxy::is_running(),
        Some(&paths),
    ))
}

#[tauri::command]
pub async fn save_newapi_provider(
    repo: State<'_, Mutex<Repository>>,
    provider: ProviderInput,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::save_provider(&paths, provider)
}

#[tauri::command]
pub async fn delete_newapi_provider(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::delete_provider(&paths, &id)
}

#[tauri::command]
pub async fn set_newapi_provider_enabled(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    enabled: bool,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::set_enabled(&paths, &id, enabled)
}

#[tauri::command]
pub async fn set_newapi_provider_network(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    mode: String,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::set_network_mode(&paths, &id, &mode)
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
    let previous = router::load_store(&paths)?;
    let previously_owned = router::active_routing_config_is_ours(&paths);
    if enabled && previous.codex_access_mode == CodexAccessMode::Router {
        routing_runtime::ensure_available(&paths).await?;
    }
    let mut change = match router::set_smart_routing(&paths, enabled) {
        Ok(change) => change,
        Err(error) => {
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
    };
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        if enabled && change.state.router.access_mode == CodexAccessMode::Router {
            let _ = router::set_smart_routing(&paths, previous.router.enabled && previously_owned);
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
        eprintln!("[AgentHarbor] local router for other clients is unavailable: {error}");
    }
    change.state.proxy_running = router_proxy::is_running();
    Ok(change)
}

#[tauri::command]
pub async fn set_newapi_access_mode(
    repo: State<'_, Mutex<Repository>>,
    mode: CodexAccessMode,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let previous = router::load_store(&paths)?;
    if previous.router.enabled
        && mode == CodexAccessMode::Router
        && router::active_routing_config_is_ours(&paths)
    {
        routing_runtime::ensure_available(&paths).await?;
    }
    let mut change = match router::set_access_mode(&paths, mode) {
        Ok(change) => change,
        Err(error) => {
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
    };
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        if change.state.router.enabled && mode == CodexAccessMode::Router {
            let _ = router::set_access_mode(&paths, previous.codex_access_mode);
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
        eprintln!("[AgentHarbor] local router for other clients is unavailable: {error}");
    }
    change.state.proxy_running = router_proxy::is_running();
    Ok(change)
}

#[tauri::command]
pub async fn set_newapi_accountless(
    repo: State<'_, Mutex<Repository>>,
    enabled: bool,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::set_accountless(&paths, enabled)
}

#[tauri::command]
pub async fn import_newapi_providers(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<ImportResult, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::import_providers(&paths, std::path::Path::new(&path))
}

#[tauri::command]
pub async fn export_newapi_providers(
    repo: State<'_, Mutex<Repository>>,
    path: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    super::blocking(move || router::export_providers(&paths, std::path::Path::new(&path))).await
}
