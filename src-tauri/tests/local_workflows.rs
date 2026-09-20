use agentharbor_lib::core::{custom_instructions, mcp, skills};
use agentharbor_lib::core::models::{McpServerSummary, McpTransport};
use agentharbor_lib::platform::paths::CodexPaths;
use std::{collections::HashMap, fs};

#[test]
fn skill_import_replace_remove_and_restore_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let source = temp.path().join("example-skill");
    fs::create_dir_all(source.join("references")).unwrap();
    fs::write(source.join("SKILL.md"), "# Example\nFirst version").unwrap();
    fs::write(source.join("references/guide.md"), "reference").unwrap();
    let imported = skills::import_skill(&paths.skills_dir, &paths.app_data_dir, source.to_str().unwrap()).unwrap();
    assert!(!imported.replaced_existing);
    fs::write(source.join("SKILL.md"), "# Example\nSecond version").unwrap();
    let replaced = skills::import_skill(&paths.skills_dir, &paths.app_data_dir, source.to_str().unwrap()).unwrap();
    assert!(replaced.replaced_existing);
    assert!(replaced.backup.is_some());
    let removed = skills::remove_skill(&paths.skills_dir, &paths.app_data_dir, &imported.skill.id).unwrap();
    assert_eq!(removed.remaining_installed_count, 0);
    skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, &removed.backup.id).unwrap();
    assert_eq!(fs::read_to_string(paths.skills_dir.join("example-skill/SKILL.md")).unwrap(), "# Example\nSecond version");
    assert_eq!(fs::read_to_string(paths.skills_dir.join("example-skill/references/guide.md")).unwrap(), "reference");
    skills::delete_skill_backup(&paths.app_data_dir, &removed.backup.id).unwrap();
    assert_eq!(skills::load_skill_backups(&paths.skill_backups_dir).unwrap().len(), 1);
}

#[test]
fn nested_skill_backup_is_discoverable_on_windows() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let nested = paths.skills_dir.join("group/nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("SKILL.md"), "# Nested").unwrap();
    let installed = skills::load_installed_skills(&paths.skills_dir).unwrap();
    let removed = skills::remove_skill(&paths.skills_dir, &paths.app_data_dir, &installed[0].id).unwrap();
    assert!(!removed.backup.id.contains(['/', '\\']));
    assert_eq!(skills::load_skill_backups(&paths.skill_backups_dir).unwrap().len(), 1);
    skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, &removed.backup.id).unwrap();
    assert!(nested.join("SKILL.md").exists());
}

#[test]
fn invalid_backup_path_cannot_delete_outside_the_backup_directory() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let unrelated = paths.app_data_dir.join("keep");
    fs::create_dir_all(&unrelated).unwrap();
    fs::write(unrelated.join("important.txt"), "keep me").unwrap();
    assert!(skills::delete_skill_backup(&paths.app_data_dir, "../keep").is_err());
    assert!(skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, "../keep").is_err());
    assert!(unrelated.join("important.txt").exists());
}

#[test]
fn instruction_rollback_keeps_later_user_edits() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.global_agents_path, "User rules\n").unwrap();
    let state = custom_instructions::apply_managed_content(&paths, "Managed first", Some("test".into()), None, None).unwrap();
    let before_first_apply = state.history[0].id.clone();
    let text = fs::read_to_string(&paths.global_agents_path).unwrap();
    fs::write(&paths.global_agents_path, format!("{text}\nLater user rules\n")).unwrap();
    let preview = custom_instructions::preview_apply(&paths, "Managed second").unwrap();
    assert!(preview.resulting_content.contains("Later user rules"));
    custom_instructions::rollback_history(&paths, &before_first_apply).unwrap();
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
    let state = custom_instructions::apply_managed_content(&paths, "Managed", None, None, None).unwrap();
    let broken = "User rules\n<!-- AGENT_HARBOR_CUSTOM_INSTRUCTIONS_START -->\nbroken";
    fs::write(&paths.global_agents_path, broken).unwrap();
    assert!(custom_instructions::preview_apply(&paths, "new").is_err());
    assert!(custom_instructions::rollback_history(&paths, &state.history[0].id).is_err());
    assert_eq!(fs::read_to_string(&paths.global_agents_path).unwrap(), broken);
}

