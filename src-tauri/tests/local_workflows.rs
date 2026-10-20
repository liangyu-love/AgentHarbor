use agentharbor_lib::core::{custom_instructions, mcp};
use agentharbor_lib::platform::paths::CodexPaths;
use std::fs;

#[test]
fn instruction_rollback_keeps_later_user_edits() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    let instructions = custom_instructions::InstructionPaths::for_target(&paths, &agentharbor_lib::core::client_config::ClientPaths::from_home(temp.path()), agentharbor_lib::core::targets::TargetId::Codex).unwrap();
    paths.ensure_directories().unwrap();
    fs::write(&paths.global_agents_path, "User rules\n").unwrap();
    let state = custom_instructions::apply_managed_content(
        &instructions,
        "Managed first",
        Some("test".into()),
        None,
        None,
    )
    .unwrap();
    let before_first_apply = state.history[0].id.clone();
    let text = fs::read_to_string(&paths.global_agents_path).unwrap();
    fs::write(
        &paths.global_agents_path,
        format!("{text}\nLater user rules\n"),
    )
    .unwrap();
    let preview = custom_instructions::preview_apply(&instructions, "Managed second").unwrap();
    assert!(preview.resulting_content.contains("Later user rules"));
    custom_instructions::rollback_history(&instructions, &before_first_apply).unwrap();
    let restored = fs::read_to_string(&paths.global_agents_path).unwrap();
    assert!(restored.contains("User rules"));
    assert!(restored.contains("Later user rules"));
    assert!(!restored.contains("Managed first"));
    assert!(!restored.contains("AGENT_HARBOR_CUSTOM_INSTRUCTIONS_START"));
}

#[test]
fn broken_instruction_markers_refuse_writes_and_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    let instructions = custom_instructions::InstructionPaths::for_target(&paths, &agentharbor_lib::core::client_config::ClientPaths::from_home(temp.path()), agentharbor_lib::core::targets::TargetId::Codex).unwrap();
    let state =
        custom_instructions::apply_managed_content(&instructions, "Managed", None, None, None).unwrap();
    let broken = "User rules\n<!-- AGENT_HARBOR_CUSTOM_INSTRUCTIONS_START -->\nbroken";
    fs::write(&paths.global_agents_path, broken).unwrap();
    assert!(custom_instructions::preview_apply(&instructions, "new").is_err());
    assert!(custom_instructions::rollback_history(&instructions, &state.history[0].id).is_err());
    assert_eq!(
        fs::read_to_string(&paths.global_agents_path).unwrap(),
        broken
    );
}

#[test]
fn instruction_history_rejects_paths_outside_its_directory() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    let instructions = custom_instructions::InstructionPaths::for_target(&paths, &agentharbor_lib::core::client_config::ClientPaths::from_home(temp.path()), agentharbor_lib::core::targets::TargetId::Codex).unwrap();
    paths.ensure_directories().unwrap();
    fs::write(&paths.global_agents_path, "Keep user rules").unwrap();
    assert!(custom_instructions::rollback_history(&instructions, "../outside").is_err());
    assert!(custom_instructions::rollback_history(&instructions, r"..\outside").is_err());
    assert_eq!(
        fs::read_to_string(&paths.global_agents_path).unwrap(),
        "Keep user rules"
    );
}

/// The MCP library over a temporary home, with only Codex installed.
fn mcp_context(temp: &std::path::Path, paths: &CodexPaths) -> mcp::McpContext {
    fs::create_dir_all(&paths.app_data_dir).unwrap();
    let clients = agentharbor_lib::core::client_config::ClientPaths::from_home(&temp.join("home"));
    mcp::McpContext::new(paths, &clients, None)
}

fn codex_server(name: &str, command: &str, args: &[&str]) -> mcp::McpServerEntry {
    mcp::McpServerEntry {
        name: name.into(),
        spec: mcp::McpSpec {
            transport: mcp::McpKind::Stdio,
            command: Some(command.into()),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            ..Default::default()
        },
        note: String::new(),
        apps: vec![mcp::McpApp::Codex],
    }
}

