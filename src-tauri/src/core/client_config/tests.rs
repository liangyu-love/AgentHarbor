use super::*;
#[path = "native_tests.rs"]
mod native_tests;
#[path = "ownership_tests.rs"]
mod ownership_tests;

fn disabled_one_m() -> ClaudeOneM {
    ClaudeOneM {
        startup: false,
        sonnet: false,
        opus: false,
        fable: false,
        haiku: false,
        subagent: false,
    }
}

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
            base_url: "https://relay.example/gateway/v1".into(),
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
        options: ClientOptions {
            access_mode: ClientAccessMode::Router,
            // These legacy-configuration fixtures explicitly opt out of 1M.
            claude_one_m: disabled_one_m(),
            ..Default::default()
        },
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
fn first_claude_apply_reopens_without_a_false_conflict() {
    let (_temp, manager, store, request) = fixture();
    apply(&manager, &store, &request);
    let status = &manager.list()[0];
    assert!(status.configured);
    assert!(!status.conflict);
    assert!(status.error.is_none());
    assert_eq!(status.model.as_deref(), Some("claude-test"));
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
fn exit_restores_local_router_clients_but_keeps_direct_claude_configuration() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    apply(&manager, &store, &request);
    let direct_claude = std::fs::read_to_string(&manager.paths.claude).unwrap();
    request.target_id = TargetId::OpenCode;
    request.options.access_mode = ClientAccessMode::Router;
    apply(&manager, &store, &request);
    assert!(manager.restore_router_configs_on_exit().is_empty());
    assert_eq!(
        std::fs::read_to_string(&manager.paths.claude).unwrap(),
        direct_claude
    );
    assert!(!manager.paths.opencode.exists());
    assert!(manager.list()[0].configured);
    assert!(!manager.list()[1].configured);
}

#[test]
fn exit_does_not_overwrite_an_external_claude_takeover() {
    let (_temp, manager, store, request) = fixture();
    apply(&manager, &store, &request);
    let mut changed = read(&manager.paths.claude);
    changed["env"]["ANTHROPIC_BASE_URL"] = json!("https://other-tool.example");
    write(&manager.paths.claude, &changed);
    let errors = manager.restore_router_configs_on_exit();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, TargetId::ClaudeCode);
    assert_eq!(errors[0].1, "clients.errors.conflict");
    assert_eq!(read(&manager.paths.claude), changed);
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
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    // Use a neutral alias so this test exercises the provider's explicit
    // protocol instead of the Claude-name Anthropic default.
    store.providers[0].model = "custom-model".into();
    request.model = "custom-model".into();
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
        let mut provider = Provider {
            id: id.into(),
            protocol,
            ..store.providers[0].clone()
        };
        provider.model = format!("{id}-default");
        store.providers.push(provider);
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
fn claude_validates_the_requested_models_effective_protocol() {
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    let provider = &mut store.providers[0];
    provider.protocol = ModelProtocol::Responses;
    provider.model = "unavailable-default".into();
    provider.models = vec![
        crate::core::newapi::Model {
            id: "unavailable-default".into(),
            endpoint_types: vec!["anthropic-messages".into()],
            ..Default::default()
        },
        crate::core::newapi::Model {
            id: "claude-test".into(),
            protocol: Some(ModelProtocol::AnthropicMessages),
            endpoint_types: vec!["anthropic-messages".into()],
            ..Default::default()
        },
        crate::core::newapi::Model {
            id: "responses-model".into(),
            ..Default::default()
        },
    ];
    assert!(!provider.is_routable());
    apply(&manager, &store, &request);
    assert_eq!(read(&manager.paths.claude)["model"], "claude-test");
    let installed = std::fs::read(&manager.paths.claude).unwrap();
    request.model = "responses-model".into();
    assert_eq!(
        manager.preview(&store, &request).err().as_deref(),
        Some("clients.errors.protocol")
    );
    assert_eq!(std::fs::read(&manager.paths.claude).unwrap(), installed);
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert!(!manager.paths.claude.exists());
}

