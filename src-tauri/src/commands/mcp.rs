use crate::core::mcp::{self, McpApp, McpContext, McpLibraryView, McpServerEntry, PastedServer};
use crate::core::repository::Repository;
use std::sync::Mutex;
use tauri::State;

fn context(repo: &State<'_, Mutex<Repository>>) -> Result<McpContext, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    Ok(McpContext::discover(repo.paths()))
}

/// The MCP library and each agent's state; seeded from the agents the first time.
#[tauri::command]
pub async fn load_mcp_library(
    repo: State<'_, Mutex<Repository>>,
) -> Result<McpLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || mcp::load_library(&context)).await
}

#[tauri::command]
pub async fn import_mcp_servers(
    repo: State<'_, Mutex<Repository>>,
) -> Result<McpLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || mcp::import_servers(&context)).await
}

/// Add a server (`original` absent) or change one, and write it to the agents it is on.
#[tauri::command]
pub async fn save_mcp_server(
    repo: State<'_, Mutex<Repository>>,
    original: Option<String>,
    server: McpServerEntry,
) -> Result<McpLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || mcp::save_server(&context, original.as_deref(), server)).await
}

/// Turn servers on or off for one agent, or for every agent when `app` is absent.
#[tauri::command]
pub async fn set_mcp_servers(
    repo: State<'_, Mutex<Repository>>,
    names: Vec<String>,
    app: Option<McpApp>,
    on: bool,
) -> Result<McpLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || mcp::set_servers(&context, &names, app, on)).await
}

#[tauri::command]
pub async fn remove_mcp_server(
    repo: State<'_, Mutex<Repository>>,
    name: String,
) -> Result<McpLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || mcp::remove_server(&context, &name)).await
}

/// Write every server again to the agents it is on.
#[tauri::command]
pub async fn resync_mcp_servers(
    repo: State<'_, Mutex<Repository>>,
) -> Result<McpLibraryView, String> {
    let context = context(&repo)?;
    super::blocking(move || mcp::resync_servers(&context)).await
}

/// The servers in pasted JSON, TOML or a command line, for the add form.
#[tauri::command]
pub fn parse_mcp_paste(text: String) -> Result<Vec<PastedServer>, String> {
    mcp::parse_pasted(&text)
}
