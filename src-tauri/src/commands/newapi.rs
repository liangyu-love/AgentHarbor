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
use serde::Serialize;
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

/// Effective metadata for selected and pending models in an unsaved editor.
#[tauri::command]
pub async fn preview_newapi_model_resolutions(
    repo: State<'_, Mutex<Repository>>,
    draft: crate::core::provider_model_preview::DraftProviderModels,
) -> Result<crate::core::provider_model_preview::DraftModelResolutions, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::provider_model_preview::preview(&paths, draft)
    })
    .await
    .map_err(|_| "newapi.errors.storage".to_owned())?
}

#[tauri::command]
pub async fn preview_newapi_capability_clear(
    repo: State<'_, Mutex<Repository>>,
    provider_id: String,
    model_id: String,
    field: router::ClearCapabilityField,
) -> Result<crate::core::capability_preview::ClearPreview, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = router::load_store(&paths)?;
        crate::core::capability_preview::preview_clear(
            &paths,
            &store,
            &provider_id,
            &model_id,
            field,
        )
    })
    .await
    .map_err(|_| "newapi.errors.storage".to_owned())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearApplyResult {
    pub applied: Option<StateChange>,
    pub refreshed_preview: Option<crate::core::capability_preview::ClearPreview>,
}

#[tauri::command]
pub async fn apply_newapi_capability_clear(
    repo: State<'_, Mutex<Repository>>,
    provider_id: String,
    model_id: String,
    field: router::ClearCapabilityField,
    config_revision: String,
    catalog_revision: String,
    official_revision: String,
) -> Result<ClearApplyResult, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    match router::clear_capability_if_current(
        &paths,
        &provider_id,
        &model_id,
        field,
        &config_revision,
        &catalog_revision,
        &official_revision,
    ) {
        Ok(applied) => Ok(ClearApplyResult {
            applied: Some(applied),
            refreshed_preview: None,
        }),
        Err(error) if error == "newapi.errors.previewStale" => {
            let store = router::load_store(&paths)?;
            let preview = crate::core::capability_preview::preview_clear(
                &paths,
                &store,
                &provider_id,
                &model_id,
                field,
            )?;
            Ok(ClearApplyResult {
                applied: None,
                refreshed_preview: Some(preview),
            })
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn set_router_web_search_provider(
    repo: State<'_, Mutex<Repository>>,
    provider_id: Option<String>,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::set_web_search_provider(&paths, provider_id)
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

/// Explicit per-model price override (USD per million tokens); `null` clears it
/// so the catalog price applies again.
#[tauri::command]
pub async fn set_newapi_model_price(
    repo: State<'_, Mutex<Repository>>,
    provider_id: String,
    model_id: String,
    price: Option<newapi::PriceOverrides>,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    router::set_model_price_override(&paths, &provider_id, &model_id, price)
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
pub async fn load_codex_subagents(
    repo: State<'_, Mutex<Repository>>,
) -> Result<router::subagents::View, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || router::subagents::load(&paths))
        .await
        .map_err(|_| "newapi.errors.storage".to_owned())?
}

#[tauri::command]
pub async fn save_codex_subagents(
    repo: State<'_, Mutex<Repository>>,
    settings: Option<router::subagents::Settings>,
) -> Result<StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    super::blocking(move || router::subagents::save(&paths, settings)).await
}

/// Reveal one saved provider API key only after an explicit user action in the
/// provider editor. The regular provider view deliberately keeps credentials
/// out of the frontend state; this command is the opt-in path behind the eye
/// button and never persists or logs the returned value.
#[tauri::command]
pub async fn reveal_newapi_provider_api_key(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<String, String> {
    let paths = paths(&repo)?;
    let store = router::load_store(&paths)?;
    store
        .providers
        .into_iter()
        .find(|provider| provider.id == id)
        .map(|provider| provider.api_key)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "newapi.errors.apiKeyMissing".to_owned())
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
pub async fn set_newapi_balance_refresh_interval(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    expected_base_url: String,
    interval_minutes: u32,
) -> Result<ProviderView, String> {
    let paths = paths(&repo)?;
    super::blocking(move || {
        router::set_balance_refresh_interval(&paths, &id, &expected_base_url, interval_minutes)
    })
    .await
}

pub fn spawn_balance_refresh(app: tauri::AppHandle, paths: CodexPaths) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let mut schedule = crate::core::balance_refresh::Schedule::default();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let query_paths = paths.clone();
            let update_handle = app.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                if let Ok(targets) = router::balance_refresh_targets(&query_paths) {
                    let now = chrono::Utc::now().timestamp();
                    let proxy = newapi::proxy_from_settings(&query_paths.settings_path);
                    for id in schedule.due(&targets, now) {
                        if matches!(router::query_scheduled_provider_balance(&query_paths, &id, &proxy), Ok(Some(_))) {
                            let _ = update_handle.emit("provider-balances-updated", ());
                        }
                    }
                }
                schedule
            })
            .await;
            let Ok(next_schedule) = result else {
                break;
            };
            schedule = next_schedule;
        }
    });
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
    force: Option<bool>,
) -> Result<router::CodexConnectionChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let previous = router::load_store(&paths)?;
    let previously_owned = router::active_routing_config_is_ours(&paths);
    if enabled && previous.codex_access_mode == CodexAccessMode::Router {
        routing_runtime::ensure_available(&paths).await?;
    }
    let result = if enabled {
        router::set_smart_routing(&paths, true).map(|change| router::CodexConnectionChange {
            change,
            thread_migration: None,
        })
    } else {
        let close_paths = paths.clone();
        super::blocking(move || {
            router::change_codex_connection(
                &close_paths,
                router::CodexConnectionTarget::Disabled,
                force.unwrap_or(false),
            )
        })
        .await
    };
    let mut change = match result {
        Ok(change) => change,
        Err(error) => {
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
    };
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        if enabled && change.change.state.router.access_mode == CodexAccessMode::Router {
            let _ = router::set_smart_routing(&paths, previous.router.enabled && previously_owned);
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
        eprintln!("[AgentHarbor] local router for other clients is unavailable: {error}");
    }
    change.change.state.proxy_running = router_proxy::is_running();
    Ok(change)
}

#[tauri::command]
pub async fn set_newapi_access_mode(
    repo: State<'_, Mutex<Repository>>,
    mode: CodexAccessMode,
    force: Option<bool>,
) -> Result<router::CodexConnectionChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let previous = router::load_store(&paths)?;
    if previous.router.enabled
        && mode == CodexAccessMode::Router
        && router::active_routing_config_is_ours(&paths)
    {
        routing_runtime::ensure_available(&paths).await?;
    }
    let result = if mode == CodexAccessMode::Native && previous.router.enabled {
        let close_paths = paths.clone();
        super::blocking(move || {
            router::change_codex_connection(
                &close_paths,
                router::CodexConnectionTarget::Native,
                force.unwrap_or(false),
            )
        })
        .await
    } else {
        router::set_access_mode(&paths, mode).map(|change| router::CodexConnectionChange {
            change,
            thread_migration: None,
        })
    };
    let mut change = match result {
        Ok(change) => change,
        Err(error) => {
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
    };
    if let Err(error) = routing_runtime::reconcile(&paths).await {
        if change.change.state.router.enabled && mode == CodexAccessMode::Router {
            let _ = router::set_access_mode(&paths, previous.codex_access_mode);
            let _ = routing_runtime::reconcile(&paths).await;
            return Err(error);
        }
        eprintln!("[AgentHarbor] local router for other clients is unavailable: {error}");
    }
    change.change.state.proxy_running = router_proxy::is_running();
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
