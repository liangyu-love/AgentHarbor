use super::*;

#[test]
fn deepseek_native_messages_injection_uses_anthropic_root_and_keeps_model_marker() {
    for base in ["https://api.deepseek.com/", "https://api.deepseek.com/v1", "https://api.deepseek.com/anthropic/v1"] {
        let (_temp, manager, mut store, mut request) = fixture();
        let provider=&mut store.providers[0];
        provider.base_url=base.into();
        provider.model="deepseek-flash".into();
        provider.protocol=ModelProtocol::AnthropicMessages;
        request.model="deepseek-flash".into();
        request.options.access_mode=ClientAccessMode::Native;
        request.options.claude_one_m.startup=true;
        apply(&manager,&store,&request);
        let installed=read(&manager.paths.claude);
        assert_eq!(installed["env"]["ANTHROPIC_BASE_URL"],"https://api.deepseek.com/anthropic");
        assert_eq!(installed["env"]["ANTHROPIC_MODEL"],"deepseek-flash[1m]");
        assert_eq!(installed["env"]["ANTHROPIC_AUTH_TOKEN"],store.providers[0].api_key);
        manager.restore(TargetId::ClaudeCode).unwrap();
    }
}

fn assert_mixed_deepseek_opencode_endpoints(access_mode: ClientAccessMode) {
    for base in [
        "https://api.deepseek.com/v1",
        "https://api.deepseek.com/anthropic/v1",
    ] {
        let (_temp, manager, mut store, mut request) = fixture();
        request.target_id = TargetId::OpenCode;
        request.options.access_mode = access_mode;
        store.providers[0].base_url = base.into();
        store.providers[0].models = vec![
            crate::core::newapi::Model {
                id: "deepseek-chat".into(),
                protocol: Some(ModelProtocol::ChatCompletions),
                included: true,
                ..Default::default()
            },
            crate::core::newapi::Model {
                id: "deepseek-messages".into(),
                protocol: Some(ModelProtocol::AnthropicMessages),
                included: true,
                ..Default::default()
            },
        ];
        for default in [
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ] {
            store.providers[0].protocol = default;
            store.providers[0].model = if default == ModelProtocol::ChatCompletions {
                "deepseek-chat"
            } else {
                "deepseek-messages"
            }
            .into();
            for selected in ["deepseek-chat", "deepseek-messages"] {
                request.model = selected.into();
                apply(&manager, &store, &request);
                let installed = read(&manager.paths.opencode);
                let mut selected_key = String::new();
                for (protocol, suffix, npm, model, direct_base) in [
                    (
                        ModelProtocol::ChatCompletions,
                        "chat",
                        "@ai-sdk/openai-compatible",
                        "deepseek-chat",
                        "https://api.deepseek.com/v1",
                    ),
                    (
                        ModelProtocol::AnthropicMessages,
                        "anthropic",
                        "@ai-sdk/anthropic",
                        "deepseek-messages",
                        "https://api.deepseek.com/anthropic/v1",
                    ),
                ] {
                    let key = if protocol == default {
                        "agentharbor_test-relay".to_owned()
                    } else {
                        format!("agentharbor_test-relay_{suffix}")
                    };
                    let group = &installed["provider"][&key];
                    let (expected_base, expected_key) =
                        if access_mode == ClientAccessMode::Native {
                            (direct_base, "upstream-secret")
                        } else {
                            ("http://127.0.0.1:43127/clients/opencode/p/test-relay/v1", "local-secret")
                        };
                    assert_eq!(group["options"]["baseURL"], expected_base);
                    assert_eq!(group["options"]["apiKey"], expected_key);
                    assert_eq!(group["npm"], npm);
                    assert_eq!(group["models"].as_object().unwrap().len(), 1);
                    assert!(group["models"].get(model).is_some());
                    if selected == model {
                        selected_key = key;
                    }
                }
                assert_eq!(installed["model"], format!("{selected_key}/{selected}"));
            }
        }
        manager.restore(TargetId::OpenCode).unwrap();
    }
}

#[test]
fn opencode_mixed_deepseek_native_groups_keep_their_own_roots_when_selection_changes() {
    assert_mixed_deepseek_opencode_endpoints(ClientAccessMode::Native);
}

