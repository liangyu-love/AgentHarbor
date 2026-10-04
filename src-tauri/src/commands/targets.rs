use crate::core::{
    repository::Repository,
    targets::{TargetId, TargetRegistry},
};
use std::sync::Mutex;
use tauri::State;

/// Restarting waits for the process to exit, so run it off the main thread and
/// without holding the repository lock that every other command needs.
#[tauri::command]
pub async fn restart_target(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<(), String> {
    let paths = repo
        .lock()
        .map_err(|error| error.to_string())?
        .paths()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let registry = TargetRegistry::new(paths);
        let controller = registry
            .process(target_id)
            .ok_or_else(|| "Unsupported target process controller".to_owned())?;
        controller.restart().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