#[test]
fn mcp_crud_keeps_other_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    fs::create_dir_all(&paths.codex_home).unwrap();
    fs::write(&paths.config_path, "# User settings\nmodel = \"test-model\"\n\n[projects.\"D:\\\\project\"]\ntrust_level = \"trusted\"\n").unwrap();
    let context = mcp_context(temp.path(), &paths);
    mcp::load_library(&context).unwrap();
    let view = mcp::save_server(&context, None, codex_server("test-service", "node", &["server.js"])).unwrap();
    assert!(view.failures.is_empty(), "{:?}", view.failures);
    assert_eq!(mcp::read_codex_servers(&paths).unwrap().len(), 1);
    mcp::set_servers(&context, &["test-service".into()], Some(mcp::McpApp::Codex), false).unwrap();
    assert!(mcp::read_codex_servers(&paths).unwrap().is_empty());
    mcp::save_server(&context, Some("test-service"), codex_server("test-service", "python", &[])).unwrap();
    let servers = mcp::read_codex_servers(&paths).unwrap();
    assert_eq!(servers[0].1["command"].as_str(), Some("python"));
    mcp::remove_server(&context, "test-service").unwrap();
    let text = fs::read_to_string(&paths.config_path).unwrap();
    assert!(text.contains("# User settings"));
    let parsed: toml::Value = text.parse().unwrap();
    assert_eq!(parsed["model"].as_str(), Some("test-model"));
    assert_eq!(
        parsed["projects"]["D:\\project"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert!(parsed.get("mcp_servers").is_none_or(|servers| servers.as_table().unwrap().is_empty()));
}

#[test]
fn mcp_edit_preserves_windows_arguments_and_unmanaged_options() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    fs::create_dir_all(&paths.codex_home).unwrap();
    fs::write(
        &paths.config_path,
        r###"
[mcp_servers.example]
command = 'C:\Program Files\nodejs\node.exe'
args = [
  'C:\tools\server.js',
  '--features=a,b',
]
startup_timeout_sec = 45
enabled_tools = ["search"]
[mcp_servers.example.env]
"APP.DATA" = 'C:\data'
"# --- AgentHarbor Managed" = "value"
"# --- End AgentHarbor Managed" = "value"
"[not_a_table]" = "value"
"# comment" = "value"
"###,
    )
    .unwrap();
    let context = mcp_context(temp.path(), &paths);
    let view = mcp::load_library(&context).unwrap();
    let before = view.servers[0].clone();
    assert_eq!(before.spec.args, [r"C:\tools\server.js", "--features=a,b"]);
    assert_eq!(before.spec.env.len(), 5);
    // Saved again unchanged: the agent's own options stay.
    let entry = mcp::McpServerEntry {
        name: before.name.clone(),
        spec: before.spec.clone(),
        note: String::new(),
        apps: vec![mcp::McpApp::Codex],
    };
    let after = mcp::save_server(&context, Some("example"), entry).unwrap();
    assert!(after.failures.is_empty(), "{:?}", after.failures);
    assert_eq!(after.servers[0].spec, before.spec);
    let parsed: toml::Value = fs::read_to_string(&paths.config_path)
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        parsed["mcp_servers"]["example"]["startup_timeout_sec"].as_integer(),
        Some(45)
    );
    assert_eq!(
        parsed["mcp_servers"]["example"]["enabled_tools"][0].as_str(),
        Some("search")
    );
}

#[test]
fn mcp_http_headers_roundtrip_and_invalid_config_is_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    fs::create_dir_all(&paths.codex_home).unwrap();
    fs::write(&paths.config_path, "[mcp_servers.remote]
url = 'https://example.com/mcp'
[mcp_servers.remote.http_headers]
'X.Test' = 'fixture'
").unwrap();
    let context = mcp_context(temp.path(), &paths);
    let view = mcp::load_library(&context).unwrap();
    let mut spec = view.servers[0].spec.clone();
    assert_eq!(spec.headers.get("X.Test").map(String::as_str), Some("fixture"));
    spec.headers.insert("X.Test".into(), "updated".into());
    let entry = |spec: mcp::McpSpec| mcp::McpServerEntry {
        name: "remote".into(),
        spec,
        note: String::new(),
        apps: vec![mcp::McpApp::Codex],
    };
    let view = mcp::save_server(&context, Some("remote"), entry(spec.clone())).unwrap();
    assert!(view.failures.is_empty(), "{:?}", view.failures);
    let servers = mcp::read_codex_servers(&paths).unwrap();
    assert_eq!(servers[0].1["http_headers"]["X.Test"].as_str(), Some("updated"));
    let broken = "[mcp_servers.remote
url = 'invalid TOML'
";
    fs::write(&paths.config_path, broken).unwrap();
    let view = mcp::save_server(&context, Some("remote"), entry(spec)).unwrap();
    assert_eq!(view.failures.len(), 1);
    assert_eq!(fs::read_to_string(&paths.config_path).unwrap(), broken);
}
