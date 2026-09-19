use crate::{core::{newapi::{self, AppliedConfig, Balance, Connection, ConnectionView, Model}, repository::Repository}, platform::paths::CodexPaths};
use std::sync::Mutex;
use tauri::State;

fn paths(repo: &State<'_, Mutex<Repository>>) -> Result<CodexPaths, String> {
    Ok(repo.lock().map_err(|_| "newapi.errors.storage")?.paths().clone())
}

#[tauri::command]
pub fn load_newapi_connection(repo: State<'_, Mutex<Repository>>) -> Result<ConnectionView, String> {
    Ok(newapi::load(&paths(&repo)?)?.view())
}

#[tauri::command]
pub fn save_newapi_connection(repo: State<'_, Mutex<Repository>>, connection: Connection) -> Result<ConnectionView, String> {
    newapi::save(&paths(&repo)?, connection)
}

#[tauri::command]
pub fn forget_newapi_connection(repo: State<'_, Mutex<Repository>>) -> Result<(), String> {
    newapi::forget(&paths(&repo)?)
}

#[tauri::command]
pub async fn query_newapi_balance(repo: State<'_, Mutex<Repository>>) -> Result<Balance, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || newapi::balance(&newapi::load(&paths)?, &newapi::proxy_from_settings(&paths.settings_path)))
        .await.map_err(|_| "newapi.errors.network")?
}

#[tauri::command]
pub async fn fetch_newapi_models(repo: State<'_, Mutex<Repository>>) -> Result<Vec<Model>, String> {
    let paths = paths(&repo)?;
    tauri::async_runtime::spawn_blocking(move || newapi::models(&newapi::load(&paths)?, &newapi::proxy_from_settings(&paths.settings_path)))
        .await.map_err(|_| "newapi.errors.network")?
}

#[tauri::command]
pub fn preview_newapi_config(repo: State<'_, Mutex<Repository>>) -> Result<String, String> {
    newapi::config_snippet(&newapi::load(&paths(&repo)?)?)
}

#[tauri::command]
pub fn apply_newapi_config(repo: State<'_, Mutex<Repository>>, write_api_key: bool) -> Result<AppliedConfig, String> {
    let repo = repo.lock().map_err(|_| "newapi.errors.storage")?;
    newapi::apply_config(repo.paths(), &newapi::load(repo.paths())?, write_api_key)
}
