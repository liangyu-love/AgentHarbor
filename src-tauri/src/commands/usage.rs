use crate::core::{
    relay_usage::{self, RelayUsagePayload},
    repository::Repository,
};
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn get_relay_usage(repo: State<'_, Mutex<Repository>>) -> Result<RelayUsagePayload, String> {
    let data_dir = repo
        .lock()
        .map_err(|_| "Repository lock poisoned")?
        .paths()
        .app_data_dir
        .clone();
    relay_usage::load(&data_dir)
}