#[test]
fn opencode_mixed_deepseek_router_groups_share_the_local_root() {
    assert_mixed_deepseek_opencode_endpoints(ClientAccessMode::Router);
}

#[test]
fn opencode_native_messages_gateway_auth_is_exclusive_and_restores_user_headers() {
    for base in [
        "https://openrouter.ai/api/v1",
        "https://api.siliconflow.cn/v1",
        "https://api.siliconflow.com/v1",
        "https://api.novita.ai/v3/openai",
        "https://api.xiaomimimo.com/v1",
        "https://token-plan-cn.xiaomimimo.com/anthropic",
        "https://open.bigmodel.cn/api/coding/paas/v4",
        "https://open.bigmodel.cn/api/anthropic",
        "https://api.z.ai/api/coding/paas/v4",
        "https://api.z.ai/api/anthropic",
    ] {
        let (_temp, manager, mut store, mut request) = fixture();
        store.providers[0].base_url = base.into();
        request.target_id = TargetId::OpenCode;
        request.options.access_mode = ClientAccessMode::Native;
        let original = json!({"provider":{"agentharbor_test-relay":{"options":{
            "apiKey":"original-key", "authToken":"original-token",
            "headers":{
                "authorization":"Bearer original-header",
                "X-API-Key":"original-header-key", "X-Custom":"keep"
            }
        }}}});
        write(&manager.paths.opencode, &original);
        let preview = manager.preview(&store, &request).unwrap();
        for secret in [
            "upstream-secret",
            "original-token",
            "original-header",
            "original-key",
        ] {
            assert!(!preview.content.contains(secret));
        }
        apply(&manager, &store, &request);
        let installed = read(&manager.paths.opencode);
        let options = &installed["provider"]["agentharbor_test-relay"]["options"];
        assert_eq!(options["apiKey"], "");
        assert_eq!(options["authToken"], "upstream-secret");
        assert_eq!(options["headers"], json!({"X-Custom":"keep"}));

        store.providers[0].api_key = "Bearer updated-secret".into();
        apply(&manager, &store, &request);
        let installed = read(&manager.paths.opencode);
        assert_eq!(
            installed["provider"]["agentharbor_test-relay"]["options"]["authToken"],
            "updated-secret"
        );
        assert!(!installed.to_string().contains("upstream-secret"));

        request.options.access_mode = ClientAccessMode::Router;
        apply(&manager, &store, &request);
        let installed = read(&manager.paths.opencode);
        let options = &installed["provider"]["agentharbor_test-relay"]["options"];
        assert_eq!(options["apiKey"], "local-secret");
        assert!(options.get("authToken").is_none());
        assert_eq!(options["headers"], json!({"X-Custom":"keep"}));
        assert!(!installed.to_string().contains("updated-secret"));
        manager.restore(TargetId::OpenCode).unwrap();
        assert_eq!(read(&manager.paths.opencode), original);
    }
}

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
    // No explicit subagent mapping: the override is dropped so subagents
    // keep asking for their own tiers.
    assert!(installed["env"].get("CLAUDE_CODE_SUBAGENT_MODEL").is_none());
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
    assert!(installed["env"].get("CLAUDE_CODE_SUBAGENT_MODEL").is_none());
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

#[test]
fn claude_never_touches_cloud_selectors_and_refuses_enabled_ones() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    // Absent keys stay absent: no "0" placeholders are created.
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    for name in [
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        assert!(installed["env"].get(name).is_none(), "{name}");
    }
    // A falsy leftover keeps its value; it is falsy already, so rewriting it
    // would only churn the user's file.
    manager.release(TargetId::ClaudeCode).unwrap();
    let falsy = json!({"env": {"CLAUDE_CODE_USE_BEDROCK": "0", "CLAUDE_CODE_USE_VERTEX": "false"}});
    write(&manager.paths.claude, &falsy);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["env"]["CLAUDE_CODE_USE_BEDROCK"], "0");
    assert_eq!(installed["env"]["CLAUDE_CODE_USE_VERTEX"], "false");
    assert!(installed["env"].get("CLAUDE_CODE_USE_FOUNDRY").is_none());
    // A truthy selector is the user's own cloud setup and blocks the write.
    for enabled in ["1", "true", "yes", "on", " ON "] {
        manager.release(TargetId::ClaudeCode).unwrap();
        let original = json!({"env": {"CLAUDE_CODE_USE_FOUNDRY": enabled}});
        write(&manager.paths.claude, &original);
        assert_eq!(
            manager.preview(&store, &request).err().as_deref(),
            Some("clients.errors.cloudMode"),
            "flag {enabled}"
        );
        assert_eq!(read(&manager.paths.claude), original, "flag {enabled}");
    }
}

