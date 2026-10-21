//! Remote hosts and the Codex and Claude Code configuration on them. Each
//! command opens its own SSH session on the blocking pool and closes it.

use super::blocking;
use crate::core::{
    client_config::{ClientStatus, ConfigApplied, ConfigManager, ConfigPreview, ConfigRequest},
    model_catalog,
    remote::{
        codex::{
            CodexRemote, CodexRemoteApplied, CodexRemotePreview, CodexRemoteRequest,
            CodexRemoteStatus,
        },
        ConnectionTest, RemoteHostInput, RemoteHostView, RemoteSession, RemoteStore,
    },
    repository::Repository,
    router,
    targets::TargetId,
};
use crate::platform::paths::CodexPaths;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::State;

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo
        .lock()
        .map_err(|_| "remote.errors.storage")?
        .paths()
        .clone())
}

fn connect(store: &RemoteStore, id: &str) -> Result<Arc<RemoteSession>, String> {
    store
        .connect(id, None)
        .map(Arc::new)
        .map_err(|error| error.key())
}

fn official(paths: &CodexPaths) -> Vec<serde_json::Value> {
    model_catalog::official(&paths.codex_home, &paths.app_data_dir).models
}

fn claude(
    paths: &CodexPaths,
    store: &RemoteStore,
    id: &str,
    session: Arc<RemoteSession>,
) -> Result<ConfigManager, String> {
    let home = session.home().to_owned();
    Ok(
        ConfigManager::remote(&paths.app_data_dir, &store.journal_dir(id)?, &home, session)
            .with_official_home(&paths.codex_home),
    )
}

#[tauri::command]
pub async fn list_remote_hosts(
    repo: State<'_, Mutex<Repository>>,
) -> Result<Vec<RemoteHostView>, String> {
    let paths = paths(&repo)?;
    blocking(move || RemoteStore::new(&paths.app_data_dir).list()).await
}

/// The concrete host aliases of `~/.ssh/config`.
#[tauri::command]
pub async fn ssh_config_aliases() -> Result<Vec<String>, String> {
    blocking(|| Ok(crate::core::remote::ssh_aliases())).await
}

#[tauri::command]
pub async fn save_remote_host(
    repo: State<'_, Mutex<Repository>>,
    input: RemoteHostInput,
) -> Result<RemoteHostView, String> {
    let paths = paths(&repo)?;
    blocking(move || RemoteStore::new(&paths.app_data_dir).save_host(input)).await
}

#[tauri::command]
pub async fn delete_remote_host(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    blocking(move || RemoteStore::new(&paths.app_data_dir).delete_host(&id)).await
}

/// Connects to the host. `trust` is the host key fingerprint the user
/// confirmed after a first attempt reported it.
#[tauri::command]
pub async fn test_remote_host(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    trust: Option<String>,
) -> Result<ConnectionTest, String> {
    let paths = paths(&repo)?;
    blocking(move || Ok(RemoteStore::new(&paths.app_data_dir).test(&id, trust))).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteClients {
    pub home: String,
    pub codex: CodexRemoteStatus,
    pub claude_code: ClientStatus,
}

#[tauri::command]
pub async fn remote_client_status(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<RemoteClients, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let session = connect(&store, &id)?;
        let relays = router::load_store(&paths).ok();
        let codex = CodexRemote::new(
            session.as_ref(),
            session.home(),
            &store.journal_dir(&id)?,
            &paths.app_data_dir,
            official(&paths),
        )
        .status(relays.as_ref());
        let claude_code = claude(&paths, &store, &id, session.clone())?
            .status(TargetId::ClaudeCode, relays.as_ref());
        Ok(RemoteClients {
            home: session.home().to_owned(),
            codex,
            claude_code,
        })
    })
    .await
}

#[tauri::command]
pub async fn preview_remote_codex(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    request: CodexRemoteRequest,
    replace_conflicts: Option<bool>,
) -> Result<CodexRemotePreview, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let session = connect(&store, &id)?;
        CodexRemote::new(
            session.as_ref(),
            session.home(),
            &store.journal_dir(&id)?,
            &paths.app_data_dir,
            official(&paths),
        )
        .preview(
            &router::load_store(&paths)?,
            &request,
            replace_conflicts.unwrap_or(false),
        )
    })
    .await
}

#[tauri::command]
pub async fn apply_remote_codex(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    request: CodexRemoteRequest,
    revision: String,
    replace_conflicts: Option<bool>,
) -> Result<CodexRemoteApplied, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let session = connect(&store, &id)?;
        CodexRemote::new(
            session.as_ref(),
            session.home(),
            &store.journal_dir(&id)?,
            &paths.app_data_dir,
            official(&paths),
        )
        .apply(
            &router::load_store(&paths)?,
            &request,
            &revision,
            replace_conflicts.unwrap_or(false),
        )
    })
    .await
}

#[tauri::command]
pub async fn restore_remote_codex(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    replace_conflicts: Option<bool>,
) -> Result<CodexRemoteApplied, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let session = connect(&store, &id)?;
        CodexRemote::new(
            session.as_ref(),
            session.home(),
            &store.journal_dir(&id)?,
            &paths.app_data_dir,
            Vec::new(),
        )
        .restore(replace_conflicts.unwrap_or(false))
    })
    .await
}

/// Stops managing Codex on the host without connecting to it.
#[tauri::command]
pub async fn release_remote_codex(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let offline = RemoteSession::offline();
        CodexRemote::new(
            &offline,
            "",
            &store.journal_dir(&id)?,
            &paths.app_data_dir,
            Vec::new(),
        )
        .release()
    })
    .await
}

#[tauri::command]
pub async fn preview_remote_claude(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    request: ConfigRequest,
    replace_conflicts: Option<bool>,
) -> Result<ConfigPreview, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let session = connect(&store, &id)?;
        claude(&paths, &store, &id, session)?.preview_with_conflicts(
            &router::load_store(&paths)?,
            &claude_request(request)?,
            replace_conflicts.unwrap_or(false),
        )
    })
    .await
}

#[tauri::command]
pub async fn apply_remote_claude(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    request: ConfigRequest,
    revision: String,
    replace_conflicts: Option<bool>,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let session = connect(&store, &id)?;
        claude(&paths, &store, &id, session)?.apply_with_conflicts(
            &router::load_store(&paths)?,
            &claude_request(request)?,
            &revision,
            replace_conflicts.unwrap_or(false),
        )
    })
    .await
}

#[tauri::command]
pub async fn restore_remote_claude(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    replace_conflicts: Option<bool>,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        let session = connect(&store, &id)?;
        claude(&paths, &store, &id, session)?
            .restore_confirmed(TargetId::ClaudeCode, replace_conflicts.unwrap_or(false))
    })
    .await
}

/// Stops managing Claude Code on the host without connecting to it.
#[tauri::command]
pub async fn release_remote_claude(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    blocking(move || {
        let store = RemoteStore::new(&paths.app_data_dir);
        claude(&paths, &store, &id, Arc::new(RemoteSession::offline()))?
            .release(TargetId::ClaudeCode)
    })
    .await
}

/// Remote Claude Code is direct only, whatever the dialog sent.
fn claude_request(request: ConfigRequest) -> Result<ConfigRequest, String> {
    if request.target_id != TargetId::ClaudeCode {
        return Err("clients.errors.unsupported".into());
    }
    Ok(request)
}
