//! Remote hosts and the Codex and Claude Code configuration on them. Each
//! command opens its own SSH session on the blocking pool and closes it.
//! A change that may start or end a routed configuration reconciles the
//! router afterwards, which also starts or stops the host's tunnel.

use super::blocking;
use crate::core::{
    client_config::{
        ClientFiles, ClientStatus, ConfigApplied, ConfigManager, ConfigPreview, ConfigRequest,
    },
    model_catalog,
    remote::{
        codex::{
            self, CodexRemote, CodexRemoteApplied, CodexRemotePreview, CodexRemoteRequest,
            CodexRemoteStatus, RouterModel,
        },
        ConnectionTest, RemoteHostInput, RemoteHostView, RemoteSession, RemoteStore,
    },
    repository::Repository,
    router::{self, Store},
    routing_runtime,
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

/// The relays, with the router reached at host `id`'s tunnel port: that is
/// the address a routed configuration on the host is written with.
fn relays(paths: &CodexPaths, store: &RemoteStore, id: &str) -> Result<Store, String> {
    let mut relays = router::load_store(paths)?;
    relays.router.port = store.tunnel_port(id)?;
    Ok(relays)
}

fn codex<'a>(
    paths: &CodexPaths,
    store: &RemoteStore,
    id: &str,
    files: &'a dyn ClientFiles,
    home: &str,
) -> Result<CodexRemote<'a>, String> {
    Ok(CodexRemote::new(
        files,
        home,
        &store.journal_dir(id)?,
        &paths.app_data_dir,
        model_catalog::official(&paths.codex_home, &paths.app_data_dir).models,
    )
    .with_router(&paths.codex_home, store.tunnel_port(id)?))
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

/// The router and the tunnels follow what is now configured.
async fn settle(paths: &CodexPaths) {
    if let Err(error) = routing_runtime::reconcile(paths).await {
        log::warn!("router reconcile after a remote change failed: {error}");
    }
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
    let saved = blocking({
        let paths = paths.clone();
        move || RemoteStore::new(&paths.app_data_dir).save_host(input)
    })
    .await?;
    // A changed address or sign-in reaches a running tunnel on its next start.
    settle(&paths).await;
    Ok(saved)
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
        let relays = relays(&paths, &store, &id).ok();
        let codex =
            codex(&paths, &store, &id, session.as_ref(), session.home())?.status(relays.as_ref());
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

/// The models a routed Codex on a remote host is offered: the router's own
/// menu, without official models.
#[tauri::command]
pub async fn remote_codex_router_models(
    repo: State<'_, Mutex<Repository>>,
) -> Result<Vec<RouterModel>, String> {
    let paths = paths(&repo)?;
    blocking(move || {
        Ok(codex::router_menu(
            &paths.app_data_dir,
            &paths.codex_home,
            &router::load_store(&paths)?,
        ))
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
        codex(&paths, &store, &id, session.as_ref(), session.home())?.preview(
            &relays(&paths, &store, &id)?,
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
    let _guard = routing_runtime::mutation_guard().await;
    if request.router {
        routing_runtime::ensure_available(&paths).await?;
    }
    let applied = blocking({
        let paths = paths.clone();
        move || {
            let store = RemoteStore::new(&paths.app_data_dir);
            let session = connect(&store, &id)?;
            codex(&paths, &store, &id, session.as_ref(), session.home())?.apply(
                &relays(&paths, &store, &id)?,
                &request,
                &revision,
                replace_conflicts.unwrap_or(false),
            )
        }
    })
    .await;
    settle(&paths).await;
    applied
}

#[tauri::command]
pub async fn restore_remote_codex(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    replace_conflicts: Option<bool>,
) -> Result<CodexRemoteApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let restored = blocking({
        let paths = paths.clone();
        move || {
            let store = RemoteStore::new(&paths.app_data_dir);
            let session = connect(&store, &id)?;
            codex(&paths, &store, &id, session.as_ref(), session.home())?
                .restore(replace_conflicts.unwrap_or(false))
        }
    })
    .await;
    settle(&paths).await;
    restored
}

/// Stops managing Codex on the host without connecting to it.
#[tauri::command]
pub async fn release_remote_codex(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let released = blocking({
        let paths = paths.clone();
        move || {
            let store = RemoteStore::new(&paths.app_data_dir);
            let offline = RemoteSession::offline();
            codex(&paths, &store, &id, &offline, "")?.release()
        }
    })
    .await;
    settle(&paths).await;
    released
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
            &relays(&paths, &store, &id)?,
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
    let request = claude_request(request)?;
    let _guard = routing_runtime::mutation_guard().await;
    if request.needs_router() {
        routing_runtime::ensure_available(&paths).await?;
    }
    let applied = blocking({
        let paths = paths.clone();
        move || {
            let store = RemoteStore::new(&paths.app_data_dir);
            let session = connect(&store, &id)?;
            claude(&paths, &store, &id, session)?.apply_with_conflicts(
                &relays(&paths, &store, &id)?,
                &request,
                &revision,
                replace_conflicts.unwrap_or(false),
            )
        }
    })
    .await;
    settle(&paths).await;
    applied
}

#[tauri::command]
pub async fn restore_remote_claude(
    repo: State<'_, Mutex<Repository>>,
    id: String,
    replace_conflicts: Option<bool>,
) -> Result<ConfigApplied, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let restored = blocking({
        let paths = paths.clone();
        move || {
            let store = RemoteStore::new(&paths.app_data_dir);
            let session = connect(&store, &id)?;
            claude(&paths, &store, &id, session)?
                .restore_confirmed(TargetId::ClaudeCode, replace_conflicts.unwrap_or(false))
        }
    })
    .await;
    settle(&paths).await;
    restored
}

/// Stops managing Claude Code on the host without connecting to it.
#[tauri::command]
pub async fn release_remote_claude(
    repo: State<'_, Mutex<Repository>>,
    id: String,
) -> Result<(), String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    let released = blocking({
        let paths = paths.clone();
        move || {
            let store = RemoteStore::new(&paths.app_data_dir);
            claude(&paths, &store, &id, Arc::new(RemoteSession::offline()))?
                .release(TargetId::ClaudeCode)
        }
    })
    .await;
    settle(&paths).await;
    released
}

fn claude_request(request: ConfigRequest) -> Result<ConfigRequest, String> {
    if request.target_id != TargetId::ClaudeCode {
        return Err("clients.errors.unsupported".into());
    }
    Ok(request)
}
