use super::*;

fn fixture(mode: ClientAccessMode) -> (tempfile::TempDir, ConfigManager, Store, ConfigRequest) {
    let dir = tempfile::tempdir().unwrap();
    let manager = ConfigManager::new(&dir.path().join("app"), ClientPaths::from_home(dir.path()));
    let store = Store {
        providers: vec![Provider {
            id: "relay".into(),
            name: "Relay".into(),
            model: "claude-test".into(),
            base_url: "https://relay.example/v1".into(),
            api_key: "upstream-secret".into(),
            protocol: ModelProtocol::AnthropicMessages,
            enabled: true,
            ..Default::default()
        }],
        router: crate::core::router::RouterSettings {
            bearer_token: "local-secret".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let request = ConfigRequest {
        target_id: TargetId::ClaudeCode,
        provider_id: "relay".into(),
        model: "claude-test".into(),
        context_window: None,
        max_output_tokens: None,
        options: ClientOptions {
            access_mode: mode,
            ..Default::default()
        },
    };
    (dir, manager, store, request)
}

fn write(path: &Path, value: Value) {
    storage::write_atomic(
        path,
        serde_json::to_string_pretty(&value).unwrap().as_bytes(),
    )
    .unwrap();
}

fn read(path: &Path) -> Value {
    parse(read_text(path).unwrap().as_deref(), TargetId::ClaudeCode).unwrap()
}

fn inject(manager: &ConfigManager, store: &Store, request: &ConfigRequest) {
    let preview = manager.preview(store, request).unwrap();
    manager.apply(store, request, &preview.revision).unwrap();
}

#[test]
fn official_switch_does_not_restore_a_third_party_baseline_or_touch_login() {
    for mode in [ClientAccessMode::Native, ClientAccessMode::Router] {
        let (dir, manager, store, request) = fixture(mode);
        let user_row = json!({"model":"user-model","label":"User row","description":"keep"});
        write(
            &manager.paths.claude,
            json!({
                "env":{"ANTHROPIC_BASE_URL":"https://old.example","ANTHROPIC_API_KEY":"old-secret","CUSTOM":"keep"},
                "permissions":{"allow":["Read"]},"hooks":{"keep":true},
                "modelPicker":{"options":[user_row.clone()]},"availableModels":["user-model"]
            }),
        );
        let credentials = dir.path().join(".claude/.credentials.json");
        let account = dir.path().join(".claude.json");
        storage::write_atomic(&credentials, b"saved official login").unwrap();
        storage::write_atomic(&account, b"account preferences").unwrap();
        inject(&manager, &store, &request);
        let mut installed = read(&manager.paths.claude);
        let added = json!({"model":"external-model","label":"Added later"});
        installed["modelPicker"]["options"]
            .as_array_mut()
            .unwrap()
            .push(added.clone());
        installed["permissions"]["allow"]
            .as_array_mut()
            .unwrap()
            .push(json!("Write"));
        installed["env"]["ANTHROPIC_AUTH_TOKEN"] = json!("changed-by-another-switcher");
        write(&manager.paths.claude, installed.clone());
        assert!(manager.list()[0].conflict);
        let preview = manager.preview_claude_official().unwrap();
        let serialized = serde_json::to_string(&preview).unwrap();
        assert!(preview.has_changes);
        for secret in [
            "old-secret",
            "upstream-secret",
            "local-secret",
            "changed-by-another-switcher",
        ] {
            assert!(!serialized.contains(secret));
        }
        let applied = manager.switch_claude_official(&preview.revision).unwrap();
        let backup = secret_file::read(Path::new(&applied.backup_path))
            .unwrap()
            .unwrap();
        let original: Option<String> = serde_json::from_slice(&backup).unwrap();
        assert_eq!(
            parse(original.as_deref(), TargetId::ClaudeCode).unwrap(),
            installed
        );
        let official = read(&manager.paths.claude);
        assert_eq!(official["env"], json!({"CUSTOM":"keep"}));
        assert_eq!(official["permissions"]["allow"], json!(["Read", "Write"]));
        assert_eq!(official["hooks"], json!({"keep":true}));
        assert_eq!(official["modelPicker"]["options"], json!([user_row, added]));
        assert!(official.get("model").is_none());
        assert!(official.get("availableModels").is_none());
        assert!(official["modelPicker"]
            .get("replaceBuiltInOptions")
            .is_none());
        assert_eq!(std::fs::read(credentials).unwrap(), b"saved official login");
        assert_eq!(std::fs::read(account).unwrap(), b"account preferences");
        assert!(manager
            .stored_request(TargetId::ClaudeCode)
            .unwrap()
            .is_none());
        assert!(!has_router_configs(&manager.app_data_dir).unwrap());
        assert!(manager.restore_router_configs_on_exit().is_empty());
        assert_eq!(read(&manager.paths.claude), official);
    }
}

#[test]
fn official_switch_drops_the_model_picker_it_empties() {
    for mode in [ClientAccessMode::Native, ClientAccessMode::Router] {
        let (_dir, manager, store, request) = fixture(mode);
        write(
            &manager.paths.claude,
            json!({"permissions":{"allow":["Read"]}}),
        );
        inject(&manager, &store, &request);
        let preview = manager.preview_claude_official().unwrap();
        assert!(preview.removed_fields.contains(&"modelPicker".into()));
        manager.switch_claude_official(&preview.revision).unwrap();
        assert_eq!(
            read(&manager.paths.claude),
            json!({"permissions":{"allow":["Read"]}}),
            "neither an emptied modelPicker nor an emptied env is left behind"
        );
    }
    // An env the user kept empty is theirs.
    let (_dir, manager, _store, _request) = fixture(ClientAccessMode::Native);
    write(&manager.paths.claude, json!({"env":{}}));
    assert!(!manager.preview_claude_official().unwrap().has_changes);
    // An earlier build left `"modelPicker": {}` behind; a user's own keys stay.
    for (original, expected) in [
        (json!({"modelPicker":{}}), json!({})),
        (
            json!({"modelPicker":{"custom":"keep"}}),
            json!({"modelPicker":{"custom":"keep"}}),
        ),
    ] {
        let (_dir, manager, _store, _request) = fixture(ClientAccessMode::Native);
        write(&manager.paths.claude, original);
        let preview = manager.preview_claude_official().unwrap();
        manager.switch_claude_official(&preview.revision).unwrap();
        assert_eq!(read(&manager.paths.claude), expected);
    }
}

#[test]
fn official_switch_cleans_unmanaged_cloud_and_gateway_fields_but_keeps_policy() {
    let (_dir, manager, _store, _request) = fixture(ClientAccessMode::Native);
    let mut env = serde_json::Map::new();
    for field in ENV_FIELDS {
        env.insert((*field).into(), json!("legacy"));
    }
    env.insert("AWS_ACCESS_KEY_ID".into(), json!("shared-with-other-tools"));
    env.insert("HTTP_PROXY".into(), json!("http://127.0.0.1:10808"));
    write(
        &manager.paths.claude,
        json!({"env":env,"apiKeyHelper":"some-script","model":"relay-model",
        "forceLoginMethod":"claudeai","forceLoginOrgUUID":"organization-policy","permissions":{}}),
    );
    let preview = manager.preview_claude_official().unwrap();
    assert!(preview.removed_fields.contains(&"apiKeyHelper".into()));
    manager.switch_claude_official(&preview.revision).unwrap();
    let value = read(&manager.paths.claude);
    assert_eq!(
        value["env"],
        json!({"AWS_ACCESS_KEY_ID":"shared-with-other-tools","HTTP_PROXY":"http://127.0.0.1:10808"})
    );
    assert_eq!(value["forceLoginMethod"], "claudeai");
    assert_eq!(value["forceLoginOrgUUID"], "organization-policy");
    assert!(value.get("apiKeyHelper").is_none());
}

#[test]
fn already_official_is_byte_preserving_and_does_not_create_files_or_backups() {
    let (_dir, manager, _store, _request) = fixture(ClientAccessMode::Router);
    let absent = manager.preview_claude_official().unwrap();
    assert!(!absent.has_changes);
    assert!(manager
        .switch_claude_official(&absent.revision)
        .unwrap()
        .backup_path
        .is_empty());
    assert!(!manager.paths.claude.exists());
    assert!(!manager.data_dir.exists());
    let original =
        "\u{feff}{ // user comment\n \"permissions\":{},\"env\":{\"CUSTOM\":\"keep\"},\n}";
    storage::write_atomic(&manager.paths.claude, original.as_bytes()).unwrap();
    let preview = manager.preview_claude_official().unwrap();
    assert!(!preview.has_changes);
    manager.switch_claude_official(&preview.revision).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.claude).unwrap(),
        original
    );
    assert!(!manager.data_dir.exists());
}

