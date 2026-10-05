use agentharbor_lib::core::models::{McpServerSummary, McpTransport};
use agentharbor_lib::core::{custom_instructions, mcp, skills};

#[test]
fn skill_disable_enable_preserves_nested_and_hidden_content() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let dir = paths.skills_dir.join("group/nested");
    fs::create_dir_all(dir.join(".private")).unwrap();
    fs::write(dir.join("SKILL.md"), "# Nested\nDetails").unwrap();
    fs::write(dir.join(".private/data"), [0, 1, 255]).unwrap();
    let id = skills::load_installed_skills(&paths.skills_dir).unwrap()[0]
        .id
        .clone();
    let disabled =
        skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, &id, false).unwrap();
    assert!(!disabled.enabled);
    assert!(!dir.exists());
    assert!(skills::load_installed_skills(&paths.skills_dir)
        .unwrap()
        .is_empty());
    assert_eq!(
        skills::load_skill_catalog(&paths.skills_dir, &paths.app_data_dir)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fs::read(std::path::Path::new(&disabled.directory_path).join(".private/data")).unwrap(),
        [0, 1, 255]
    );
    assert!(
        !skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, &id, false)
            .unwrap()
            .enabled
    );
    assert!(
        skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, &id, true)
            .unwrap()
            .enabled
    );
    assert_eq!(fs::read(dir.join(".private/data")).unwrap(), [0, 1, 255]);
    assert!(skills::load_skill_backups(&paths.skill_backups_dir)
        .unwrap()
        .is_empty());
}

#[test]
fn disabled_skill_reimport_restore_and_uninstall_keep_activation_consistent() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    let source = temp.path().join("sample");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), "# Original").unwrap();
    let first = skills::import_skill(
        &paths.skills_dir,
        &paths.app_data_dir,
        source.to_str().unwrap(),
    )
    .unwrap();
    skills::set_skill_enabled(
        &paths.skills_dir,
        &paths.app_data_dir,
        &first.skill.id,
        false,
    )
    .unwrap();
    fs::write(source.join("SKILL.md"), "# Updated").unwrap();
    let updated = skills::import_skill(
        &paths.skills_dir,
        &paths.app_data_dir,
        source.to_str().unwrap(),
    )
    .unwrap();
    assert!(!updated.skill.enabled && updated.replaced_existing);
    assert!(!paths.skills_dir.join("sample").exists());
    let restored = skills::restore_skill_backup(
        &paths.skills_dir,
        &paths.app_data_dir,
        &updated.backup.unwrap().id,
    )
    .unwrap();
    assert!(!restored.restored_skill.enabled);
    assert_eq!(
        fs::read_to_string(&restored.restored_skill.skill_file_path).unwrap(),
        "# Original"
    );
    let removed =
        skills::remove_skill(&paths.skills_dir, &paths.app_data_dir, &first.skill.id).unwrap();
    assert_eq!(removed.remaining_installed_count, 0);
    assert!(
        skills::load_skill_catalog(&paths.skills_dir, &paths.app_data_dir)
            .unwrap()
            .is_empty()
    );
    assert!(
        skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, &removed.backup.id)
            .unwrap()
            .restored_skill
            .enabled
    );
}

#[test]
fn skill_activation_conflicts_preserve_both_copies() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let active = paths.skills_dir.join("sample");
    fs::create_dir_all(&active).unwrap();
    fs::write(active.join("SKILL.md"), "# Managed").unwrap();
    let inactive =
        skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, "sample", false).unwrap();
    fs::create_dir_all(&active).unwrap();
    fs::write(active.join("SKILL.md"), "# External").unwrap();
    let listed = skills::load_skill_catalog(&paths.skills_dir, &paths.app_data_dir).unwrap();
    assert!(listed[0].enabled && listed[0].conflict);
    assert!(
        skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, "sample", true).is_err()
    );
    assert!(skills::remove_skill(&paths.skills_dir, &paths.app_data_dir, "sample").is_err());
    assert_eq!(
        fs::read_to_string(active.join("SKILL.md")).unwrap(),
        "# External"
    );
    assert_eq!(
        fs::read_to_string(inactive.skill_file_path).unwrap(),
        "# Managed"
    );
    fs::remove_file(active.join("SKILL.md")).unwrap();
    let listed = skills::load_skill_catalog(&paths.skills_dir, &paths.app_data_dir).unwrap();
    assert!(
        !listed[0].enabled && listed[0].conflict,
        "even a non-skill destination must block overwrite"
    );
}

