use crate::core::auth::current_timestamp;
use crate::core::models::*;
use crate::core::repository::Repository;
use crate::core::threads::{self, ThreadExportFormat};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn list_codex_threads(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<ThreadListPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let paths = repo.paths();
    let items = threads::scan_threads(&paths.sessions_dir, &paths.archived_sessions_dir);
    Ok(CoreEnvelope::ok(ThreadListPayload {
        total: items.len() as i32,
        last_scan_at: current_timestamp(),
        items,
    }))
}

#[tauri::command]
pub fn export_codex_threads(
    repo: State<'_, Mutex<Repository>>,
    ids: Vec<String>,
    target_dir: String,
    format: String,
) -> Result<CoreEnvelope<ThreadExportPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let paths = repo.paths();
    let format = ThreadExportFormat::parse(&format).map_err(|e| e.to_string())?;
    let result = threads::export_threads(
        &paths.sessions_dir,
        &paths.archived_sessions_dir,
        &ids,
        &PathBuf::from(&target_dir),
        format,
    )
    .map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(ThreadExportPayload {
        exported: result.exported,
        directory: result.directory,
        files: result.files,
    }))
}