#[test]
fn official_switch_checks_file_and_management_revisions() {
    let (_dir, manager, store, request) = fixture(ClientAccessMode::Router);
    inject(&manager, &store, &request);
    let preview = manager.preview_claude_official().unwrap();
    let mut external = read(&manager.paths.claude);
    external["hooks"] = json!({"updated":true});
    write(&manager.paths.claude, external.clone());
    assert_eq!(
        manager
            .switch_claude_official(&preview.revision)
            .err()
            .unwrap(),
        "clients.errors.changed"
    );
    assert_eq!(read(&manager.paths.claude), external);
    assert!(manager
        .stored_request(TargetId::ClaudeCode)
        .unwrap()
        .is_some());
    let preview = manager.preview_claude_official().unwrap();
    manager.release(TargetId::ClaudeCode).unwrap();
    assert_eq!(
        manager
            .switch_claude_official(&preview.revision)
            .err()
            .unwrap(),
        "clients.errors.changed"
    );
    let preview = manager.preview_claude_official().unwrap();
    secret_file::write(&owned::path(&manager.data_dir, TargetId::ClaudeCode), b"{}").unwrap();
    assert_eq!(
        manager
            .switch_claude_official(&preview.revision)
            .err()
            .unwrap(),
        "clients.errors.changed"
    );
}

