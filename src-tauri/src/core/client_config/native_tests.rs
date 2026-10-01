use super::*;

#[test]
fn claude_one_m_defaults_on_and_explicit_off_survives_serialization_and_reopen() {
    let (temp, manager, store, mut request) = fixture();
    request.options = serde_json::from_value(json!({})).unwrap();
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["model"], "claude-test[1m]");
    assert_eq!(
        installed["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "claude-test[1m]"
    );
    let disabled = json!({"startup":false,"sonnet":false,"opus":false,"fable":false,"haiku":false,"subagent":false});
    request.options.claude_one_m = serde_json::from_value(disabled.clone()).unwrap();
    apply(&manager, &store, &request);
    let reopened = ConfigManager::new(
        &temp.path().join("data"),
        ClientPaths::from_home(temp.path()),
    );
    let saved = reopened.list()[0].request.clone().unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap()["claudeOneM"],
        disabled
    );
    apply(&reopened, &store, &saved);
    assert_eq!(read(&manager.paths.claude)["model"], "claude-test");
    assert_eq!(
        read(&manager.paths.claude)["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "claude-test"
    );
}

#[test]
fn legacy_claude_one_m_choices_remain_off_while_missing_haiku_defaults_on() {
    let options: ClientOptions = serde_json::from_value(json!({"claudeOneM": {
        "startup":false,"sonnet":false,"opus":false,"fable":false,"subagent":false
    }}))
    .unwrap();
    assert_eq!(
        serde_json::to_value(options).unwrap()["claudeOneM"],
        json!({
            "startup":false,"sonnet":false,"opus":false,"fable":false,"haiku":true,"subagent":false
        })
    );
}

