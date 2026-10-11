pub mod account_sources;
pub mod accounts;
pub mod aggregation;
pub mod auto_switch;
pub mod claude_desktop;
pub mod client_config;
pub mod custom_instructions;
pub mod desktop_exit;
pub mod gateway;
pub mod hotspot;
pub mod image;
pub mod mcp;
pub mod newapi;
pub mod skills;
pub mod system;
pub mod targets;
pub mod thread_recovery;
pub mod threads;
pub mod tray_menu;
pub mod usage;

use crate::core::repository::Repository;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Sync commands run on the main thread and freeze the window while they work.
/// Filesystem-heavy commands move their work onto the blocking pool instead.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

/// Like [`blocking`], holding the repository lock only on the blocking thread.
pub(crate) async fn with_repo<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&Repository) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    blocking(move || {
        let state = app.state::<Mutex<Repository>>();
        let repo = state.lock().map_err(|error| error.to_string())?;
        work(&repo)
    })
    .await
}