#[test]
fn claude_overwrites_a_leftover_small_fast_model_but_never_creates_it() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    request.options.claude_models.haiku = "claude-test".into();
    let original = json!({"env": {"ANTHROPIC_SMALL_FAST_MODEL": "old-gateway-haiku"}});
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(
        installed["env"]["ANTHROPIC_SMALL_FAST_MODEL"],
        "claude-test"
    );
    assert_eq!(
        installed["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "claude-test"
    );
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(read(&manager.paths.claude), original);
    // Absent stays absent: modern builds ignore the variable entirely.
    manager.release(TargetId::ClaudeCode).unwrap();
    write(&manager.paths.claude, &json!({"env": {}}));
    apply(&manager, &store, &request);
    assert!(read(&manager.paths.claude)["env"]
        .get("ANTHROPIC_SMALL_FAST_MODEL")
        .is_none());
}

#[test]
fn claude_subagent_variable_follows_explicit_mapping_or_the_tiers() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    request.options.claude_models.subagent = "claude-test".into();
    write(&manager.paths.claude, &json!({"env": {}}));
    apply(&manager, &store, &request);
    assert_eq!(
        read(&manager.paths.claude)["env"]["CLAUDE_CODE_SUBAGENT_MODEL"],
        "claude-test"
    );
    // Dropping the mapping withdraws the override so subagents keep asking
    // for their own tiers.
    request.options.claude_models.subagent = "".into();
    apply(&manager, &store, &request);
    assert!(read(&manager.paths.claude)["env"]
        .get("CLAUDE_CODE_SUBAGENT_MODEL")
        .is_none());
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(read(&manager.paths.claude), json!({"env": {}}));
}

#[test]
fn preview_warns_when_a_first_injection_overwrites_existing_fields() {
    let (_temp, manager, store, request) = fixture();
    write(
        &manager.paths.claude,
        &json!({"env": {"ANTHROPIC_BASE_URL": "https://cc-switch.example", "ANTHROPIC_AUTH_TOKEN": "other-tool-secret"},
            "model": "their-model", "modelPicker": {"options": [{"model": "their-model", "label": "Theirs"}]}}),
    );
    let preview = manager.preview(&store, &request).unwrap();
    for key in [
        "clients.warnings.takeoverEndpoint",
        "clients.warnings.takeoverCredential",
        "clients.warnings.takeoverModelConfig",
    ] {
        assert!(
            preview.warnings.iter().any(|warning| warning == key),
            "{key}: {:?}",
            preview.warnings
        );
    }
    // A re-apply under a session is guarded by field-level conflicts instead.
    apply(&manager, &store, &request);
    let second = manager.preview(&store, &request).unwrap();
    assert!(!second
        .warnings
        .iter()
        .any(|warning| warning.contains("takeover")));
    manager.restore(TargetId::ClaudeCode).unwrap();
}

