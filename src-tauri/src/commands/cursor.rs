//! Cursor injection: Cursor IDE and `cursor-agent` on AgentHarbor's models.
use crate::core::{cursor_inject, repository::Repository, router::ModelSources, routing_runtime};
use crate::platform::paths::CodexPaths;
use cursor_engine::CommitLocale;
use std::sync::Mutex;
use tauri::State;

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo.lock().map_err(|_| "cursor.errors.storage")?.paths().clone())
}

async fn status(paths: CodexPaths) -> Result<cursor_inject::CursorStatus, String> {
    super::blocking(move || cursor_inject::status(&paths)).await
}

#[tauri::command]
pub async fn load_cursor_status(repo: State<'_, Mutex<Repository>>) -> Result<cursor_inject::CursorStatus, String> {
    status(paths(&repo)?).await
}

#[tauri::command]
pub async fn set_cursor_ide(repo: State<'_, Mutex<Repository>>, enabled: bool) -> Result<cursor_inject::CursorStatus, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    cursor_inject::set_ide(&paths, enabled).await?;
    status(paths).await
}

#[tauri::command]
pub async fn set_cursor_cli(repo: State<'_, Mutex<Repository>>, enabled: bool) -> Result<cursor_inject::CursorStatus, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    cursor_inject::set_cli(&paths, enabled).await?;
    status(paths).await
}

#[tauri::command]
pub async fn set_cursor_sources(repo: State<'_, Mutex<Repository>>, sources: ModelSources) -> Result<cursor_inject::CursorStatus, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    cursor_inject::set_sources(&paths, sources).await?;
    status(paths).await
}

#[tauri::command]
pub async fn set_cursor_tab_model(repo: State<'_, Mutex<Repository>>, model: Option<String>) -> Result<cursor_inject::CursorStatus, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    cursor_inject::set_tab(&paths, model).await?;
    status(paths).await
}

#[tauri::command]
pub async fn set_cursor_commit_model(
    repo: State<'_, Mutex<Repository>>,
    model: Option<String>,
    locale: CommitLocale,
) -> Result<cursor_inject::CursorStatus, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    cursor_inject::set_commit(&paths, model, locale).await?;
    status(paths).await
}

/// Windows shows its own confirmation before the certificate is trusted.
#[tauri::command]
pub async fn trust_cursor_certificate(repo: State<'_, Mutex<Repository>>) -> Result<cursor_inject::CursorStatus, String> {
    let paths = paths(&repo)?;
    let trusting = paths.clone();
    super::blocking(move || cursor_inject::trust(&trusting)).await?;
    status(paths).await
}

#[tauri::command]
pub async fn distrust_cursor_certificate(repo: State<'_, Mutex<Repository>>) -> Result<cursor_inject::CursorStatus, String> {
    let paths = paths(&repo)?;
    let _guard = routing_runtime::mutation_guard().await;
    cursor_inject::distrust(&paths).await?;
    status(paths).await
}

#[tauri::command]
pub async fn restart_cursor_ide() -> Result<(), String> {
    super::blocking(cursor_inject::ide::restart).await
}

#[tauri::command]
pub async fn open_cursor_terminal() -> Result<(), String> {
    super::blocking(cursor_inject::open_terminal).await
}