#[test]
fn skill_activation_rejects_escape_overlap_and_hidden_nested_destination() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    assert!(
        skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, "../outside", false)
            .is_err()
    );
    assert!(skills::set_skill_enabled(
        &paths.skills_dir,
        &paths.skills_dir.join("storage"),
        "sample",
        false
    )
    .is_err());
    let child = paths.skills_dir.join("group/nested");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("SKILL.md"), "# Nested").unwrap();
    let id = skills::load_installed_skills(&paths.skills_dir).unwrap()[0]
        .id
        .clone();
    let inactive =
        skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, &id, false).unwrap();
    fs::write(paths.skills_dir.join("group/SKILL.md"), "# Parent").unwrap();
    assert!(skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, &id, true).is_err());
    assert!(std::path::Path::new(&inactive.skill_file_path).exists());
}

#[test]
fn concurrent_skill_toggles_are_serialized() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::create_dir_all(paths.skills_dir.join("sample")).unwrap();
    fs::write(paths.skills_dir.join("sample/SKILL.md"), "# Sample").unwrap();
    std::thread::scope(|scope| {
        for _ in 0..6 {
            scope.spawn(|| {
                for enabled in [false, true, false] {
                    skills::set_skill_enabled(
                        &paths.skills_dir,
                        &paths.app_data_dir,
                        "sample",
                        enabled,
                    )
                    .unwrap();
                }
            });
        }
    });
    let listed = skills::load_skill_catalog(&paths.skills_dir, &paths.app_data_dir).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].conflict);
    assert_eq!(
        fs::read_to_string(&listed[0].skill_file_path).unwrap(),
        "# Sample"
    );
}

#[cfg(windows)]
#[test]
fn junctions_are_not_followed_by_skill_mutations() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    let external = temp.path().join("external");
    let skill = paths.skills_dir.join("sample");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("SKILL.md"), "# External").unwrap();
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "# Sample").unwrap();
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:SKILL_TEST_LINK -Target $env:SKILL_TEST_TARGET -ErrorAction Stop | Out-Null"])
        .env("SKILL_TEST_LINK", skill.join("references"))
        .env("SKILL_TEST_TARGET", &external).status().unwrap();
    assert!(status.success());
    assert!(
        skills::set_skill_enabled(&paths.skills_dir, &paths.app_data_dir, "sample", false).is_err()
    );
    assert!(skill.join("SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(external.join("SKILL.md")).unwrap(),
        "# External"
    );
    assert!(skills::import_skill(
        &paths.skills_dir,
        &paths.app_data_dir,
        skill.to_str().unwrap()
    )
    .is_ok());
    // A linked ancestor must also be rejected, even when it points inside the root.
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:SKILL_TEST_LINK -Target $env:SKILL_TEST_TARGET -ErrorAction Stop | Out-Null"])
        .env("SKILL_TEST_LINK", paths.skills_dir.join("alias"))
        .env("SKILL_TEST_TARGET", &skill).status().unwrap();
    assert!(status.success());
    assert!(skills::set_skill_enabled(
        &paths.skills_dir,
        &paths.app_data_dir,
        "alias/references",
        false
    )
    .is_err());
    assert!(external.join("SKILL.md").exists());
}
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
    let imported = skills::import_skill(
        &paths.skills_dir,
        &paths.app_data_dir,
        source.to_str().unwrap(),
    )
    .unwrap();
    assert!(!imported.replaced_existing);
    fs::write(source.join("SKILL.md"), "# Example\nSecond version").unwrap();
    let replaced = skills::import_skill(
        &paths.skills_dir,
        &paths.app_data_dir,
        source.to_str().unwrap(),
    )
    .unwrap();
    assert!(replaced.replaced_existing);
    assert!(replaced.backup.is_some());
    let removed =
        skills::remove_skill(&paths.skills_dir, &paths.app_data_dir, &imported.skill.id).unwrap();
    assert_eq!(removed.remaining_installed_count, 0);
    skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, &removed.backup.id)
        .unwrap();
    assert_eq!(
        fs::read_to_string(paths.skills_dir.join("example-skill/SKILL.md")).unwrap(),
        "# Example\nSecond version"
    );
    assert_eq!(
        fs::read_to_string(paths.skills_dir.join("example-skill/references/guide.md")).unwrap(),
        "reference"
    );
    skills::delete_skill_backup(&paths.app_data_dir, &removed.backup.id).unwrap();
    assert_eq!(
        skills::load_skill_backups(&paths.skill_backups_dir)
            .unwrap()
            .len(),
        1
    );
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
    let removed =
        skills::remove_skill(&paths.skills_dir, &paths.app_data_dir, &installed[0].id).unwrap();
    assert!(!removed.backup.id.contains(['/', '\\']));
    assert_eq!(
        skills::load_skill_backups(&paths.skill_backups_dir)
            .unwrap()
            .len(),
        1
    );
    skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, &removed.backup.id)
        .unwrap();
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
    assert!(
        skills::restore_skill_backup(&paths.skills_dir, &paths.app_data_dir, "../keep").is_err()
    );
    assert!(unrelated.join("important.txt").exists());
}