#[test]
fn managed_settings_are_reported_only_when_they_conflict() {
    let (temp, manager, store, request) = fixture();
    let managed = temp.path().join("managed-settings.json");
    std::fs::write(
        &managed,
        br#"{"env": {"ANTHROPIC_BASE_URL": "https://managed.example"}}"#,
    )
    .unwrap();
    struct ManagedPathGuard;
    impl Drop for ManagedPathGuard {
        fn drop(&mut self) {
            takeover::set_managed_settings_path(None);
        }
    }
    let _guard = ManagedPathGuard;
    takeover::set_managed_settings_path(Some(managed.clone()));
    let preview = manager.preview(&store, &request).unwrap();
    assert!(
        preview
            .warnings
            .iter()
            .any(|warning| warning == "clients.warnings.managedSettings"),
        "{:?}",
        preview.warnings
    );
    // Unrelated keys and missing files are not a conflict.
    std::fs::write(&managed, br#"{"env": {"COMPANY_KEY": "x"}}"#).unwrap();
    let preview = manager.preview(&store, &request).unwrap();
    assert!(!preview
        .warnings
        .iter()
        .any(|warning| warning == "clients.warnings.managedSettings"));
}

#[test]
fn owned_arrays_withdraw_exactly_the_previous_elements_and_adopt_user_rows() {
    let (_temp, manager, store, mut request) = versioned_claude_fixture();
    let original = json!({"modelPicker": {"options": [
        {"model": "user-fancy", "label": "User", "description": "mine"}
    ]}, "availableModels": ["user-fancy"]});
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(
        installed["modelPicker"]["options"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(installed["availableModels"].as_array().unwrap().len(), 4);
    // The user hand-writes a minimal row between applies. The ownership
    // record now exists, so it survives even though its shape is plain.
    let mut current = read(&manager.paths.claude);
    current["modelPicker"]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"model": "user-plain", "label": "user-plain"}));
    write(&manager.paths.claude, &current);
    request.options.claude_model_ids =
        Some(vec!["claude-opus-5-5".into(), "claude-fable-5-1".into()]);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    let entries = installed["modelPicker"]["options"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries.contains(&json!({"model": "user-plain", "label": "user-plain"})));
    assert!(
        entries.contains(&json!({"model": "user-fancy", "label": "User", "description": "mine"}))
    );
    assert!(!entries
        .iter()
        .any(|entry| entry["model"] == "claude-opus-4-8"));
    assert_eq!(
        installed["availableModels"],
        json!(["user-fancy", "claude-opus-5-5", "claude-fable-5-1"])
    );
    manager.restore(TargetId::ClaudeCode).unwrap();
    // A row adopted between applies remains the user's data on restore too.
    let mut expected = original;
    expected["modelPicker"]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"model":"user-plain","label":"user-plain"}));
    assert_eq!(read(&manager.paths.claude), expected);
}

#[test]
fn user_added_picker_rows_are_adopted_instead_of_conflicting() {
    let (_temp, manager, store, request) = versioned_claude_fixture();
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    current["modelPicker"]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"model": "user-added", "label": "Mine"}));
    write(&manager.paths.claude, &current);
    assert!(!manager.list()[0].conflict);
    apply(&manager, &store, &request);
    let entries = read(&manager.paths.claude)["modelPicker"]["options"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(entries.len(), 4);
    assert!(entries.contains(&json!({"model": "user-added", "label": "Mine"})));
}

#[test]
fn edits_to_owned_picker_rows_are_adopted_on_reapply() {
    let (_temp, manager, store, request) = versioned_claude_fixture();
    apply(&manager, &store, &request);

    let mut current = read(&manager.paths.claude);
    let entries = current["modelPicker"]["options"].as_array_mut().unwrap();
    let opus = entries
        .iter_mut()
        .find(|entry| entry["model"] == "claude-opus-4-8")
        .unwrap();
    opus["label"] = json!("My Opus label");
    opus["description"] = json!("Configured by me");
    write(&manager.paths.claude, &current);

    assert!(!manager.list()[0].conflict);
    apply(&manager, &store, &request);

    let entries = read(&manager.paths.claude)["modelPicker"]["options"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["model"] == "claude-opus-4-8")
            .count(),
        1
    );
    assert!(entries.contains(&json!({
        "model": "claude-opus-4-8",
        "label": "My Opus label",
        "description": "Configured by me"
    })));
}

