//! The general gateway: its address, named keys and the models it serves.
use crate::core::{aggregation, gateway, model_targets_export, models_dev, protocol::ModelProtocol, repository::Repository, router, routing_runtime};
use crate::platform::paths::CodexPaths;
use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo.lock().map_err(|_| "gateway.errors.storage")?.paths().clone())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayState {
    pub port: u16,
    /// The local listener is up right now.
    pub running: bool,
    /// The listener stays up for agents even when no client is injected.
    pub keep_running: bool,
    pub keys: Vec<gateway::KeyView>,
    pub models: Vec<gateway::GatewayModel>,
}

fn state(paths: &CodexPaths) -> Result<GatewayState, String> {
    let store = router::load_store(paths)?;
    let catalog = models_dev::load(&paths.app_data_dir);
    Ok(GatewayState {
        port: store.router.port,
        running: crate::core::router_proxy::is_running(),
        keep_running: store.router.aggregate_external_enabled,
        keys: gateway::key_views(&store),
        models: gateway::models(&store, &catalog),
    })
}

#[tauri::command]
pub async fn load_gateway_state(repo: State<'_, Mutex<Repository>>) -> Result<GatewayState, String> {
    let paths = paths(&repo)?;
    super::blocking(move || state(&paths)).await
}

#[tauri::command]
pub async fn create_gateway_key(repo: State<'_, Mutex<Repository>>, name: String) -> Result<GatewayState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    super::blocking(move || {
        gateway::create_key(&paths, &name)?;
        state(&paths)
    }).await
}

#[tauri::command]
pub async fn rename_gateway_key(repo: State<'_, Mutex<Repository>>, id: String, name: String) -> Result<GatewayState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    super::blocking(move || {
        gateway::rename_key(&paths, &id, &name)?;
        state(&paths)
    }).await
}

#[tauri::command]
pub async fn regenerate_gateway_key(repo: State<'_, Mutex<Repository>>, id: String) -> Result<GatewayState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    super::blocking(move || {
        gateway::regenerate_key(&paths, &id)?;
        state(&paths)
    }).await
}

#[tauri::command]
pub async fn delete_gateway_key(repo: State<'_, Mutex<Repository>>, id: String) -> Result<GatewayState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    super::blocking(move || {
        gateway::delete_key(&paths, &id)?;
        state(&paths)
    }).await
}

/// One key's secret, only after an explicit reveal or copy.
#[tauri::command]
pub async fn reveal_gateway_key(repo: State<'_, Mutex<Repository>>, id: String) -> Result<String, String> {
    let paths = paths(&repo)?;
    super::blocking(move || gateway::reveal_key(&router::load_store(&paths)?, &id)).await
}

/// Connection settings for one agent, with the chosen key shown only when `reveal`.
#[tauri::command]
pub async fn export_gateway_connection(
    repo: State<'_, Mutex<Repository>>,
    ingress_protocol: ModelProtocol,
    selected_model_id: Option<String>,
    consumer_profile: String,
    key_id: String,
    reveal_credential: bool,
) -> Result<model_targets_export::ExportBundle, String> {
    let paths = paths(&repo)?;
    super::blocking(move || {
        let store = router::load_store(&paths)?;
        let catalog = models_dev::load(&paths.app_data_dir);
        model_targets_export::export(&store, &catalog, ingress_protocol, selected_model_id.as_deref(),
            &consumer_profile, &key_id, reveal_credential)
    }).await
}

#[tauri::command]
pub async fn set_gateway_keep_running(repo: State<'_, Mutex<Repository>>, enabled: bool) -> Result<GatewayState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    if enabled {
        routing_runtime::ensure_available(&paths).await?;
    }
    if let Err(error) = aggregation::set_external_enabled(&paths, enabled) {
        let _ = routing_runtime::reconcile(&paths).await;
        return Err(error);
    }
    routing_runtime::reconcile(&paths).await?;
    state(&paths)
}
