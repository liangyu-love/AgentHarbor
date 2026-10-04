use crate::core::account_monitor;
use crate::core::account_login::{self, LoginManager};
use crate::core::accounts::{self, CodexAccountListPayload};
use crate::core::repository::Repository;
use std::sync::Mutex;
use tauri::State;

fn lock_repo<'a>(
    repo: &'a State<'_, Mutex<Repository>>,
) -> Result<std::sync::MutexGuard<'a, Repository>, String> {
    repo.lock()
        .map_err(|_| "Repository lock poisoned".to_string())
}

#[tauri::command]
pub fn list_codex_accounts(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CodexAccountListPayload, String> {
    let repo = lock_repo(&repo)?;
    accounts::list(&repo).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn refresh_codex_accounts(
    repo: State<'_, Mutex<Repository>>,
) -> Result<CodexAccountListPayload, String> {
    refresh_all(&repo).await
}

/// Shared by the accounts page and the auto-switch loop.
pub(crate) async fn refresh_all(
    repo: &Mutex<Repository>,
) -> Result<CodexAccountListPayload, String> {
    let (credentials, proxy) = {
        let repo = repo
            .lock()
            .map_err(|_| "Repository lock poisoned".to_string())?;
        accounts::credentials(&repo).map_err(|error| error.to_string())?
    };
    // Each account may start its own app-server with multi-second timeouts, so
    // refresh a few at a time instead of one after another.
    use futures_util::StreamExt;
    let updates: Vec<_> = futures_util::stream::iter(credentials)
        .map(|credential| account_monitor::refresh_account(credential, proxy.clone()))
        .buffered(3)
        .collect()
        .await;
    let repo = repo
        .lock()
        .map_err(|_| "Repository lock poisoned".to_string())?;
    accounts::apply_usage_updates(&repo, updates).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn login_codex_account(
    repo: State<'_, Mutex<Repository>>,
    manager: State<'_, LoginManager>,
    attempt_id: String,
) -> Result<CodexAccountListPayload, String> {
    let attempt = manager.begin(attempt_id)?;
    let (paths, proxy) = {
        let repo = lock_repo(&repo)?;
        (
            repo.paths().clone(),
            crate::core::newapi::proxy_from_settings(&repo.paths().settings_path),
        )
    };
    let artifact = account_login::login(&paths, &proxy, attempt.cancellation()).await?;
    attempt.commit(|| {
        let repo = lock_repo(&repo)?;
        accounts::register_login(&repo, &artifact.auth_path).map_err(|error| error.to_string())
    })
}

#[tauri::command]
pub async fn cancel_codex_account_login(manager: State<'_, LoginManager>, attempt_id: String) -> Result<bool, String> {
    manager.cancel(attempt_id).await
}

#[tauri::command]
pub fn switch_codex_account(
    repo: State<'_, Mutex<Repository>>,
    account_key: String,
) -> Result<CodexAccountListPayload, String> {
    let repo = lock_repo(&repo)?;
    accounts::switch(&repo, &account_key).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_codex_account(
    repo: State<'_, Mutex<Repository>>,
    account_key: String,
) -> Result<CodexAccountListPayload, String> {
    let repo = lock_repo(&repo)?;
    accounts::delete(&repo, &account_key).map_err(|error| error.to_string())
}
