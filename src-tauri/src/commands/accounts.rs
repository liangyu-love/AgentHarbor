use crate::core::account_monitor;
use crate::core::account_login::{self, LoginManager};
use crate::core::accounts::{self, CodexAccountListPayload};
use crate::core::repository::Repository;
use std::sync::Mutex;
use std::time::Duration;
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
    let payload = {
        let repo = repo
            .lock()
            .map_err(|_| "Repository lock poisoned".to_string())?;
        accounts::apply_usage_updates(&repo, updates).map_err(|error| error.to_string())?
    };
    refresh_model_catalogs(repo, None, CATALOG_REFRESH_INTERVAL).await;
    Ok(payload)
}

/// Official catalogs change rarely; quota refreshes renew them at most this often.
pub(crate) const CATALOG_REFRESH_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// Bound account switching and login when the catalog service is slow.
const CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Fetch official model catalogs with each account's own credentials. `only`
/// limits this to one account; `max_age` skips catalogs fetched recently.
/// Failures keep that account's last good catalog and never fail the caller.
pub(crate) async fn refresh_model_catalogs(
    repo: &Mutex<Repository>,
    only: Option<&str>,
    max_age: Duration,
) {
    let due = {
        let Ok(repo) = repo.lock() else { return };
        let Ok((credentials, proxy)) = accounts::credentials(&repo) else {
            return;
        };
        let app_data = repo.paths().app_data_dir.clone();
        let credentials: Vec<_> = credentials
            .into_iter()
            .filter(|credential| only.is_none_or(|key| key == credential.account_key))
            .filter(|credential| {
                crate::core::model_catalog::account_catalog_due(
                    &app_data,
                    &credential.account_key,
                    max_age,
                )
            })
            .collect();
        (credentials, proxy)
    };
    let (credentials, proxy) = due;
    use futures_util::StreamExt;
    let fetched: Vec<_> = futures_util::stream::iter(credentials)
        .map(|credential| {
            let proxy = proxy.clone();
            async move {
                let (catalog, refresh) = tokio::time::timeout(
                    CATALOG_FETCH_TIMEOUT,
                    account_monitor::fetch_model_catalog(&credential, &proxy),
                )
                .await
                .unwrap_or_else(|_| (Err("model catalog fetch timed out".into()), None));
                (credential.account_key, catalog, refresh)
            }
        })
        .buffered(2)
        .collect()
        .await;
    let Ok(repo) = repo.lock() else { return };
    for (account_key, catalog, refresh) in fetched {
        accounts::apply_catalog_fetch(&repo, &account_key, catalog, refresh);
    }
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
    let payload = attempt.commit(|| {
        let repo = lock_repo(&repo)?;
        accounts::register_login(&repo, &artifact.auth_path).map_err(|error| error.to_string())
    })?;
    // Give a new login its own official catalog before it is first selected.
    refresh_model_catalogs(&repo, None, CATALOG_REFRESH_INTERVAL).await;
    Ok(payload)
}

#[tauri::command]
pub async fn cancel_codex_account_login(manager: State<'_, LoginManager>, attempt_id: String) -> Result<bool, String> {
    manager.cancel(attempt_id).await
}

#[tauri::command]
pub async fn switch_codex_account(
    repo: State<'_, Mutex<Repository>>,
    account_key: String,
) -> Result<CodexAccountListPayload, String> {
    // Codex can flush its in-memory account and model catalog as it exits.
    // Stop that writer before replacing auth.json; the UI restarts it after
    // this command returns, using the newly selected account.
    let paths = { lock_repo(&repo)?.paths().clone() };
    let was_running = crate::platform::process::is_codex_app_running();
    if was_running {
        #[cfg(target_os = "windows")]
        crate::platform::process::snapshot_codex_path_before_stop();
        tauri::async_runtime::spawn_blocking(|| {
            crate::platform::process::stop_codex_app_gracefully(std::time::Duration::from_secs(8))
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    }
    let switched = (|| -> Result<CodexAccountListPayload, String> {
        let repo = lock_repo(&repo)?;
        accounts::switch(&repo, &account_key).map_err(|error| error.to_string())
    })();
    let result = match switched {
        Ok(result) => result,
        Err(error) => {
            if was_running {
                let reopened = tauri::async_runtime::spawn_blocking(move || {
                    let registry = crate::core::targets::TargetRegistry::new(paths);
                    registry
                        .process(crate::core::targets::TargetId::Codex)
                        .ok_or_else(|| "Codex process controller unavailable".to_owned())?
                        .restart()
                        .map_err(|error| error.to_string())
                })
                .await;
                if let Err(launch) = reopened.unwrap_or_else(|error| Err(error.to_string())) {
                    log::warn!("Codex reopen after failed account switch failed: {launch}");
                }
            }
            return Err(error);
        }
    };
    // Codex is stopped and reads the injected catalog only when it starts, so
    // fetch the selected account's own models before the UI restarts it.
    if let Some(key) = result.active_account_key.as_deref() {
        refresh_model_catalogs(&repo, Some(key), Duration::ZERO).await;
    }
    Ok(result)
}

#[tauri::command]
pub fn delete_codex_account(
    repo: State<'_, Mutex<Repository>>,
    account_key: String,
) -> Result<CodexAccountListPayload, String> {
    let repo = lock_repo(&repo)?;
    accounts::delete(&repo, &account_key).map_err(|error| error.to_string())
}
