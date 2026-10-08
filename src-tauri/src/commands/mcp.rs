use crate::core::auth::current_timestamp;
use crate::core::mcp;
use crate::core::models::*;
use crate::core::repository::Repository;
use crate::core::targets::TargetId;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

#[tauri::command]
pub fn load_mcp_servers(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
) -> Result<CoreEnvelope<McpServerListPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let config = mcp::McpConfig::for_target(repo.paths(), target_id.unwrap_or(TargetId::Codex))
        .map_err(|e| e.to_string())?;
    let items = config.load().map_err(|e| e.to_string())?;
    let payload = McpServerListPayload {
        total: items.len() as i32,
        source_path: config.source_path(),
        last_scan_at: current_timestamp(),
        items,
    };
    Ok(CoreEnvelope::ok(payload))
}

#[tauri::command]
pub fn upsert_mcp_server(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
    name: String,
    transport: String,
    enabled: bool,
    command: Option<String>,
    args: Vec<String>,
    url: Option<String>,
    headers: HashMap<String, String>,
    environment: HashMap<String, String>,
) -> Result<CoreEnvelope<McpServerMutationPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let config = mcp::McpConfig::for_target(repo.paths(), target_id.unwrap_or(TargetId::Codex))
        .map_err(|e| e.to_string())?;
    let transport_enum = match transport.as_str() {
        "stdio" => McpTransport::Stdio,
        "http" => McpTransport::Http,
        "sse" => McpTransport::Sse,
        _ => McpTransport::Unknown,
    };
    let server = McpServerSummary {
        name,
        transport: transport_enum,
        enabled,
        source_path: config.source_path(),
        command,
        args,
        url,
        headers,
        environment,
    };
    let saved = config.upsert(&server).map_err(|e| e.to_string())?;
    let all = config.load().map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(McpServerMutationPayload {
        server: saved,
        total: all.len() as i32,
        source_path: config.source_path(),
    }))
}

#[tauri::command]
pub fn set_mcp_server_enabled(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
    name: String,
    enabled: bool,
) -> Result<CoreEnvelope<McpServerMutationPayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let config = mcp::McpConfig::for_target(repo.paths(), target_id.unwrap_or(TargetId::Codex))
        .map_err(|e| e.to_string())?;
    let saved = config
        .set_enabled(&name, enabled)
        .map_err(|e| e.to_string())?;
    let all = config.load().map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(McpServerMutationPayload {
        server: saved,
        total: all.len() as i32,
        source_path: config.source_path(),
    }))
}

#[tauri::command]
pub fn remove_mcp_server(
    repo: State<'_, Mutex<Repository>>,
    target_id: Option<TargetId>,
    name: String,
) -> Result<CoreEnvelope<McpServerRemovePayload>, String> {
    let repo = repo.lock().map_err(|e| e.to_string())?;
    let config = mcp::McpConfig::for_target(repo.paths(), target_id.unwrap_or(TargetId::Codex))
        .map_err(|e| e.to_string())?;
    config.remove(&name).map_err(|e| e.to_string())?;
    let all = config.load().map_err(|e| e.to_string())?;
    Ok(CoreEnvelope::ok(McpServerRemovePayload {
        removed_name: name,
        total: all.len() as i32,
        source_path: config.source_path(),
    }))
}