#[test]
fn opencode_groups_mixed_models_and_removes_obsolete_protocol_groups_on_reapply() {
    let (_temp, manager, mut store, mut request) = fixture();
    let provider = &mut store.providers[0];
    provider.protocol = ModelProtocol::Responses;
    provider.model = "responses-model".into();
    provider.models = vec![
        crate::core::newapi::Model {
            id: "responses-model".into(),
            ..Default::default()
        },
        crate::core::newapi::Model {
            id: "chat-model".into(),
            protocol: Some(ModelProtocol::ChatCompletions),
            endpoint_types: vec!["openai-chat".into()],
            ..Default::default()
        },
        crate::core::newapi::Model {
            id: "claude-test".into(),
            protocol: Some(ModelProtocol::AnthropicMessages),
            endpoint_types: vec!["anthropic-messages".into()],
            ..Default::default()
        },
    ];
    request.target_id = TargetId::OpenCode;
    request.model = "chat-model".into();
    let original = json!({
        "provider": {"openai": {"name": "Official"}},
        "model": "openai/original",
        "enabled_providers": ["openai"],
        "disabled_providers": ["agentharbor_test-relay_chat", "unrelated"]
    });
    write(&manager.paths.opencode, &original);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.opencode);
    assert_eq!(installed["model"], "agentharbor_test-relay_chat/chat-model");
    for (key, npm, model) in [
        (
            "agentharbor_test-relay",
            "@ai-sdk/openai",
            "responses-model",
        ),
        (
            "agentharbor_test-relay_chat",
            "@ai-sdk/openai-compatible",
            "chat-model",
        ),
        (
            "agentharbor_test-relay_anthropic",
            "@ai-sdk/anthropic",
            "claude-test",
        ),
    ] {
        let group = &installed["provider"][key];
        assert_eq!(group["npm"], npm);
        assert_eq!(group["models"].as_object().unwrap().len(), 1);
        assert!(group["models"].get(model).is_some());
        assert_eq!(
            group["options"]["baseURL"],
            "http://127.0.0.1:43127/p/test-relay/v1"
        );
        assert_eq!(group["options"]["apiKey"], "local-secret");
    }
    assert!(!installed.to_string().contains("upstream-secret"));
    assert_eq!(installed["disabled_providers"], json!(["unrelated"]));
    assert_eq!(installed["enabled_providers"].as_array().unwrap().len(), 4);
    store.providers[0].models[1].protocol = Some(ModelProtocol::AnthropicMessages);
    store.providers[0].models[1].endpoint_types = vec!["anthropic-messages".into()];
    apply(&manager, &store, &request);
    let updated = read(&manager.paths.opencode);
    assert!(updated["provider"]
        .get("agentharbor_test-relay_chat")
        .is_none());
    assert_eq!(
        updated["model"],
        "agentharbor_test-relay_anthropic/chat-model"
    );
    assert_eq!(
        updated["provider"]["agentharbor_test-relay_anthropic"]["models"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        updated["provider"]["openai"],
        original["provider"]["openai"]
    );
    manager.restore(TargetId::OpenCode).unwrap();
    assert_eq!(read(&manager.paths.opencode), original);
}

#[test]
fn clients_have_independent_lifetimes_and_new_files_are_removed_on_restore() {
    let (_temp, manager, store, mut request) = fixture();
    apply(&manager, &store, &request);
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert!(!manager.paths.claude.exists());
    assert!(manager.session(TargetId::OpenCode).unwrap().is_some());
    assert!(manager.session(TargetId::ClaudeCode).unwrap().is_none());
    assert!(manager.list()[1].configured);
    manager.restore(TargetId::OpenCode).unwrap();
    assert!(manager.session(TargetId::ClaudeCode).unwrap().is_none());
    assert!(manager.session(TargetId::OpenCode).unwrap().is_none());
    assert!(!manager.paths.opencode.exists());
}
