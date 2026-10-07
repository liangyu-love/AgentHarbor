use crate::core::auth::current_timestamp;
use crate::core::claude_sessions;
use crate::core::models::*;
use crate::core::repository::Repository;
use crate::core::targets::TargetId;
use crate::core::thread_delete::{self, ThreadDeleteResult};
use crate::core::threads::{self, ThreadExportFormat};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

fn conversation_roots(
    repo: &Mutex<Repository>,
    target_id: TargetId,
) -> Result<(PathBuf, PathBuf), String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let source = repo
        .target_registry()
        .conversations(target_id)
        .ok_or_else(|| "Unsupported conversation target".to_owned())?;
    Ok((
        source.sessions_dir().to_path_buf(),
        source.archived_sessions_dir().to_path_buf(),
    ))
}

#[tauri::command]
pub async fn get_conversation_details(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
    key: String,
    thread_id: String,
) -> Result<CoreEnvelope<threads::details::ThreadDetails>, String> {
    if target_id == TargetId::ClaudeCode {
        let root = claude_sessions::projects_root();
        let detail = tauri::async_runtime::spawn_blocking(move || {
            claude_sessions::read_details(&root, &key, &thread_id)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        return Ok(CoreEnvelope::ok(detail));
    }
    let (sessions, archived) = conversation_roots(&repo, target_id)?;
    let detail = tauri::async_runtime::spawn_blocking(move || {
        threads::details::read_details(&sessions, &archived, &key, &thread_id)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(detail))
}

#[tauri::command]
pub async fn get_conversation_messages(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
    key: String,
    thread_id: String,
    snapshot_bytes: u64,
    offset: usize,
    snapshot_id: Option<String>,
) -> Result<CoreEnvelope<threads::details::MessagePage>, String> {
    if target_id == TargetId::ClaudeCode {
        let root = claude_sessions::projects_root();
        let page = tauri::async_runtime::spawn_blocking(move || {
            claude_sessions::read_messages_versioned(
                &root,
                &key,
                &thread_id,
                snapshot_bytes,
                offset,
                snapshot_id.as_deref(),
            )
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        return Ok(CoreEnvelope::ok(page));
    }
    let (sessions, archived) = conversation_roots(&repo, target_id)?;
    let page = tauri::async_runtime::spawn_blocking(move || {
        threads::details::read_messages_versioned(
            &sessions,
            &archived,
            &key,
            &thread_id,
            snapshot_bytes,
            offset,
            snapshot_id.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(page))
}

#[tauri::command]
pub async fn list_conversations(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<CoreEnvelope<ThreadListPayload>, String> {
    if target_id == TargetId::ClaudeCode {
        let root = claude_sessions::projects_root();
        let scan =
            tauri::async_runtime::spawn_blocking(move || claude_sessions::scan_threads(&root))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
        return Ok(CoreEnvelope::ok_with_warnings(
            ThreadListPayload {
                total: scan.items.len(),
                last_scan_at: current_timestamp(),
                items: scan.items,
                pending_deletion_keys: vec![],
                pending_deletion_recovery_needed: false,
            },
            scan.warnings,
        ));
    }
    let (sessions_dir, archived_sessions_dir, state_database_path, thread_cache_path) = {
        let repo = repo.lock().map_err(|e| e.to_string())?;
        let source = repo
            .target_registry()
            .conversations(target_id)
            .ok_or_else(|| "Unsupported conversation target".to_owned())?;
        (
            source.sessions_dir().to_path_buf(),
            source.archived_sessions_dir().to_path_buf(),
            source.state_database_path().to_path_buf(),
            repo.paths()
                .app_data_dir
                .join("thread-summary-cache.sqlite3"),
        )
    };
    let (scan, pending) = tauri::async_runtime::spawn_blocking(move || {
        let pending = thread_delete::process_pending_deletes(
            &state_database_path,
            &sessions_dir,
            &archived_sessions_dir,
        );
        let scan = threads::scan_threads_from_catalog_with_cache(
            &state_database_path,
            &sessions_dir,
            &archived_sessions_dir,
            &thread_cache_path,
        )?;
        Ok::<_, CoreError>((scan, pending))
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    let mut warnings = scan.warnings;
    warnings.extend(pending.warnings.into_iter().map(|message| CoreWarning {
        code: "THREAD_DELETE_QUEUE".into(),
        message,
    }));
    Ok(CoreEnvelope::ok_with_warnings(
        ThreadListPayload {
            total: scan.items.len(),
            last_scan_at: current_timestamp(),
            items: scan.items,
            pending_deletion_keys: pending.keys,
            pending_deletion_recovery_needed: pending.recovery_required,
        },
        warnings,
    ))
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
    if target_id == TargetId::ClaudeCode {
        let root = claude_sessions::projects_root();
        let result = tauri::async_runtime::spawn_blocking(move || {
            claude_sessions::export_threads(&root, &keys, &PathBuf::from(target_dir), format)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        return Ok(CoreEnvelope::ok(ThreadExportPayload {
            exported: result.exported,
            directory: result.directory,
            files: result.files,
        }));
    }
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
    if target_id != TargetId::Codex {
        return Err("This conversation source is read-only; deletion is unsupported".into());
    }
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

#[tauri::command]
pub async fn cancel_pending_conversation_deletions(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<CoreEnvelope<usize>, String> {
    if target_id != TargetId::Codex {
        return Err("This conversation source has no pending deletion queue".into());
    }
    let state_database_path = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        repo.target_registry()
            .conversations(target_id)
            .ok_or_else(|| "Unsupported conversation target".to_owned())?
            .state_database_path()
            .to_path_buf()
    };
    let cancelled = tauri::async_runtime::spawn_blocking(move || {
        thread_delete::cancel_pending_deletes(&state_database_path)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    Ok(CoreEnvelope::ok(cancelled))
}
