//! Accounts of the official account sources (WorkBuddy, WorkBuddy AI,
//! ChatGPT through Sign in with ChatGPT) and their share with model inject.
use crate::core::account_login::{self, LoginManager};
use crate::core::account_sources::{self, ListPayload, SourceKind};
use crate::core::repository::Repository;
use crate::core::{newapi, routing_runtime};
use crate::platform::paths::CodexPaths;
use std::sync::Mutex;
use tauri::State;

/// One cancellable account-source sign-in at a time, apart from Codex's.
#[derive(Default, Clone)]
pub struct SourceLoginManager(pub LoginManager);

fn context(repo: &State<'_, Mutex<Repository>>) -> Result<(CodexPaths, reqwest::Client), String> {
    let paths = repo.lock().map_err(|_| "Repository lock poisoned".to_owned())?.paths().clone();
    let builder = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10));
    let client = crate::core::api_client::configure_async_client(builder, &newapi::proxy_from_settings(&paths.settings_path))
        .map_err(|_| "newapi.errors.proxy".to_owned())?
        .build()
        .map_err(|_| "newapi.errors.network".to_owned())?;
    Ok((paths, client))
}

#[tauri::command]
pub async fn list_account_sources(repo: State<'_, Mutex<Repository>>) -> Result<ListPayload, String> {
    let (paths, _) = context(&repo)?;
    super::blocking(move || account_sources::list(&paths)).await
}

#[tauri::command]
pub async fn sign_in_account_source(
    repo: State<'_, Mutex<Repository>>,
    manager: State<'_, SourceLoginManager>,
    kind: SourceKind,
    attempt_id: String,
) -> Result<ListPayload, String> {
    let attempt = manager.0.begin(attempt_id)?;
    let (paths, client) = context(&repo)?;
    let account = account_sources::sign_in(&paths, &client, kind, attempt.cancellation(), account_login::open_browser).await?;
    let first = attempt.commit(|| account_sources::keep(&paths, account))?;
    drop(attempt);
    {
        let _guard = routing_runtime::mutation_guard().await;
        // The account is kept; a model list or quota read that fails now is
        // retried from the Accounts page.
        if let Err(error) = account_sources::after_sign_in(&paths, &client, kind, first).await {
            log::warn!("account source follow-up after sign-in failed: {error}");
        }
    }
    account_sources::list(&paths)
}

#[tauri::command]
pub async fn cancel_account_source_sign_in(manager: State<'_, SourceLoginManager>, attempt_id: String) -> Result<bool, String> {
    manager.0.cancel(attempt_id).await
}

#[tauri::command]
pub async fn activate_account_source(repo: State<'_, Mutex<Repository>>, account_id: String) -> Result<ListPayload, String> {
    let (paths, _) = context(&repo)?;
    super::blocking(move || {
        account_sources::activate(&paths, &account_id)?;
        account_sources::list(&paths)
    }).await
}

#[tauri::command]
pub async fn remove_account_source(repo: State<'_, Mutex<Repository>>, account_id: String) -> Result<ListPayload, String> {
    let (paths, client) = context(&repo)?;
    account_sources::remove(&paths, &client, &account_id).await?;
    account_sources::list(&paths)
}

#[tauri::command]
pub async fn refresh_account_source_quota(repo: State<'_, Mutex<Repository>>, account_id: String) -> Result<ListPayload, String> {
    let (paths, client) = context(&repo)?;
    account_sources::refresh_quota(&paths, &client, &account_id).await?;
    account_sources::list(&paths)
}

#[tauri::command]
pub async fn refresh_account_source_models(repo: State<'_, Mutex<Repository>>, kind: SourceKind) -> Result<ListPayload, String> {
    let (paths, client) = context(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    account_sources::refresh_models(&paths, &client, kind).await?;
    account_sources::list(&paths)
}

#[tauri::command]
pub async fn set_account_source_exposed(repo: State<'_, Mutex<Repository>>, kind: SourceKind, exposed: bool) -> Result<ListPayload, String> {
    let (paths, _) = context(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    super::blocking(move || {
        account_sources::set_exposed(&paths, kind, exposed)?;
        account_sources::list(&paths)
    }).await
}