#[test]
fn instruction_rollback_keeps_later_user_edits() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.global_agents_path, "User rules\n").unwrap();
    let state = custom_instructions::apply_managed_content(
        &paths,
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
    let state =
        custom_instructions::apply_managed_content(&paths, "Managed", None, None, None).unwrap();
    let broken = "User rules\n<!-- AGENT_HARBOR_CUSTOM_INSTRUCTIONS_START -->\nbroken";
    fs::write(&paths.global_agents_path, broken).unwrap();
    assert!(custom_instructions::preview_apply(&paths, "new").is_err());
    assert!(custom_instructions::rollback_history(&paths, &state.history[0].id).is_err());
    assert_eq!(
        fs::read_to_string(&paths.global_agents_path).unwrap(),
        broken
    );
}

#[test]
fn instruction_history_rejects_paths_outside_its_directory() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    paths.ensure_directories().unwrap();
    fs::write(&paths.global_agents_path, "Keep user rules").unwrap();
    assert!(custom_instructions::rollback_history(&paths, "../outside").is_err());
    assert!(custom_instructions::rollback_history(&paths, r"..\outside").is_err());
    assert_eq!(
        fs::read_to_string(&paths.global_agents_path).unwrap(),
        "Keep user rules"
    );
}

#[test]
fn mcp_crud_keeps_other_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(temp.path().join("codex"));
    fs::create_dir_all(&paths.codex_home).unwrap();
    fs::write(&paths.config_path, "# User settings\nmodel = \"test-model\"\n\n[projects.\"D:\\\\project\"]\ntrust_level = \"trusted\"\n").unwrap();
    let mut server = McpServerSummary {
        name: "test-service".into(),
        transport: McpTransport::Stdio,
        enabled: true,
        source_path: paths.config_path.display().to_string(),
        command: Some("node".into()),
        args: vec!["server.js".into()],
        url: None,
        headers: HashMap::new(),
        environment: HashMap::new(),
    };
    mcp::upsert_mcp_server(&paths, &server).unwrap();
    assert_eq!(mcp::load_mcp_servers(&paths).unwrap().len(), 1);
    mcp::set_mcp_server_enabled(&paths, "test-service", false).unwrap();
    assert!(!mcp::load_mcp_servers(&paths).unwrap()[0].enabled);
    server.command = Some("python".into());
    mcp::upsert_mcp_server(&paths, &server).unwrap();
    mcp::remove_mcp_server(&paths, "test-service").unwrap();
    let text = fs::read_to_string(paths.config_path).unwrap();
    assert!(text.contains("# User settings"));
    let parsed: toml::Value = text.parse().unwrap();
    assert_eq!(parsed["model"].as_str(), Some("test-model"));
    assert_eq!(
        parsed["projects"]["D:\\project"]["trust_level"].as_str(),
        Some("trusted")
    );
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
    let before = mcp::load_mcp_servers(&paths).unwrap();
    assert_eq!(before[0].args, [r"C:\tools\server.js", "--features=a,b"]);
    mcp::set_mcp_server_enabled(&paths, "example", false).unwrap();
    let after = mcp::load_mcp_servers(&paths).unwrap();
    assert!(!after[0].enabled);
    assert_eq!(after[0].command, before[0].command);
    assert_eq!(after[0].args, before[0].args);
    assert_eq!(after[0].environment, before[0].environment);
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
    fs::write(&paths.config_path, "[mcp_servers.remote]\nurl = 'https://example.com/mcp'\n[mcp_servers.remote.http_headers]\n'X.Test' = 'fixture'\n").unwrap();
    let mut server = mcp::load_mcp_servers(&paths).unwrap().remove(0);
    assert_eq!(
        server.headers.get("X.Test").map(String::as_str),
        Some("fixture")
    );
    server.headers.insert("X.Test".into(), "updated".into());
    mcp::upsert_mcp_server(&paths, &server).unwrap();
    assert_eq!(
        mcp::load_mcp_servers(&paths).unwrap()[0].headers,
        server.headers
    );
    let broken = "[mcp_servers.remote\nurl = 'invalid TOML'\n";
    fs::write(&paths.config_path, broken).unwrap();
    assert!(mcp::upsert_mcp_server(&paths, &server).is_err());
    assert_eq!(fs::read_to_string(&paths.config_path).unwrap(), broken);
}
