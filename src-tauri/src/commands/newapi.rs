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

/// The models.dev catalog status without network access.
#[tauri::command]
pub async fn load_models_dev_status(
    repo: State<'_, Mutex<Repository>>,
) -> Result<crate::core::models_dev::CatalogStatus, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = crate::core::models_dev::load(&paths.app_data_dir);
        let now = chrono::Utc::now().timestamp();
        Ok(catalog.status(now))
    })
    .await
    .map_err(|_| "newapi.errors.storage".to_owned())?
}

/// Manual catalog refresh. Downloads, validates and atomically replaces the
/// runtime cache; any failure keeps the previously valid catalog active.
#[tauri::command]
pub async fn refresh_models_dev_catalog(
    repo: State<'_, Mutex<Repository>>,
) -> Result<crate::core::models_dev::CatalogStatus, String> {
    let (paths, proxy) = {
        let repo = repo.lock().map_err(|_| "newapi.errors.storage")?;
        (repo.paths().clone(), repo.api_proxy_config())
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::models_dev::refresh(&paths.app_data_dir, &proxy)?;
        let catalog = crate::core::models_dev::load(&paths.app_data_dir);
        Ok(catalog.status(chrono::Utc::now().timestamp()))
    })
    .await
    .map_err(|_| "newapi.errors.storage".to_owned())?
}

#[tauri::command]
pub fn load_route_diagnostics() -> Vec<crate::core::route_trace::Route> {
    crate::core::route_trace::recent()
}

#[tauri::command]
pub async fn load_model_capabilities(
    repo: State<'_, Mutex<Repository>>,
) -> Result<crate::core::capability_preview::CatalogView, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = router::load_store(&paths)?;
        Ok(crate::core::capability_preview::preview(&paths, &store))
    })
    .await
    .map_err(|_| "newapi.errors.storage".to_owned())?
}

#[tauri::command]
pub async fn set_newapi_failover(
    repo: State<'_, Mutex<Repository>>,
    enabled: bool,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::set_same_model_failover(&paths, enabled)
}

/// Global compression policy (default 90%). Existing injections are re-derived.
#[tauri::command]
pub async fn set_newapi_compression(
    repo: State<'_, Mutex<Repository>>,
    policy: crate::core::model_policy::CompressionPolicy,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::set_compression(&paths, policy)
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
    on_event: tauri::ipc::Channel<crate::core::provider_probe::ProbeEvent>,
) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    let proxy = newapi::proxy_from_settings(&paths.settings_path);
    tauri::async_runtime::spawn_blocking(move || {
        let emit = |event| {
            let _ = on_event.send(event);
        };
        router::test_provider_with_progress(&paths, &id, &model, &proxy, Some(&emit))
    })
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
pub async fn set_newapi_balance_credentials(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    expected_base_url: String,
    user_id: Option<u64>,
    access_token: String,
) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    super::blocking(move || {
        router::set_balance_credentials(&paths, &id, &expected_base_url, user_id, &access_token)
    })
    .await
}

#[tauri::command]
pub async fn set_newapi_balance_script(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    expected_base_url: String,
    script: String,
) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    super::blocking(move || router::set_balance_script(&paths, &id, &expected_base_url, &script))
        .await
}

#[tauri::command]
pub async fn set_newapi_balance_sharing(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    expected_base_url: String,
    shared: bool,
) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    super::blocking(move || router::set_balance_sharing(&paths, &id, &expected_base_url, shared))
        .await
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
