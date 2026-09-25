use crate::core::{
    local_usage::{self, LocalTokenUsagePayload},
    relay_usage::{self, RelayUsagePayload},
    repository::Repository,
};
use chrono::Utc;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsagePayload {
    pub generated_at: i64,
    pub local: LocalTokenUsagePayload,
    pub relay: RelayUsagePayload,
}

#[tauri::command]
pub async fn get_token_usage(
    repo: State<'_, Mutex<Repository>>,
) -> Result<TokenUsagePayload, String> {
    let (codex_home, data_dir) = {
        let repo = repo.lock().map_err(|error| error.to_string())?;
        (
            repo.paths().codex_home.clone(),
            repo.paths().app_data_dir.clone(),
        )
    };
    let claude_home = if let Some(path) = std::env::var_os("AGENT_HARBOR_CLAUDE_HOME")
        .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR"))
    {
        PathBuf::from(path)
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".claude")
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<TokenUsagePayload, String> {
        let local = local_usage::load(&codex_home, &claude_home, &data_dir)?;
        let relay = relay_usage::load(&data_dir)?;
        Ok(TokenUsagePayload {
            generated_at: Utc::now().timestamp(),
            local,
            relay,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}
