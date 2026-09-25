use crate::core::models::CoreEnvelope;
use crate::core::repository::Repository;
use crate::core::thread_recovery::{self, ThreadRecoveryPreview, ThreadRecoveryResult};
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub async fn preview_codex_thread_recovery(
    repo: State<'_, Mutex<Repository>>,
    thread_id: String,
) -> Result<CoreEnvelope<ThreadRecoveryPreview>, String> {
    let paths = repo
        .lock()
        .map_err(|error| error.to_string())?
        .paths()
        .clone();
    let preview = tauri::async_runtime::spawn_blocking(move || {
        thread_recovery::preview_thread_recovery(&paths, &thread_id)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(CoreEnvelope::ok(preview))
}

#[tauri::command]
pub async fn apply_codex_thread_recovery(
    repo: State<'_, Mutex<Repository>>,
    thread_id: String,
    expected_revision: String,
) -> Result<CoreEnvelope<ThreadRecoveryResult>, String> {
    let paths = repo
        .lock()
        .map_err(|error| error.to_string())?
        .paths()
        .clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        thread_recovery::apply_thread_recovery(&paths, &thread_id, &expected_revision)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(CoreEnvelope::ok(result))
}
