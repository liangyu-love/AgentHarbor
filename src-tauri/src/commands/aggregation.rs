use crate::{
    core::{
        aggregation, model_targets_export, models_dev, protocol::ModelProtocol,
        repository::Repository, router, routing_runtime,
    },
    platform::paths::CodexPaths,
};
use std::sync::Mutex;
use tauri::State;
fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "aggregation.errors.storage")?
        .paths()
        .clone())
}

#[tauri::command]
pub async fn load_aggregation_state(
    repo: State<'_, Mutex<Repository>>,
) -> Result<aggregation::AggregationState, String> {
    let paths = paths(&repo)?;
    super::blocking(move || aggregation::state(&paths)).await
}
#[tauri::command]
pub async fn save_aggregate_model(
    repo: State<'_, Mutex<Repository>>,
    model: aggregation::AggregateModel,
) -> Result<aggregation::AggregationState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    aggregation::save(&paths, model)?;
    let store = router::load_store(&paths)?;
    if store.router.aggregate_model_id.is_some() {
        router::write_catalog(&paths, &store)?;
    }
    routing_runtime::reconcile(&paths).await?;
    aggregation::state(&paths)
}
#[tauri::command]
pub async fn delete_aggregate_model(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<aggregation::AggregationState, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    aggregation::delete(&paths, &id)?;
    aggregation::affinity::shared().forget_aggregate(&id);
    aggregation::runtime::shared().forget_scope(&id);
    routing_runtime::reconcile(&paths).await?;
    aggregation::state(&paths)
}
#[tauri::command]
pub async fn set_aggregation_external_enabled(
    repo: State<'_, Mutex<Repository>>,
    enabled: bool,
) -> Result<aggregation::AggregationState, String> {
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
    aggregation::state(&paths)
}
#[tauri::command]
pub async fn select_codex_aggregate(
    repo: State<'_, Mutex<Repository>>,
    aggregate_id: Option<String>,
) -> Result<router::StateChange, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let result = router::select_aggregate(&paths, aggregate_id)?;
    routing_runtime::reconcile(&paths).await?;
    Ok(result)
}
#[tauri::command]
pub async fn export_aggregation_connection(
    repo: State<'_, Mutex<Repository>>,
    ingress_protocol: ModelProtocol,
    aggregate_ids: Vec<String>,
    selected_aggregate_id: Option<String>,
    consumer_profile: String,
    reveal_credential: bool,
) -> Result<model_targets_export::ExportBundle, String> {
    let paths = paths(&repo)?;
    super::blocking(move || {
        let store = router::load_store(&paths)?;
        let catalog = models_dev::load(&paths.app_data_dir);
        model_targets_export::export(
            &store,
            &catalog,
            ingress_protocol,
            &aggregate_ids,
            selected_aggregate_id.as_deref(),
            &consumer_profile,
            reveal_credential,
        )
    })
    .await
}