#[test]
fn released_sessions_still_withdraw_their_rows_on_the_next_apply() {
    let (_temp, manager, store, mut request) = versioned_claude_fixture();
    apply(&manager, &store, &request);
    manager.release(TargetId::ClaudeCode).unwrap();
    // The generated rows carry display-name labels, so the legacy shape
    // heuristic cannot find them; only the ownership record can.
    request.options.claude_model_ids =
        Some(vec!["claude-opus-5-5".into(), "claude-fable-5-1".into()]);
    apply(&manager, &store, &request);
    let entries = read(&manager.paths.claude)["modelPicker"]["options"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(entries.len(), 2);
    assert!(!entries
        .iter()
        .any(|entry| entry["model"] == "claude-opus-4-8"));
}

#[test]
fn ownership_is_seeded_from_a_pre_upgrade_session() {
    let (_temp, manager, store, mut request) = versioned_claude_fixture();
    apply(&manager, &store, &request);
    std::fs::remove_file(manager.data_dir.join("claude-code.owned.dat")).unwrap();
    request.options.claude_model_ids =
        Some(vec!["claude-opus-5-5".into(), "claude-fable-5-1".into()]);
    apply(&manager, &store, &request);
    let entries = read(&manager.paths.claude)["modelPicker"]["options"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(entries.len(), 2);
    assert!(!entries
        .iter()
        .any(|entry| entry["model"] == "claude-opus-4-8"));
}

#[test]
fn user_deleted_managed_rows_are_regenerated_without_duplicates() {
    let (_temp, manager, store, request) = versioned_claude_fixture();
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    let options = current["modelPicker"]["options"].as_array_mut().unwrap();
    options.retain(|entry| entry["model"] != "claude-opus-4-8");
    write(&manager.paths.claude, &current);
    assert!(!manager.list()[0].conflict);
    apply(&manager, &store, &request);
    let entries = read(&manager.paths.claude)["modelPicker"]["options"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["model"] == "claude-opus-4-8")
            .count(),
        1
    );
}

#[test]
fn claude_one_m_mark_follows_the_known_context_window() {
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    store.providers[0].models = vec![crate::core::newapi::Model {
        id: "claude-test".into(),
        included: true,
        context_window: Some(200_000),
        metadata_source: "upstream".into(),
        ..Default::default()
    }];
    request.options.claude_one_m = ClaudeOneM::default();
    let preview = manager.preview(&store, &request).unwrap();
    assert!(
        preview
            .warnings
            .iter()
            .any(|warning| warning == "clients.warnings.oneMIgnored"),
        "{:?}",
        preview.warnings
    );
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["model"], "claude-test");
    assert_eq!(installed["env"]["ANTHROPIC_MODEL"], "claude-test");
    // A model at or above 1M keeps its mark.
    store.providers[0].models[0].context_window = Some(1_000_000);
    apply(&manager, &store, &request);
    assert_eq!(read(&manager.paths.claude)["model"], "claude-test[1m]");
}

#[test]
fn user_deleted_array_keys_are_respected_on_reapply() {
    let (_temp, manager, store, request) = versioned_claude_fixture();
    write(
        &manager.paths.claude,
        &json!({"availableModels": ["legacy"]}),
    );
    apply(&manager, &store, &request);
    // The user deletes both managed arrays between applies (the picker object
    // itself stays: it also holds the managed replaceBuiltInOptions scalar,
    // whose removal would be a conflict).
    let mut current = read(&manager.paths.claude);
    current["modelPicker"]
        .as_object_mut()
        .unwrap()
        .remove("options");
    current.as_object_mut().unwrap().remove("availableModels");
    write(&manager.paths.claude, &current);
    assert!(!manager.list()[0].conflict);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    // The picker is regenerated from the current selection…
    assert_eq!(
        installed["modelPicker"]["options"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    // …but the deleted allowlist is not resurrected.
    assert!(installed.get("availableModels").is_none());
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(
        read(&manager.paths.claude),
        json!({"availableModels": ["legacy"]})
    );
}

#[test]
fn claude_role_auto_match_prefers_the_newest_version_regardless_of_order() {
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    store.providers[0].model = "claude-opus-4".into();
    request.model = "claude-opus-4".into();
    // Storage order lists the older alias first.
    store.providers[0].models = [
        ("claude-opus-4", ""),
        ("claude-opus-4-10", "Newest"),
        ("claude-fable-9-2", ""),
    ]
    .into_iter()
    .map(|(id, name)| crate::core::newapi::Model {
        id: id.into(),
        display_name: name.into(),
        included: true,
        ..Default::default()
    })
    .collect();
    apply(&manager, &store, &request);
    // The startup model fits the opus role, so the user's default wins.
    assert_eq!(
        read(&manager.paths.claude)["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "claude-opus-4"
    );
    // Without a startup hit the numerically newest alias wins, even though
    // storage order lists it second.
    store.providers[0].model = "claude-fable-9-2".into();
    request.model = "claude-fable-9-2".into();
    apply(&manager, &store, &request);
    assert_eq!(
        read(&manager.paths.claude)["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "claude-opus-4-10"
    );
}
