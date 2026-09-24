use super::*;

fn fixture() -> (tempfile::TempDir, ConfigManager, Store, ConfigRequest) {
    let temp = tempfile::tempdir().unwrap();
    let manager = ConfigManager::new(
        &temp.path().join("data"),
        ClientPaths::from_home(temp.path()),
    );
    let store = Store {
        providers: vec![Provider {
            id: "test-relay".into(),
            name: "Relay".into(),
            enabled: true,
            model: "claude-test".into(),
            api_key: "upstream-secret".into(),
            protocol: ModelProtocol::AnthropicMessages,
            ..Default::default()
        }],
        router: super::super::router::RouterSettings {
            bearer_token: "local-secret".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let request = ConfigRequest {
        target_id: TargetId::ClaudeCode,
        provider_id: "test-relay".into(),
        model: "claude-test".into(),
        context_window: 200000,
        max_output_tokens: 8192,
    };
    (temp, manager, store, request)
}

fn apply(manager: &ConfigManager, store: &Store, request: &ConfigRequest) {
    let preview = manager.preview(store, request).unwrap();
    manager.apply(store, request, &preview.revision).unwrap();
}

fn write(path: &Path, value: &Value) {
    storage::write_atomic(path, serde_json::to_string(value).unwrap().as_bytes()).unwrap();
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn claude_preserves_settings_and_restores_exact_original() {
    let (_temp, manager, store, request) = fixture();
    let original = "{\n  \"permissions\": {\"allow\": [\"Read\"]}, \"env\": {\"ANTHROPIC_API_KEY\": \"existing-secret\", \"CUSTOM\": \"kept\"}\n}";
    storage::write_atomic(&manager.paths.claude, original.as_bytes()).unwrap();
    let preview = manager.preview(&store, &request).unwrap();
    assert!(!preview.content.contains("local-secret"));
    assert!(!preview.content.contains("upstream-secret"));
    assert!(!preview.content.contains("existing-secret"));
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["env"]["CUSTOM"], "kept");
    assert_eq!(installed["permissions"]["allow"][0], "Read");
    assert_eq!(
        installed["env"]["ANTHROPIC_BASE_URL"],
        "http://127.0.0.1:43127/p/test-relay"
    );
    assert_eq!(installed["env"]["ANTHROPIC_AUTH_TOKEN"], "local-secret");
    assert!(!installed.to_string().contains("upstream-secret"));
    manager.restore(request.target_id).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.claude).unwrap(),
        original
    );
}

#[test]
fn restore_and_reapply_keep_independent_edits() {
    let (_temp, manager, store, mut request) = fixture();
    write(&manager.paths.claude, &json!({"env": {"CUSTOM": "before"}}));
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    current["env"]["CUSTOM"] = json!("after");
    current["permissions"] = json!({"allow": ["Read"]});
    write(&manager.paths.claude, &current);
    request.model = "second-model".into();
    apply(&manager, &store, &request);
    assert_eq!(read(&manager.paths.claude)["model"], "second-model");
    manager.restore(request.target_id).unwrap();
    let restored = read(&manager.paths.claude);
    assert_eq!(restored["env"]["CUSTOM"], "after");
    assert_eq!(restored["permissions"]["allow"][0], "Read");
    assert!(restored["env"].get("ANTHROPIC_BASE_URL").is_none());
}

#[test]
fn independent_edits_survive_restore_without_reapply() {
    let (_temp, manager, store, request) = fixture();
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    current["theme"] = json!("dark");
    write(&manager.paths.claude, &current);
    manager.restore(request.target_id).unwrap();
    assert_eq!(read(&manager.paths.claude)["theme"], "dark");
}

#[test]
fn claude_allowlist_includes_selected_model_and_restores() {
    let (_temp, manager, store, request) = fixture();
    let original = json!({"availableModels": ["sonnet"], "enforceAvailableModels": true});
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    assert_eq!(
        read(&manager.paths.claude)["availableModels"],
        json!(["sonnet", "claude-test"])
    );
    manager.restore(request.target_id).unwrap();
    assert_eq!(read(&manager.paths.claude), original);
}

