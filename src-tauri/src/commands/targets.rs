use crate::core::{
    client_launch,
    repository::Repository,
    targets::{TargetId, TargetRegistry},
};
use std::sync::Mutex;
use tauri::State;

/// Restarting waits for the process to exit, so run it off the main thread and
/// without holding the repository lock that every other command needs.
#[tauri::command]
pub async fn restart_target(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
) -> Result<(), String> {
    let paths = repo
        .lock()
        .map_err(|error| error.to_string())?
        .paths()
        .clone();
    tauri::async_runtime::spawn_blocking(move || {
        let registry = TargetRegistry::new(paths);
        let controller = registry
            .process(target_id)
            .ok_or_else(|| "Unsupported target process controller".to_owned())?;
        controller.restart().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// How a client starts from its panel: a terminal, an app, or a command.
#[tauri::command]
pub async fn load_client_launch(
    target_id: TargetId,
    harness_profile: Option<String>,
) -> Result<client_launch::Launch, String> {
    super::blocking(move || Ok(client_launch::plan(target_id, harness_profile.as_deref()))).await
}

/// Starts a client: a CLI in a new terminal in `folder`, an app restarted.
#[tauri::command]
pub async fn launch_client(
    repo: State<'_, Mutex<Repository>>,
    target_id: TargetId,
    harness_profile: Option<String>,
    folder: Option<String>,
) -> Result<(), String> {
    let paths = repo
        .lock()
        .map_err(|error| error.to_string())?
        .paths()
        .clone();
    super::blocking(move || {
        client_launch::launch(
            target_id,
            harness_profile.as_deref(),
            folder.as_deref().map(std::path::Path::new),
            &client_launch::direct_relay_hosts(&paths, target_id),
        )
    })
    .await
}
