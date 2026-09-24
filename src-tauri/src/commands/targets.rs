use crate::core::{repository::Repository, targets::TargetId};
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn restart_target(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<(), String> {
    let repo = repo.lock().map_err(|error| error.to_string())?;
    let controller = repo
        .target_registry()
        .process(target_id)
        .ok_or_else(|| "Unsupported target process controller".to_owned())?;
    controller.restart().map_err(|error| error.to_string())
}