#[test]
fn claude_legacy_requests_keep_explicit_models_without_a_catalog() {
    let (_temp, manager, store, mut request) = fixture();
    assert!(store.providers[0].models.is_empty());
    request.options.access_mode = ClientAccessMode::Native;
    request.model = "legacy-startup".into();
    request.options.claude_models.opus = "legacy-opus".into();
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    let entries = installed["modelPicker"]["options"].as_array().unwrap();
    for id in ["legacy-startup", "legacy-opus"] {
        assert!(entries.iter().any(|entry| entry["model"] == id));
    }
    assert_eq!(installed["model"], "legacy-startup");
    assert_eq!(
        installed["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "legacy-opus"
    );
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert!(!manager.paths.claude.exists());
}

#[test]
fn claude_one_m_marks_only_enabled_models_and_leaves_context_env_alone() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.claude_model_ids = Some(vec!["claude-test".into()]);
    request.options.claude_models.opus = "claude-test".into();
    request.options.claude_one_m.startup = true;
    request.options.claude_one_m.sonnet = true;
    request.options.claude_one_m.subagent = true;

    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["env"]["ANTHROPIC_MODEL"], "claude-test[1m]");
    assert_eq!(installed["model"], "claude-test[1m]");
    assert_eq!(
        installed["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
        "claude-test[1m]"
    );
    assert_eq!(
        installed["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "claude-test"
    );
    assert_eq!(
        installed["env"]["CLAUDE_CODE_SUBAGENT_MODEL"],
        "claude-test[1m]"
    );
    assert!(installed["env"]
        .get("CLAUDE_CODE_MAX_CONTEXT_TOKENS")
        .is_none());
    assert!(installed["env"]
        .get("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
        .is_none());
    assert!(installed["modelPicker"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["model"] == "claude-test"));

    request.options.claude_one_m = disabled_one_m();
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["env"]["ANTHROPIC_MODEL"], "claude-test");
    assert_eq!(installed["model"], "claude-test");
    assert_eq!(
        installed["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
        "claude-test"
    );
    assert_eq!(
        installed["env"]["CLAUDE_CODE_SUBAGENT_MODEL"],
        "claude-test"
    );
}

#[test]
fn claude_one_m_works_in_native_mode_and_keeps_user_context_env() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    request.options.claude_one_m.startup = true;
    write(
        &manager.paths.claude,
        &json!({"env": {"CLAUDE_CODE_AUTO_COMPACT_WINDOW": "400000"}}),
    );

    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    let model = format!("{}[1m]", request.model);
    assert_eq!(installed["env"]["ANTHROPIC_MODEL"], model.as_str());
    assert_eq!(
        installed["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"],
        "400000"
    );

    request.options.claude_one_m = disabled_one_m();
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(
        installed["env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"],
        "400000"
    );
}

fn versioned_claude_fixture() -> (tempfile::TempDir, ConfigManager, Store, ConfigRequest) {
    let (temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    request.model = "claude-opus-5-5".into();
    store.providers[0].model = request.model.clone();
    store.providers[0].models = [
        ("claude-opus-5-5", "Opus 5.5"),
        ("claude-fable-5-1", "Fable 5.1"),
        ("claude-opus-4-8", "Opus 4.8"),
    ]
    .into_iter()
    .map(|(id, name)| crate::core::newapi::Model {
        id: id.into(),
        display_name: name.into(),
        included: true,
        ..Default::default()
    })
    .collect();
    (temp, manager, store, request)
}

#[test]
fn claude_imports_multiple_versions_and_preserves_manual_picker_entries() {
    let (_temp, manager, store, request) = versioned_claude_fixture();
    let original = json!({
        "availableModels": ["opus"],
        "modelPicker": {
            "options": [
                {"model": "manual-deployment", "label": "Manual", "description": "Keep"},
                {"model": "claude-opus-4-8", "label": "Production", "behavesAs": "claude-opus-4-8"}
            ],
            "replaceBuiltInOptions": false,
            "custom": "keep"
        }
    });
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    let entries = installed["modelPicker"]["options"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    for id in ["claude-opus-5-5", "claude-fable-5-1", "claude-opus-4-8"] {
        assert_eq!(
            entries.iter().filter(|entry| entry["model"] == id).count(),
            1
        );
        assert!(installed["availableModels"]
            .as_array()
            .unwrap()
            .contains(&json!(id)));
    }
    assert_eq!(entries[0], original["modelPicker"]["options"][0]);
    assert_eq!(entries[1], original["modelPicker"]["options"][1]);
    assert_eq!(installed["modelPicker"]["custom"], "keep");
    assert_eq!(installed["modelPicker"]["replaceBuiltInOptions"], true);
    assert_eq!(installed["model"], "claude-opus-5-5");
    assert_eq!(
        installed["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"],
        "claude-fable-5-1"
    );
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(read(&manager.paths.claude), original);
}

#[test]
fn claude_reapplying_a_subset_removes_previously_managed_model_versions() {
    let (_temp, manager, store, mut request) = versioned_claude_fixture();
    apply(&manager, &store, &request);
    request.options.claude_model_ids =
        Some(vec!["claude-opus-5-5".into(), "claude-fable-5-1".into()]);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    let entries = installed["modelPicker"]["options"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(!entries
        .iter()
        .any(|entry| entry["model"] == "claude-opus-4-8"));
    assert_eq!(
        manager.list()[0]
            .request
            .as_ref()
            .unwrap()
            .options
            .claude_model_ids,
        request.options.claude_model_ids
    );
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert!(!manager.paths.claude.exists());
}

#[test]
fn claude_removes_legacy_generated_picker_rows_from_other_relays() {
    let (_temp, manager, store, request) = versioned_claude_fixture();
    let original = json!({"availableModels": ["claude-opus-5-kiro", "manual-deployment"], "modelPicker": {"options": [
        {"model": "claude-opus-5-kiro", "label": "claude-opus-5-kiro"},
        {"model": "claude-opus-5-5", "label": "claude-opus-5-5"},
        {"model": "manual-deployment", "label": "My own model"},
        {"model": "manual-with-metadata", "label": "manual-with-metadata", "description": "Keep"}
    ]}});
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    let entries = installed["modelPicker"]["options"].as_array().unwrap();
    assert!(!entries
        .iter()
        .any(|entry| entry["model"] == "claude-opus-5-kiro"));
    assert!(entries
        .iter()
        .any(|entry| entry["model"] == "manual-deployment"));
    assert!(entries
        .iter()
        .any(|entry| entry["model"] == "manual-with-metadata"));
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["model"] == "claude-opus-5-5")
            .count(),
        1
    );
    assert!(!installed["availableModels"]
        .as_array()
        .unwrap()
        .contains(&json!("claude-opus-5-kiro")));
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(read(&manager.paths.claude), original);
}

#[test]
fn claude_rejects_empty_unknown_and_inconsistent_model_selections_without_writing() {
    let (_temp, manager, store, mut request) = versioned_claude_fixture();
    for ids in [
        vec![],
        vec!["not-in-provider".into()],
        vec!["claude-opus-4-8".into()],
    ] {
        request.options.claude_model_ids = Some(ids);
        assert_eq!(
            manager.preview(&store, &request).err().as_deref(),
            Some("clients.errors.modelSelection")
        );
        assert!(!manager.paths.claude.exists());
    }
    request.options.claude_model_ids = Some(vec![request.model.clone()]);
    request.options.claude_models.opus = "claude-opus-4-8".into();
    assert_eq!(
        manager.preview(&store, &request).err().as_deref(),
        Some("clients.errors.protocol")
    );
    assert!(!manager.paths.claude.exists());
}

#[test]
fn claude_rejects_malformed_existing_picker_without_overwriting_it() {
    let (_temp, manager, store, request) = versioned_claude_fixture();
    for picker in [json!("invalid"), json!({"options": {}})] {
        let original = json!({"modelPicker": picker});
        write(&manager.paths.claude, &original);
        assert_eq!(
            manager.preview(&store, &request).err().as_deref(),
            Some("clients.errors.invalidConfig")
        );
        assert_eq!(read(&manager.paths.claude), original);
    }
}

#[test]
fn native_defaults_and_credentials_are_reversible_without_a_router() {
    let (_temp, manager, mut store, request) = fixture();
    let request: ConfigRequest = serde_json::from_value(json!({
        "targetId":"claudeCode","providerId":request.provider_id,"model":request.model,
        "contextWindow":200000,"maxOutputTokens":8192
    }))
    .unwrap();
    assert!(!request.needs_router());
    store.router.bearer_token.clear();
    let original = json!({"env":{"ANTHROPIC_API_KEY":"previous-secret","CUSTOM":"keep"}});
    write(&manager.paths.claude, &original);
    let preview = manager.preview(&store, &request).unwrap();
    for secret in ["upstream-secret", "previous-secret"] {
        assert!(!preview.content.contains(secret));
    }
    assert!(preview
        .removed_fields
        .contains(&"env.ANTHROPIC_API_KEY".to_owned()));
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(
        installed["env"]["ANTHROPIC_BASE_URL"],
        "https://relay.example/gateway"
    );
    assert_eq!(installed["env"]["ANTHROPIC_AUTH_TOKEN"], "upstream-secret");
    assert!(installed["env"].get("ANTHROPIC_API_KEY").is_none());
    assert!(!has_router_configs(manager.data_dir.parent().unwrap()).unwrap());
    assert!(!manager.list()[0].request.as_ref().unwrap().needs_router());
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(read(&manager.paths.claude), original);
}

#[test]
fn claude_matches_roles_and_restores_both_original_auth_fields() {
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    request.options.auth_field = ClaudeAuthField::ApiKey;
    request.model = "claude-sonnet-custom".into();
    store.providers[0].model = request.model.clone();
    store.providers[0].models = ["sonnet", "opus", "haiku", "fable"]
        .into_iter()
        .map(|role| crate::core::newapi::Model {
            id: format!("claude-{role}-custom"),
            display_name: format!("Relay {role}"),
            included: true,
            ..Default::default()
        })
        .collect();
    request.options.claude_models.subagent = "claude-opus-custom".into();
    let original =
        json!({"env":{"ANTHROPIC_AUTH_TOKEN":"old-token","ANTHROPIC_API_KEY":"old-key"}});
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    let env = &installed["env"];
    assert_eq!(env["ANTHROPIC_DEFAULT_OPUS_MODEL"], "claude-opus-custom");
    assert_eq!(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "claude-haiku-custom");
    assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"], "Relay sonnet");
    assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL"], "claude-opus-custom");
    assert_eq!(env["ANTHROPIC_API_KEY"], "upstream-secret");
    assert!(env.get("ANTHROPIC_AUTH_TOKEN").is_none());
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(read(&manager.paths.claude), original);
}

#[test]
fn legacy_sessions_keep_router_mode_until_explicitly_changed() {
    let (_temp, manager, store, mut request) = fixture();
    apply(&manager, &store, &request);
    let path = manager.session_path(request.target_id);
    let mut session: Value =
        serde_json::from_slice(&secret_file::read(&path).unwrap().unwrap()).unwrap();
    session["request"]
        .as_object_mut()
        .unwrap()
        .remove("accessMode");
    secret_file::write(&path, &serde_json::to_vec(&session).unwrap()).unwrap();
    assert!(manager.list()[0].request.as_ref().unwrap().needs_router());
    assert!(has_router_configs(manager.data_dir.parent().unwrap()).unwrap());
    request.options.access_mode = ClientAccessMode::Native;
    apply(&manager, &store, &request);
    assert!(!has_router_configs(manager.data_dir.parent().unwrap()).unwrap());
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert!(!manager.paths.claude.exists());
}

#[test]
fn opencode_native_selects_cards_and_preserves_headers_and_custom_models() {
    let (_temp, manager, mut store, mut request) = fixture();
    let mut second = store.providers[0].clone();
    second.id = "second".into();
    second.protocol = ModelProtocol::ChatCompletions;
    second.model = "chat-model".into();
    second.context_window = Some(999999);
    second.models = vec![
        crate::core::newapi::Model {
            id: "responses-model".into(),
            display_name: "Response model".into(),
            included: true,
            protocol: Some(ModelProtocol::Responses),
            context_window: Some(64000),
            max_output_tokens: Some(4096),
            // Collected declarations, not name-based guesses.
            metadata_source: "upstream".into(),
            ..Default::default()
        },
        crate::core::newapi::Model {
            id: "chat-model".into(),
            display_name: "Chat model".into(),
            included: true,
            protocol: Some(ModelProtocol::ChatCompletions),
            context_window: Some(96000),
            max_output_tokens: Some(4096),
            metadata_source: "upstream".into(),
            ..Default::default()
        },
    ];
    store.providers.push(second);
    store.providers.push(Provider {
        id: "unselected".into(),
        ..store.providers[0].clone()
    });
    request.target_id = TargetId::OpenCode;
    request.options.access_mode = ClientAccessMode::Native;
    request.options.provider_ids = vec![request.provider_id.clone(), "second".into()];
    request
        .options
        .provider_model_ids
        .insert("second".into(), vec!["responses-model".into()]);
    store.router.bearer_token.clear();
    let original = json!({"provider":{"official":{"name":"Existing"},"agentharbor_second":{
        "options":{"headers":{"X-Custom":"keep"}},"models":{"manual":{"name":"Manual"},
            "chat-model":{"options":{"custom":"keep"},"variants":{"fast":{"custom":true}}}}
    }}});
    write(&manager.paths.opencode, &original);
    assert!(!manager
        .preview(&store, &request)
        .unwrap()
        .content
        .contains("upstream-secret"));
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.opencode);
    let providers = &installed["provider"];
    assert!(providers.get("agentharbor_unselected").is_none());
    assert_eq!(
        providers["agentharbor_second_responses"]["options"]["baseURL"],
        "https://relay.example/gateway/v1"
    );
    assert_eq!(
        providers["agentharbor_second"]["options"]["headers"]["X-Custom"],
        "keep"
    );
    assert_eq!(
        providers["agentharbor_second"]["models"]["manual"]["name"],
        "Manual"
    );
    assert_eq!(
        providers["agentharbor_second"]["models"]["chat-model"]["options"]["custom"],
        "keep"
    );
    assert_eq!(
        providers["agentharbor_second"]["models"]["chat-model"]["variants"]["fast"]["custom"],
        true
    );
    assert_eq!(
        providers["agentharbor_second_responses"]["npm"],
        "@ai-sdk/openai"
    );
    assert_eq!(
        providers["agentharbor_second_responses"]["models"]["responses-model"]["limit"]["context"],
        // The provider-wide window setting overrides per-model values, exactly
        // as the router's resolver does.
        999999
    );
    assert!(providers["agentharbor_second"]["models"]["chat-model"]["limit"].is_null());
    assert!(!has_router_configs(manager.data_dir.parent().unwrap()).unwrap());
    manager.restore(TargetId::OpenCode).unwrap();
    assert_eq!(read(&manager.paths.opencode), original);
}

#[test]
fn claude_router_accepts_openai_protocols_but_native_rejects_them() {
    for protocol in [ModelProtocol::Responses, ModelProtocol::ChatCompletions] {
        let (_temp, manager, mut store, mut request) = fixture();
        // A non-Claude alias lets the explicitly configured protocol decide.
        store.providers[0].model = "custom-model".into();
        request.model = "custom-model".into();
        store.providers[0].protocol = protocol;
        assert!(manager.preview(&store, &request).is_ok());
        request.options.access_mode = ClientAccessMode::Native;
        assert_eq!(
            manager.preview(&store, &request).err().as_deref(),
            Some("clients.errors.protocol")
        );
        assert!(!manager.paths.claude.exists());
    }
}

#[test]
fn any_selected_native_key_change_invalidates_preview() {
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    request.target_id = TargetId::OpenCode;
    request.options.provider_ids = vec![request.provider_id.clone(), "second".into()];
    store.providers.push(Provider {
        id: "second".into(),
        ..store.providers[0].clone()
    });
    let preview = manager.preview(&store, &request).unwrap();
    store.providers[1].api_key = "rotated-secret".into();
    assert_eq!(
        manager
            .apply(&store, &request, &preview.revision)
            .err()
            .as_deref(),
        Some("clients.errors.changed")
    );
    assert!(!manager.paths.opencode.exists());
}

#[test]
fn reintroducing_a_removed_auth_key_creates_a_conflict() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    apply(&manager, &store, &request);
    let mut changed = read(&manager.paths.claude);
    changed["env"]["ANTHROPIC_API_KEY"] = json!("user-added-secret");
    write(&manager.paths.claude, &changed);
    assert!(manager.list()[0].conflict);
    assert!(manager.restore(TargetId::ClaudeCode).is_err());
    assert_eq!(read(&manager.paths.claude), changed);
}

