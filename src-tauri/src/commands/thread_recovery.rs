use crate::core::models::{CoreEnvelope, CoreWarning};
use crate::core::repository::Repository;
use crate::core::thread_recovery::{
    self, ThreadBatchCandidate, ThreadBatchPreview, ThreadBatchResult, ThreadBatchTarget,
    ThreadRecoveryPreview, ThreadRecoveryResult, ThreadRouteOverview,
};
use crate::platform::paths::CodexPaths;
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

#[tauri::command]
pub async fn adopt_codex_thread_router(
    repo: State<'_, Mutex<Repository>>,
    thread_id: String,
    expected_revision: String,
) -> Result<CoreEnvelope<ThreadRecoveryResult>, String> {
    let paths = repo
        .lock()
        .map_err(|error| error.to_string())?
        .paths()
        .clone();
    let (result, restart_warning) = tauri::async_runtime::spawn_blocking(move || {
        thread_recovery::adopt_thread_into_router(&paths, &thread_id, &expected_revision)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    let warnings = restart_warning
        .into_iter()
        .map(|message| CoreWarning {
            code: "CODEX_REOPEN_FAILED".into(),
            message,
        })
        .collect();
    Ok(CoreEnvelope::ok_with_warnings(result, warnings))
}

fn repo_paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|error| error.to_string())?
        .paths()
        .clone())
}

async fn blocking<T: Send + 'static>(
    run: impl FnOnce() -> Result<T, crate::core::models::CoreError> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(run)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// Route state per thread, for list badges. Read-only.
#[tauri::command]
pub async fn codex_thread_route_overview(
    repo: State<'_, Mutex<Repository>>,
) -> Result<ThreadRouteOverview, String> {
    let paths = repo_paths(&repo)?;
    blocking(move || thread_recovery::route_overview(&paths)).await
}

/// Preview moving `thread_ids` to the target, or every thread stored with
/// `from_provider` when no IDs are given. Read-only.
#[tauri::command]
pub async fn preview_codex_thread_batch(
    repo: State<'_, Mutex<Repository>>,
    thread_ids: Vec<String>,
    from_provider: Option<String>,
    target: ThreadBatchTarget,
) -> Result<ThreadBatchPreview, String> {
    let paths = repo_paths(&repo)?;
    blocking(move || {
        let thread_ids = match from_provider {
            Some(provider) if thread_ids.is_empty() => {
                thread_recovery::threads_with_provider(&paths, &provider)?
            }
            _ => thread_ids,
        };
        thread_recovery::preview_batch(&paths, &thread_ids, target)
    })
    .await
}

/// Apply a confirmed batch preview. Codex is stopped once and reopened if it
/// was running; each thread keeps its own backup and journal.
#[tauri::command]
pub async fn apply_codex_thread_batch(
    repo: State<'_, Mutex<Repository>>,
    candidates: Vec<ThreadBatchCandidate>,
    target: ThreadBatchTarget,
) -> Result<ThreadBatchResult, String> {
    let paths = repo_paths(&repo)?;
    blocking(move || thread_recovery::apply_batch(&paths, &candidates, target)).await
}
