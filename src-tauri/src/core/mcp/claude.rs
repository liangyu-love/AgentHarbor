use super::*;
use serde_json::{json, Map, Value};
use std::{fs, path::Path};

fn read(path: &Path) -> Result<Option<Vec<u8>>, CoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn document(bytes: Option<&[u8]>) -> Result<Value, CoreError> {
    let value: Value = match bytes {
        Some(bytes) => serde_json::from_slice(bytes)?,
        None => json!({}),
    };
    if !value.is_object() || value.get("mcpServers").is_some_and(|v| !v.is_object()) {
        return Err(CoreError::InvalidData(
            "Invalid Claude Code MCP configuration".into(),
        ));
    }
    if value
        .get("disabledMcpServers")
        .is_some_and(|v| !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
    {
        return Err(CoreError::InvalidData(
            "Invalid Claude disabledMcpServers list".into(),
        ));
    }
    Ok(value)
}

fn strings(value: &Value) -> HashMap<String, String> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.into())))
        .collect()
}

fn summaries(path: &Path, value: &Value) -> Vec<McpServerSummary> {
    let disabled = value["disabledMcpServers"].as_array();
    let mut items: Vec<_> = value["mcpServers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, spec)| McpServerSummary {
            name: name.clone(),
            transport: match spec["type"].as_str().unwrap_or("stdio") {
                "stdio" => McpTransport::Stdio,
                "http" | "streamable-http" => McpTransport::Http,
                "sse" => McpTransport::Sse,
                _ => McpTransport::Unknown,
            },
            enabled: !disabled
                .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(name.as_str()))),
            source_path: path.display().to_string(),
            command: spec["command"].as_str().map(str::to_owned),
            args: spec["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            url: spec["url"].as_str().map(str::to_owned),
            headers: strings(&spec["headers"]),
            environment: strings(&spec["env"]),
        })
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    items
}

pub(super) fn load(path: &Path) -> Result<Vec<McpServerSummary>, CoreError> {
    Ok(summaries(path, &document(read(path)?.as_deref())?))
}