#[test]
fn changed_owned_fields_are_not_overwritten_or_restored() {
    let (_temp, manager, store, request) = fixture();
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    current["env"]["ANTHROPIC_BASE_URL"] = json!("https://user-changed.example");
    write(&manager.paths.claude, &current);
    assert!(manager.preview(&store, &request).is_err());
    assert!(manager.restore(request.target_id).is_err());
    assert!(manager.list()[0].conflict);
    assert_eq!(read(&manager.paths.claude), current);
}

#[test]
fn preview_revision_rejects_config_or_provider_changes() {
    let (_temp, manager, mut store, request) = fixture();
    let preview = manager.preview(&store, &request).unwrap();
    write(&manager.paths.claude, &json!({"theme": "dark"}));
    assert!(manager.apply(&store, &request, &preview.revision).is_err());
    let preview = manager.preview(&store, &request).unwrap();
    store.router.port += 1;
    assert!(manager.apply(&store, &request, &preview.revision).is_err());
    assert!(!manager.list()[0].configured);
}

#[test]
fn rejects_wrong_protocol_disabled_provider_and_invalid_settings() {
    let (_temp, manager, mut store, request) = fixture();
    store.providers[0].protocol = ModelProtocol::ChatCompletions;
    assert!(manager.preview(&store, &request).is_err());
    store.providers[0].protocol = ModelProtocol::AnthropicMessages;
    store.providers[0].enabled = false;
    assert!(manager.preview(&store, &request).is_err());
    store.providers[0].enabled = true;
    write(&manager.paths.claude, &json!({"env": "not an object"}));
    assert!(manager.preview(&store, &request).is_err());
    assert_eq!(read(&manager.paths.claude)["env"], "not an object");
}

#[test]
fn opencode_registers_each_protocol_without_removing_official_providers() {
    let (_temp, manager, mut store, mut request) = fixture();
    for (id, protocol) in [
        ("chat", ModelProtocol::ChatCompletions),
        ("responses", ModelProtocol::Responses),
    ] {
        store.providers.push(Provider {
            id: id.into(),
            protocol,
            ..store.providers[0].clone()
        });
    }
    request.target_id = TargetId::OpenCode;
    write(
        &manager.paths.opencode,
        &json!({
            "provider": {"openai": {"name": "Official"}},
            "enabled_providers": ["openai"], "disabled_providers": ["agentharbor_chat", "unrelated"]
        }),
    );
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.opencode);
    assert_eq!(installed["provider"]["openai"]["name"], "Official");
    assert_eq!(
        installed["provider"]["agentharbor_chat"]["npm"],
        "@ai-sdk/openai-compatible"
    );
    assert_eq!(
        installed["provider"]["agentharbor_responses"]["npm"],
        "@ai-sdk/openai"
    );
    assert_eq!(
        installed["provider"]["agentharbor_test-relay"]["npm"],
        "@ai-sdk/anthropic"
    );
    assert_eq!(
        installed["provider"]["agentharbor_test-relay"]["models"]["claude-test"]["limit"]
            ["context"],
        200000
    );
    assert_eq!(installed["disabled_providers"], json!(["unrelated"]));
    assert_eq!(installed["enabled_providers"].as_array().unwrap().len(), 4);
    manager.restore(request.target_id).unwrap();
    assert_eq!(
        read(&manager.paths.opencode)["enabled_providers"],
        json!(["openai"])
    );
}

#[test]
fn jsonc_comments_are_backed_up_and_restored_exactly() {
    let (temp, mut manager, store, mut request) = fixture();
    let path = temp.path().join(".config/opencode/opencode.jsonc");
    let original = "{\n // user comment\n \"theme\": \"dark\",\n}\n";
    storage::write_atomic(&path, original.as_bytes()).unwrap();
    manager.paths = ClientPaths::from_home(temp.path());
    assert_eq!(manager.paths.opencode, path);
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    assert_eq!(read(&path)["theme"], "dark");
    manager.restore(request.target_id).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn clients_have_independent_lifetimes_and_new_files_are_removed_on_restore() {
    let (temp, manager, store, mut request) = fixture();
    apply(&manager, &store, &request);
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert!(!manager.paths.claude.exists());
    assert!(has_active_configs(&temp.path().join("data")).unwrap());
    assert!(manager.list()[1].configured);
    manager.restore(TargetId::OpenCode).unwrap();
    assert!(!has_active_configs(&temp.path().join("data")).unwrap());
    assert!(!manager.paths.opencode.exists());
}