#[test]
fn official_switch_can_release_an_already_cleaned_session_without_rewriting() {
    let (_dir, manager, store, request) = fixture(ClientAccessMode::Router);
    inject(&manager, &store, &request);
    let original = "{ \"permissions\": {\"allow\":[\"Read\"]} }\n";
    storage::write_atomic(&manager.paths.claude, original.as_bytes()).unwrap();
    let modified = std::fs::metadata(&manager.paths.claude)
        .unwrap()
        .modified()
        .unwrap();
    let preview = manager.preview_claude_official().unwrap();
    assert!(preview.has_changes);
    assert!(preview.removed_fields.is_empty());
    let applied = manager.switch_claude_official(&preview.revision).unwrap();
    assert!(applied.backup_path.is_empty());
    assert_eq!(
        std::fs::read_to_string(&manager.paths.claude).unwrap(),
        original
    );
    assert_eq!(
        std::fs::metadata(&manager.paths.claude)
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
    assert!(!manager.list()[0].configured);
}

#[test]
fn official_switch_then_reinjection_restores_official_configuration() {
    let (_dir, manager, store, request) = fixture(ClientAccessMode::Router);
    inject(&manager, &store, &request);
    manager.release(TargetId::ClaudeCode).unwrap();
    let preview = manager.preview_claude_official().unwrap();
    manager.switch_claude_official(&preview.revision).unwrap();
    let official = std::fs::read_to_string(&manager.paths.claude).unwrap();
    inject(&manager, &store, &request);
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.claude).unwrap(),
        official
    );
}

#[test]
fn official_switch_refuses_invalid_documents_without_destroying_them() {
    let (_dir, manager, _store, _request) = fixture(ClientAccessMode::Router);
    for original in [
        "{broken",
        "[]",
        r#"{"env":"bad"}"#,
        r#"{"modelPicker":{"options":{}}}"#,
    ] {
        storage::write_atomic(&manager.paths.claude, original.as_bytes()).unwrap();
        assert!(
            matches!(manager.preview_claude_official(), Err(error) if error == "clients.errors.invalidConfig")
        );
        assert_eq!(
            std::fs::read_to_string(&manager.paths.claude).unwrap(),
            original
        );
        assert!(!manager.data_dir.exists());
    }
}

#[test]
fn managed_warning_identifies_active_overrides_without_false_cloud_warnings() {
    assert!(!managed_overrides_official(
        &json!({"apiKeyHelper":"","env":{
            "CLAUDE_CODE_USE_BEDROCK":"0", "CLAUDE_CODE_USE_VERTEX":false,
            "CLAUDE_CODE_SKIP_BEDROCK_AUTH":"1", "ANTHROPIC_BEDROCK_BASE_URL":"https://inactive.example"
        }})
    ));
    assert!(managed_overrides_official(
        &json!({"env":{"CLAUDE_CODE_USE_ANTHROPIC_AWS":"1"}})
    ));
    assert!(managed_overrides_official(
        &json!({"env":{"ANTHROPIC_AUTH_TOKEN":"managed-secret"}})
    ));
    assert!(managed_overrides_official(
        &json!({"apiKeyHelper":"managed-helper"})
    ));
}

#[test]
fn official_switch_releases_only_claude_and_keeps_other_router_consumers() {
    let (_dir, manager, mut store, mut request) = fixture(ClientAccessMode::Router);
    store.providers[0].context_window = Some(128_000);
    request.target_id = TargetId::OpenCode;
    inject(&manager, &store, &request);
    let opencode = std::fs::read(&manager.paths.opencode).unwrap();
    request.target_id = TargetId::ClaudeCode;
    inject(&manager, &store, &request);
    let preview = manager.preview_claude_official().unwrap();
    manager.switch_claude_official(&preview.revision).unwrap();
    assert_eq!(std::fs::read(&manager.paths.opencode).unwrap(), opencode);
    assert!(manager
        .stored_request(TargetId::OpenCode)
        .unwrap()
        .is_some());
    assert!(has_router_configs(&manager.app_data_dir).unwrap());
}