#[test]
fn instruction_history_rejects_paths_outside_its_directory() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.global_agents_path, "Keep user rules").unwrap();
    assert!(custom_instructions::rollback_history(&paths, "../outside").is_err());
    assert!(custom_instructions::rollback_history(&paths, r"..\outside").is_err());
    assert_eq!(fs::read_to_string(&paths.global_agents_path).unwrap(), "Keep user rules");
}

#[test]
fn mcp_crud_keeps_other_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    fs::write(&config, "# User settings\nmodel = \"test-model\"\n\n[projects.\"D:\\\\project\"]\ntrust_level = \"trusted\"\n").unwrap();
    let mut server = McpServerSummary {
        name: "test-service".into(), transport: McpTransport::Stdio, enabled: true,
        source_path: config.display().to_string(), command: Some("node".into()),
        args: vec!["server.js".into()], url: None, headers: HashMap::new(), environment: HashMap::new(),
    };
    mcp::upsert_mcp_server(&config, &server).unwrap();
    assert_eq!(mcp::load_mcp_servers(&config).unwrap().len(), 1);
    mcp::set_mcp_server_enabled(&config, "test-service", false).unwrap();
    assert!(!mcp::load_mcp_servers(&config).unwrap()[0].enabled);
    server.command = Some("python".into());
    mcp::upsert_mcp_server(&config, &server).unwrap();
    mcp::remove_mcp_server(&config, "test-service").unwrap();
    let text = fs::read_to_string(config).unwrap();
    assert!(text.contains("# User settings"));
    let parsed: toml::Value = text.parse().unwrap();
    assert_eq!(parsed["model"].as_str(), Some("test-model"));
    assert_eq!(parsed["projects"]["D:\\project"]["trust_level"].as_str(), Some("trusted"));
}

#[test]
fn mcp_edit_preserves_windows_arguments_and_unmanaged_options() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    fs::write(&config, r###"
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
"###).unwrap();
    let before = mcp::load_mcp_servers(&config).unwrap();
    assert_eq!(before[0].args, [r"C:\tools\server.js", "--features=a,b"]);
    mcp::set_mcp_server_enabled(&config, "example", false).unwrap();
    let after = mcp::load_mcp_servers(&config).unwrap();
    assert!(!after[0].enabled);
    assert_eq!(after[0].command, before[0].command);
    assert_eq!(after[0].args, before[0].args);
    assert_eq!(after[0].environment, before[0].environment);
    let parsed: toml::Value = fs::read_to_string(&config).unwrap().parse().unwrap();
    assert_eq!(parsed["mcp_servers"]["example"]["startup_timeout_sec"].as_integer(), Some(45));
    assert_eq!(parsed["mcp_servers"]["example"]["enabled_tools"][0].as_str(), Some("search"));
}

#[test]
fn mcp_http_headers_roundtrip_and_invalid_config_is_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    fs::write(&config, "[mcp_servers.remote]\nurl = 'https://example.com/mcp'\n[mcp_servers.remote.http_headers]\n'X.Test' = 'fixture'\n").unwrap();
    let mut server = mcp::load_mcp_servers(&config).unwrap().remove(0);
    assert_eq!(server.headers.get("X.Test").map(String::as_str), Some("fixture"));
    server.headers.insert("X.Test".into(), "updated".into());
    mcp::upsert_mcp_server(&config, &server).unwrap();
    assert_eq!(mcp::load_mcp_servers(&config).unwrap()[0].headers, server.headers);
    let broken = "[mcp_servers.remote\nurl = 'invalid TOML'\n";
    fs::write(&config, broken).unwrap();
    assert!(mcp::upsert_mcp_server(&config, &server).is_err());
    assert_eq!(fs::read_to_string(&config).unwrap(), broken);
}
