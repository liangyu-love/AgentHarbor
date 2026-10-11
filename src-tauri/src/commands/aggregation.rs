use crate::{
    core::{
        aggregation, repository::Repository, router, routing_runtime,
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
/// Which member each aggregate sticks to and its members' cooldowns, polled by
/// the injection page while it is visible.
#[tauri::command]
pub async fn load_aggregation_live(
    repo: State<'_, Mutex<Repository>>,
) -> Result<Vec<aggregation::AggregateLive>, String> {
    let paths = paths(&repo)?;
    super::blocking(move || aggregation::live(&paths)).await
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