/// Read-modify-write the latest user file; retain project entries, auth and
/// fields outside the selected server. Reject an external edit before replace.
fn update(
    path: &Path,
    edit: impl FnOnce(&mut Value) -> Result<(), CoreError>,
) -> Result<(), CoreError> {
    let before = read(path)?;
    let mut value = document(before.as_deref())?;
    edit(&mut value)?;
    if read(path)? != before {
        return Err(CoreError::OperationFailed(
            "Claude configuration changed; refresh and retry".into(),
        ));
    }
    if let Some(bytes) = before {
        crate::core::secret_file::write(&path.with_extension("agentharbor-backup.dat"), &bytes)
            .map_err(CoreError::OperationFailed)?;
    }
    crate::core::storage::write_atomic(path, &serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

fn disable(value: &mut Value, name: &str, enabled: bool) {
    let list = value
        .as_object_mut()
        .unwrap()
        .entry("disabledMcpServers")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .unwrap();
    list.retain(|v| v.as_str() != Some(name));
    if !enabled {
        list.push(json!(name));
    }
}

pub(super) fn upsert(
    path: &Path,
    server: &McpServerSummary,
) -> Result<McpServerSummary, CoreError> {
    // Use the same transport/name checks as Codex without writing a TOML file.
    render_mcp_update("", server)?;
    update(path, |value| {
        let servers = value
            .as_object_mut()
            .unwrap()
            .entry("mcpServers")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap();
        let mut spec = match servers.get(&server.name) {
            Some(v) => v
                .as_object()
                .cloned()
                .ok_or_else(|| CoreError::InvalidData("Invalid MCP server entry".into()))?,
            None => Map::new(),
        };
        for field in ["type", "command", "args", "url", "env", "headers"] {
            spec.remove(field);
        }
        match server.transport {
            McpTransport::Stdio => {
                spec.insert("type".into(), json!("stdio"));
                spec.insert("command".into(), json!(server.command));
                spec.insert("args".into(), json!(server.args));
            }
            McpTransport::Http | McpTransport::Sse => {
                spec.insert(
                    "type".into(),
                    json!(if server.transport == McpTransport::Http {
                        "http"
                    } else {
                        "sse"
                    }),
                );
                spec.insert("url".into(), json!(server.url));
                if !server.headers.is_empty() {
                    spec.insert("headers".into(), json!(server.headers));
                }
            }
            McpTransport::Unknown => unreachable!(),
        }
        if !server.environment.is_empty() {
            spec.insert("env".into(), json!(server.environment));
        }
        servers.insert(server.name.clone(), Value::Object(spec));
        disable(value, &server.name, server.enabled);
        Ok(())
    })?;
    load(path)?
        .into_iter()
        .find(|s| s.name == server.name)
        .ok_or_else(|| CoreError::NotFound(server.name.clone()))
}

pub(super) fn set_enabled(
    path: &Path,
    name: &str,
    enabled: bool,
) -> Result<McpServerSummary, CoreError> {
    update(path, |value| {
        if value["mcpServers"].get(name).is_none() {
            return Err(CoreError::NotFound(name.into()));
        }
        disable(value, name, enabled);
        Ok(())
    })?;
    load(path)?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| CoreError::NotFound(name.into()))
}

pub(super) fn remove(path: &Path, name: &str) -> Result<(), CoreError> {
    update(path, |value| {
        value["mcpServers"]
            .as_object_mut()
            .and_then(|v| v.remove(name))
            .ok_or_else(|| CoreError::NotFound(name.into()))?;
        disable(value, name, true);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(transport: McpTransport) -> McpServerSummary {
        McpServerSummary {
            name: "docs".into(),
            transport,
            enabled: true,
            source_path: "ignored".into(),
            command: Some("node".into()),
            args: vec!["server.js".into()],
            url: Some("https://example.invalid/mcp".into()),
            headers: HashMap::from([("Authorization".into(), "Bearer test-only".into())]),
            environment: HashMap::from([("TOKEN".into(), "${TOKEN}".into())]),
        }
    }

    #[test]
    fn crud_and_disable_use_claude_schema_and_preserve_projects_auth_and_unknown_keys() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".claude.json");
        let original = json!({"oauthAccount":{"id":"test"},"projects":{"C:/work":{"mcpServers":{"local":{"command":"local"}},"disabledMcpServers":["docs"]}},"mcpServers":{"docs":{"type":"http","url":"https://old.invalid","oauth":{"clientId":"keep"}},"neighbor":{"command":"keep"}},"disabledMcpServers":["neighbor"]});
        fs::write(&path, original.to_string()).unwrap();
        let result = upsert(&path, &server(McpTransport::Sse)).unwrap();
        assert_eq!(result.transport, McpTransport::Sse);
        assert_eq!(result.source_path, path.display().to_string());
        assert!(result.enabled);
        let raw = document(read(&path).unwrap().as_deref()).unwrap();
        assert_eq!(raw["projects"], original["projects"]);
        assert_eq!(raw["oauthAccount"], original["oauthAccount"]);
        assert_eq!(
            raw["mcpServers"]["docs"]["oauth"],
            original["mcpServers"]["docs"]["oauth"]
        );
        assert_eq!(
            raw["mcpServers"]["neighbor"],
            original["mcpServers"]["neighbor"]
        );
        assert!(!set_enabled(&path, "docs", false).unwrap().enabled);
        let raw = document(read(&path).unwrap().as_deref()).unwrap();
        assert_eq!(raw["disabledMcpServers"], json!(["neighbor", "docs"]));
        assert!(raw["mcpServers"]["docs"].get("enabled").is_none());
        let result = upsert(&path, &server(McpTransport::Stdio)).unwrap();
        assert_eq!(result.args, vec!["server.js"]);
        let raw = document(read(&path).unwrap().as_deref()).unwrap();
        assert!(raw["mcpServers"]["docs"].get("url").is_none());
        assert!(raw["mcpServers"]["docs"].get("headers").is_none());
        assert_eq!(result.environment["TOKEN"], "${TOKEN}");
        remove(&path, "docs").unwrap();
        let raw = document(read(&path).unwrap().as_deref()).unwrap();
        assert!(raw["mcpServers"].get("docs").is_none());
        assert_eq!(raw["disabledMcpServers"], json!(["neighbor"]));
        assert!(
            crate::core::secret_file::read(&path.with_extension("agentharbor-backup.dat"))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn invalid_input_does_not_overwrite_config_and_missing_file_is_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".claude.json");
        assert!(load(&path).unwrap().is_empty());
        assert!(!path.exists());
        let mut bad = server(McpTransport::Http);
        bad.url = Some("file:///private".into());
        assert!(upsert(&path, &bad).is_err());
        assert!(!path.exists());
        for broken in [
            "{",
            "[]",
            r#"{"mcpServers":[]}"#,
            r#"{"disabledMcpServers":"docs"}"#,
        ] {
            fs::write(&path, broken).unwrap();
            assert!(upsert(&path, &server(McpTransport::Http)).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), broken);
        }
    }

    #[test]
    fn path_contract_covers_default_and_custom_claude_root() {
        let clients = crate::core::client_config::ClientPaths::from_home(Path::new("home"));
        assert_eq!(clients.claude_user_config(), Path::new("home/.claude.json"));
        let custom = crate::core::client_config::ClientPaths {
            claude: Path::new("custom/settings.json").into(),
            claude_user: Path::new("custom/.claude.json").into(),
            opencode: Path::new("unused").into(),
            pi: Path::new("unused-pi").into(),
            cursor: Path::new("unused-cursor").into(),
            cursor_cli: Path::new("unused-cursor-cli").into(),
        };
        assert_eq!(
            custom.claude_user_config(),
            Path::new("custom/.claude.json")
        );
        let named_default = crate::core::client_config::ClientPaths {
            claude: Path::new("custom/.claude/settings.json").into(),
            claude_user: Path::new("custom/.claude/.claude.json").into(),
            opencode: Path::new("unused").into(),
            pi: Path::new("unused-pi").into(),
            cursor: Path::new("unused-cursor").into(),
            cursor_cli: Path::new("unused-cursor-cli").into(),
        };
        assert_eq!(
            named_default.claude_user_config(),
            Path::new("custom/.claude/.claude.json")
        );
    }

    #[test]
    fn concurrent_external_changes_are_not_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".claude.json");
        fs::write(&path, "{}").unwrap();
        let external = r#"{"oauthAccount":{"id":"new-account"}}"#;
        assert!(update(&path, |value| {
            value["mcpServers"] = json!({"new": {"command":"node"}});
            fs::write(&path, external)?;
            Ok(())
        })
        .is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), external);
        assert!(!path.with_extension("agentharbor-backup.dat").exists());
    }
}
