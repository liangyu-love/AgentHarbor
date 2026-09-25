use crate::core::auth::current_timestamp;
use crate::core::models::*;
use crate::core::repository::Repository;
use crate::core::targets::TargetId;
use crate::core::thread_delete::{self, ThreadDeleteResult};
use crate::core::threads::{self, ThreadExportFormat};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub async fn list_codex_threads(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CoreEnvelope<ThreadListPayload>, String> {
    list_conversations(repo, TargetId::Codex).await
}

#[tauri::command]
pub async fn list_conversations(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<CoreEnvelope<ThreadListPayload>, String> {
    let (sessions_dir, archived_sessions_dir, state_database_path) = {
        let repo = repo.lock().map_err(|e| e.to_string())?;
        let source = repo
            .target_registry()
            .conversations(target_id)
            .ok_or_else(|| "Unsupported conversation target".to_owned())?;
        (
            source.sessions_dir().to_path_buf(),
            source.archived_sessions_dir().to_path_buf(),
            source.state_database_path().to_path_buf(),
        )
    };
    let scan = tauri::async_runtime::spawn_blocking(move || {
        threads::scan_threads_from_catalog_with_warnings(
            &state_database_path,
            &sessions_dir,
            &archived_sessions_dir,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(CoreEnvelope::ok_with_warnings(
        ThreadListPayload {
            total: scan.items.len(),
            last_scan_at: current_timestamp(),
            items: scan.items,
        },
        scan.warnings,
    ))
}

#[tauri::command]
pub async fn export_codex_threads(
    repo: State<'_, Mutex<Repository>>,
    keys: Vec<String>,
    target_dir: String,
    format: String,
) -> Result<CoreEnvelope<ThreadExportPayload>, String> {
    export_conversations(repo, TargetId::Codex, keys, target_dir, format).await
}

#[tauri::command]
pub async fn export_conversations(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
    keys: Vec<String>,
    target_dir: String,
    format: String,
) -> Result<CoreEnvelope<ThreadExportPayload>, String> {
    let format = ThreadExportFormat::parse(&format).map_err(|e| e.to_string())?;
    let (sessions_dir, archived_sessions_dir) = {
        let repo = repo.lock().map_err(|e| e.to_string())?;
        let source = repo
            .target_registry()
            .conversations(target_id)
            .ok_or_else(|| "Unsupported conversation target".to_owned())?;
        (
            source.sessions_dir().to_path_buf(),
            source.archived_sessions_dir().to_path_buf(),
        )
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        threads::export_threads(
            &sessions_dir,
            &archived_sessions_dir,
            &keys,
            &PathBuf::from(&target_dir),
            format,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(ThreadExportPayload {
        exported: result.exported,
        directory: result.directory,
        files: result.files,
    }))
}

#[tauri::command]
pub async fn delete_conversations(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
    keys: Vec<String>,
) -> Result<CoreEnvelope<ThreadDeleteResult>, String> {
    let (sessions_dir, archived_sessions_dir, state_database_path) = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        let source = repo
            .target_registry()
            .conversations(target_id)
            .ok_or_else(|| "Unsupported conversation target".to_owned())?;
        (
            source.sessions_dir().to_path_buf(),
            source.archived_sessions_dir().to_path_buf(),
            source.state_database_path().to_path_buf(),
        )
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        thread_delete::delete_threads(
            &state_database_path,
            &sessions_dir,
            &archived_sessions_dir,
            &keys,
        )
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(CoreEnvelope::ok(result))
}