#[test]
fn release_leaves_a_conflicting_file_untouched_and_allows_reapply() {
    let (_temp, manager, store, request) = fixture();
    apply(&manager, &store, &request);
    // The user edits a managed field, e.g. through Claude Code's /model.
    let mut edited = read(&manager.paths.claude);
    edited["model"] = json!("user-picked");
    write(&manager.paths.claude, &edited);
    assert!(manager.list()[0].conflict);
    assert!(manager.restore(TargetId::ClaudeCode).is_err());

    manager.release(TargetId::ClaudeCode).unwrap();
    assert_eq!(
        read(&manager.paths.claude),
        edited,
        "release never rewrites the file"
    );
    let status = &manager.list()[0];
    assert!(!status.configured && !status.conflict);

    apply(&manager, &store, &request);
    assert_eq!(read(&manager.paths.claude)["model"], request.model.as_str());
}

#[test]
fn unreadable_session_does_not_block_router_reconcile() {
    let (_temp, manager, _store, _request) = fixture();
    std::fs::create_dir_all(&manager.data_dir).unwrap();
    std::fs::write(manager.session_path(TargetId::ClaudeCode), b"corrupt").unwrap();
    assert!(!has_router_configs(manager.data_dir.parent().unwrap()).unwrap());
    assert!(manager.list()[0].error.is_some());
    manager.release(TargetId::ClaudeCode).unwrap();
    assert!(manager.list()[0].error.is_none());
}

#[test]
fn apply_keeps_the_user_key_order() {
    let (_temp, manager, store, request) = fixture();
    storage::write_atomic(
        &manager.paths.claude,
        br#"{"zeta":1,"permissions":{"allow":["Read"]},"alpha":2}"#,
    )
    .unwrap();
    apply(&manager, &store, &request);
    let text = std::fs::read_to_string(&manager.paths.claude).unwrap();
    let zeta = text.find("\"zeta\"").unwrap();
    let permissions = text.find("\"permissions\"").unwrap();
    let alpha = text.find("\"alpha\"").unwrap();
    assert!(zeta < permissions && permissions < alpha, "{text}");
}
