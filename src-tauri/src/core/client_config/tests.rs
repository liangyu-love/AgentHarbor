use super::*;
#[path = "aggregate_tests.rs"]
mod aggregate_tests;
#[path = "native_tests.rs"]
mod native_tests;
#[path = "ownership_tests.rs"]
mod ownership_tests;
#[path = "pi_tests.rs"]
mod pi_tests;
#[path = "proxy_tests.rs"]
mod proxy_tests;

#[test]
fn claude_user_json_follows_the_actual_config_root_not_its_directory_name() {
    let home = Path::new("home");
    assert_eq!(
        claude_user_path(home, &home.join(".claude")),
        home.join(".claude.json")
    );
    assert_eq!(
        claude_user_path(home, Path::new("custom")),
        Path::new("custom/.claude.json")
    );
    assert_eq!(
        claude_user_path(home, Path::new("custom/.claude")),
        Path::new("custom/.claude/.claude.json")
    );
}

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
        context_window: Some(200000),
        max_output_tokens: Some(8192),
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
        "http://127.0.0.1:43127/clients/claude-code/p/test-relay"
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

/// Quitting withdraws a routed client; the next router brings it back, as
/// Cursor comes back, unless it was configured by hand in between.
#[test]
fn a_router_client_withdrawn_on_exit_resumes_with_the_next_router() {
    let (_temp, manager, store, mut request) = fixture();
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    let installed = std::fs::read_to_string(&manager.paths.opencode).unwrap();
    assert!(manager.restore_router_configs_on_exit().is_empty());
    assert!(!manager.paths.opencode.exists());
    let waiting = &manager.list()[1];
    assert!(!waiting.configured && waiting.resume_pending);
    assert_eq!(waiting.request.as_ref().map(|request| request.model.as_str()), Some("claude-test"), "the dialog opens on it");

    // A relay that is gone keeps it waiting instead of dropping it.
    let mut emptied = store.clone();
    emptied.providers.clear();
    assert_eq!(manager.resume_parked(&emptied).len(), 1);
    assert!(manager.list()[1].resume_pending);

    assert!(manager.resume_parked(&store).is_empty());
    let resumed = &manager.list()[1];
    assert!(resumed.configured && !resumed.resume_pending && !resumed.conflict);
    assert_eq!(std::fs::read_to_string(&manager.paths.opencode).unwrap(), installed);
    assert!(!manager.has_parked());
}

#[test]
fn a_client_restored_released_or_reconfigured_by_hand_does_not_come_back_on_its_own() {
    let (_temp, manager, store, request) = fixture();
    let id = request.target_id;
    apply(&manager, &store, &request);
    assert!(manager.restore_router_configs_on_exit().is_empty());
    manager.discard_resume(id).unwrap();
    assert!(manager.resume_parked(&store).is_empty());
    assert!(!manager.list()[0].configured, "a discarded resume stays withdrawn");

    // Configured again by hand while waiting: that configuration stands.
    apply(&manager, &store, &request);
    assert!(manager.restore_router_configs_on_exit().is_empty());
    let mut direct = request.clone();
    direct.options.access_mode = ClientAccessMode::Native;
    apply(&manager, &store, &direct);
    assert!(!manager.has_parked());

    // Restoring by hand ends a wait the exit left behind.
    let mut routed = request.clone();
    routed.target_id = TargetId::OpenCode;
    apply(&manager, &store, &routed);
    assert!(manager.restore_router_configs_on_exit().is_empty());
    assert!(manager.has_parked());
    manager.release(TargetId::OpenCode).unwrap();
    assert!(!manager.has_parked());
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
fn exit_withdraws_the_router_connection_and_keeps_the_users_own_model_edit() {
    let (_temp, manager, store, request) = fixture();
    write(&manager.paths.claude, &json!({"env": {"CUSTOM": "before"}}));
    apply(&manager, &store, &request);
    let mut edited = read(&manager.paths.claude);
    edited["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"] = json!("my-own-opus");
    write(&manager.paths.claude, &edited);
    assert!(manager.restore_router_configs_on_exit().is_empty(), "a model edit no longer blocks the exit restore");
    let restored = read(&manager.paths.claude);
    assert!(restored["env"].get("ANTHROPIC_BASE_URL").is_none(), "the dead router address is gone");
    assert!(!restored.to_string().contains("local-secret"), "the local token never stays behind");
    assert_eq!(restored["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "my-own-opus", "the user's edit is kept");
    assert_eq!(restored["env"]["CUSTOM"], "before");
    assert!(!manager.list()[0].configured, "the client is no longer managed");
}

#[test]
fn a_direct_configuration_reports_a_relay_card_changed_since_it_was_written() {
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    apply(&manager, &store, &request);
    assert!(!manager.list_with(Some(&store))[0].stale);
    store.providers[0].api_key = "rotated-secret".into();
    assert!(manager.list_with(Some(&store))[0].stale, "the file still holds the old key");
    assert!(!manager.list()[0].stale, "without the store nothing is claimed");
    manager.reapply(&store, TargetId::ClaudeCode, false).unwrap();
    assert!(!manager.list_with(Some(&store))[0].stale, "reapplying writes the new key");
    assert_eq!(read(&manager.paths.claude)["env"]["ANTHROPIC_AUTH_TOKEN"], "rotated-secret");
    store.providers.clear();
    assert!(manager.list_with(Some(&store))[0].stale, "a deleted card counts as changed");

    let (_temp, manager, mut store, request) = fixture();
    apply(&manager, &store, &request);
    store.providers[0].api_key = "rotated-secret".into();
    assert!(!manager.list_with(Some(&store))[0].stale, "the router reads the card live");
}

#[test]
fn a_direct_configuration_drops_a_pasted_bearer_prefix_from_the_key() {
    let (_temp, manager, mut store, mut request) = fixture();
    request.options.access_mode = ClientAccessMode::Native;
    store.providers[0].api_key = "Bearer upstream-secret".into();
    apply(&manager, &store, &request);
    assert_eq!(read(&manager.paths.claude)["env"]["ANTHROPIC_AUTH_TOKEN"], "upstream-secret");
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
fn external_baseline_restore_allows_new_configuration_without_release() {
    let (_temp, manager, store, mut request) = fixture();
    let original = json!({"env":{"CUSTOM":"original"},"model":"original-model"});
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    write(&manager.paths.claude, &original);
    assert!(!manager.list()[0].conflict);
    request.model = "second-model".into();
    apply(&manager, &store, &request);
    manager.restore(request.target_id).unwrap();
    assert_eq!(read(&manager.paths.claude), original);
    assert!(!manager.list()[0].configured);
}

#[test]
fn confirmed_conflict_restore_keeps_independent_settings_and_model_rows() {
    let (_temp, manager, store, request) = fixture();
    write(&manager.paths.claude, &json!({"model":"original-model"}));
    apply(&manager, &store, &request);
    let mut changed = read(&manager.paths.claude);
    changed["model"] = json!("other-model");
    changed["theme"] = json!("dark");
    changed["modelPicker"]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"model":"user-model","label":"User"}));
    write(&manager.paths.claude, &changed);
    assert!(manager.restore(request.target_id).is_err());
    manager.restore_confirmed(request.target_id, true).unwrap();
    let restored = read(&manager.paths.claude);
    assert_eq!(restored["model"], "original-model");
    assert_eq!(restored["theme"], "dark");
    assert_eq!(
        restored["modelPicker"]["options"],
        json!([{"model":"user-model","label":"User"}])
    );
    assert!(!manager.list()[0].configured);
    apply(&manager, &store, &request);
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
fn claude_picker_selection_allows_restore_reapply_and_update_without_release() {
    for action in ["restore", "reapply", "update"] {
        let (_temp, manager, mut store, mut request) = fixture();
        store.providers[0].models = ["claude-test", "second-model"]
            .into_iter()
            .map(|id| crate::core::newapi::Model {
                id: id.into(),
                included: true,
                ..Default::default()
            })
            .collect();
        request.options.claude_model_ids = Some(vec!["claude-test".into(), "second-model".into()]);
        let original = json!({"model":"original-model","theme":"dark"});
        write(&manager.paths.claude, &original);
        apply(&manager, &store, &request);
        let mut current = read(&manager.paths.claude);
        current["model"] = json!("second-model[1m]");
        write(&manager.paths.claude, &current);
        assert!(!manager.list()[0].conflict, "{action}");
        match action {
            "restore" => {
                manager.restore(request.target_id).unwrap();
                assert_eq!(read(&manager.paths.claude), original);
            }
            "reapply" => {
                manager.reapply(&store, request.target_id, false).unwrap();
            }
            _ => {
                request.model = "second-model".into();
                apply(&manager, &store, &request);
            }
        }
        assert!(!manager.list()[0].conflict);
        request.model = "second-model".into();
        apply(&manager, &store, &request);
        assert_eq!(read(&manager.paths.claude)["model"], "second-model");
        manager.restore(request.target_id).unwrap();
        assert_eq!(read(&manager.paths.claude), original);
    }
}

#[test]
fn conflicting_configuration_can_be_previewed_and_replaced_without_release() {
    let (_temp, manager, store, mut request) = fixture();
    let original = json!({"model":"original-model","env":{"CUSTOM":"keep"}});
    write(&manager.paths.claude, &original);
    apply(&manager, &store, &request);
    let mut changed = read(&manager.paths.claude);
    changed["model"] = json!("external-model");
    changed["env"]["ANTHROPIC_BASE_URL"] = json!("https://external.example");
    changed["theme"] = json!("dark");
    changed["modelPicker"]["options"]
        .as_array_mut()
        .unwrap()
        .push(json!({"model":"user-model","label":"Mine"}));
    write(&manager.paths.claude, &changed);
    assert!(manager.list()[0].conflict);
    assert!(manager.preview(&store, &request).is_err());
    request.model = "second-model".into();
    let preview = manager
        .preview_with_conflicts(&store, &request, true)
        .unwrap();
    assert_eq!(read(&manager.paths.claude), changed, "preview is read-only");
    // The preview names what the replacement overwrites, and nothing of the user's own.
    assert!(preview.conflict_fields.contains(&"model".to_owned()));
    assert!(preview
        .conflict_fields
        .contains(&"env.ANTHROPIC_BASE_URL".to_owned()));
    assert!(preview
        .conflict_fields
        .iter()
        .all(|field| field != "theme" && !field.starts_with("modelPicker")));
    let applied = manager
        .apply_with_conflicts(&store, &request, &preview.revision, true)
        .unwrap();
    let backup: Option<String> = serde_json::from_slice(
        &secret_file::read(Path::new(&applied.backup_path))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(backup.as_deref().unwrap()).unwrap(),
        changed
    );
    assert!(!manager.list()[0].conflict);
    assert_eq!(read(&manager.paths.claude)["model"], "second-model");
    manager.reapply(&store, request.target_id, false).unwrap();
    manager.restore(request.target_id).unwrap();
    let restored = read(&manager.paths.claude);
    assert_eq!(restored["model"], "original-model");
    assert_eq!(restored["theme"], "dark");
    assert_eq!(restored["env"], original["env"]);
    assert_eq!(
        restored["modelPicker"]["options"],
        json!([{"model":"user-model","label":"Mine"}])
    );
    assert!(!manager.list()[0].configured);
    apply(&manager, &store, &request);
}

#[test]
fn confirmed_update_rejects_changes_after_preview() {
    let (_temp, manager, store, request) = fixture();
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    current["model"] = json!("external-model");
    write(&manager.paths.claude, &current);
    let preview = manager
        .preview_with_conflicts(&store, &request, true)
        .unwrap();
    assert!(manager.apply(&store, &request, &preview.revision).is_err());
    current["model"] = json!("changed-again");
    write(&manager.paths.claude, &current);
    assert_eq!(
        manager
            .apply_with_conflicts(&store, &request, &preview.revision, true)
            .err()
            .unwrap(),
        "clients.errors.changed"
    );
    assert_eq!(read(&manager.paths.claude), current);
}

#[test]
fn rebaseline_uses_the_same_restore_point_with_or_without_later_edits() {
    for later_edit in [false, true] {
        let (_temp, manager, store, mut request) = fixture();
        write(
            &manager.paths.claude,
            &json!({"model":"original-model","theme":"light"}),
        );
        apply(&manager, &store, &request);
        let mut current = read(&manager.paths.claude);
        current["model"] = json!("external-model");
        current["theme"] = json!("dark");
        write(&manager.paths.claude, &current);
        manager.reapply(&store, request.target_id, true).unwrap();
        assert!(!manager.list()[0].conflict);
        request.model = "second-model".into();
        apply(&manager, &store, &request);
        if later_edit {
            let mut current = read(&manager.paths.claude);
            current["theme"] = json!("blue");
            write(&manager.paths.claude, &current);
        }
        manager.restore(request.target_id).unwrap();
        let restored = read(&manager.paths.claude);
        assert_eq!(restored["model"], "external-model");
        assert_eq!(restored["theme"], if later_edit { "blue" } else { "dark" });
        assert!(restored.pointer("/env/ANTHROPIC_BASE_URL").is_none());
        assert!(!manager.list()[0].configured);
        apply(&manager, &store, &request);
    }
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
fn opencode_enabled_providers_keep_the_users_own_entries() {
    let (_temp, manager, store, mut request) = fixture();
    request.target_id = TargetId::OpenCode;
    write(
        &manager.paths.opencode,
        &json!({"enabled_providers": ["openai"]}),
    );
    apply(&manager, &store, &request);
    // Enabling another provider after the apply is the user's, not a conflict.
    let mut document = read(&manager.paths.opencode);
    document["enabled_providers"]
        .as_array_mut()
        .unwrap()
        .push(json!("anthropic"));
    write(&manager.paths.opencode, &document);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.opencode);
    assert!(installed["enabled_providers"]
        .as_array()
        .unwrap()
        .contains(&json!("anthropic")));
    manager.restore(request.target_id).unwrap();
    assert_eq!(
        read(&manager.paths.opencode)["enabled_providers"],
        json!(["openai", "anthropic"])
    );
}

#[test]
fn opencode_disabled_providers_keep_the_users_own_entries() {
    let (_temp, manager, store, mut request) = fixture();
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    let ours = read(&manager.paths.opencode)["model"]
        .as_str()
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    // The user had disabled our provider before the first apply.
    manager.restore(request.target_id).unwrap();
    write(
        &manager.paths.opencode,
        &json!({"disabled_providers": [ours.clone(), "unrelated"]}),
    );
    apply(&manager, &store, &request);
    assert_eq!(
        read(&manager.paths.opencode)["disabled_providers"],
        json!(["unrelated"])
    );
    // Disabling another provider after the apply is the user's, not a conflict.
    let mut document = read(&manager.paths.opencode);
    document["disabled_providers"]
        .as_array_mut()
        .unwrap()
        .push(json!("openai"));
    write(&manager.paths.opencode, &document);
    apply(&manager, &store, &request);
    assert_eq!(
        read(&manager.paths.opencode)["disabled_providers"],
        json!(["unrelated", "openai"])
    );
    // Restoring keeps the user's addition and disables ours again.
    manager.restore(request.target_id).unwrap();
    assert_eq!(
        read(&manager.paths.opencode)["disabled_providers"],
        json!(["unrelated", "openai", ours])
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
    let injected = std::fs::read_to_string(&path).unwrap();
    assert_eq!(json5::from_str::<Value>(&injected).unwrap()["theme"], "dark");
    manager.restore(request.target_id).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn jsonc_comments_survive_injection_and_restore_keeps_independent_edits() {
    let (temp, mut manager, store, mut request) = fixture();
    let path = temp.path().join(".config/opencode/opencode.jsonc");
    let original = "{\n // user comment\n \"theme\": \"dark\",\n}\n";
    storage::write_atomic(&path, original.as_bytes()).unwrap();
    manager.paths = ClientPaths::from_home(temp.path());
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    let injected = std::fs::read_to_string(&path).unwrap();
    assert!(injected.contains("// user comment"));
    assert_eq!(
        json5::from_str::<Value>(&injected).unwrap()["provider"]["agentharbor_test-relay"]["npm"],
        "@ai-sdk/anthropic"
    );
    // An edit made while injected is the user's: it survives the restore, comment included.
    let edited = injected.replace("\"theme\": \"dark\"", "\"theme\": \"light\"");
    storage::write_atomic(&path, edited.as_bytes()).unwrap();
    manager.restore(request.target_id).unwrap();
    let restored = std::fs::read_to_string(&path).unwrap();
    assert!(restored.contains("// user comment"));
    assert_eq!(
        json5::from_str::<Value>(&restored).unwrap(),
        json!({"theme": "light"})
    );
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
            "http://127.0.0.1:43127/clients/opencode/p/test-relay/v1"
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

#[test]
fn claude_settings_with_comments_can_be_injected_and_the_original_is_restored() {
    let (_temp, manager, store, request) = fixture();
    let original = "{
 // written by another tool
 \"permissions\": {\"allow\": [\"Read\"]},
}
";
    storage::write_atomic(&manager.paths.claude, original.as_bytes()).unwrap();
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["permissions"]["allow"][0], "Read");
    assert!(installed["env"]["ANTHROPIC_BASE_URL"].is_string());
    manager.restore(request.target_id).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.claude).unwrap(),
        original
    );
}

#[test]
fn restore_then_fresh_apply_preserves_plain_user_picker_rows() {
    let (_temp, manager, store, request) = fixture();
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    current["modelPicker"]["options"].as_array_mut().unwrap().push(json!({"model":"user-model","label":"user-model"}));
    write(&manager.paths.claude, &current);
    manager.restore(request.target_id).unwrap();
    apply(&manager, &store, &request);
    assert!(read(&manager.paths.claude)["modelPicker"]["options"].as_array().unwrap().iter().any(|row| row["model"] == "user-model"));
    manager.restore(request.target_id).unwrap();
    assert!(read(&manager.paths.claude)["modelPicker"]["options"].as_array().unwrap().iter().any(|row| row["model"] == "user-model"));
}

#[test]
fn installed_user_picker_selection_is_allowed_outside_requested_ids() {
    let (_temp, manager, store, mut request) = fixture();
    request.options.claude_model_ids = Some(vec!["claude-test".into()]);
    write(&manager.paths.claude, &json!({"model":"original","modelPicker":{"options":[{"model":"user-model","label":"Custom"}]}}),
    );
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.claude);
    current["model"] = json!("user-model");
    write(&manager.paths.claude, &current);
    assert!(!manager.list()[0].conflict);
    manager.restore(request.target_id).unwrap();
    assert_eq!(read(&manager.paths.claude)["model"], "original");
}

/// Files kept in memory, standing in for a remote host.
#[derive(Default)]
struct RemoteFiles(Mutex<std::collections::BTreeMap<String, String>>);

impl ClientFiles for RemoteFiles {
    fn read(&self, path: &Path) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(&path.to_string_lossy().into_owned()).cloned())
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.0.lock().unwrap().insert(path.to_string_lossy().into_owned(), String::from_utf8(bytes.to_vec()).unwrap());
        Ok(())
    }
    fn remove(&self, path: &Path) -> Result<(), String> {
        self.0.lock().unwrap().remove(&path.to_string_lossy().into_owned());
        Ok(())
    }
}

#[test]
fn a_remote_claude_code_is_configured_through_its_own_files() {
    let (temp, _local, store, mut request) = fixture();
    let files = Arc::new(RemoteFiles::default());
    files.write(Path::new("/home/u/.claude/settings.json"), br#"{"theme":"dark"}"#).unwrap();
    let journals = temp.path().join("remote-journal");
    let manager = ConfigManager::remote(&temp.path().join("data"), &journals, "/home/u/", files.clone());
    // Routed, it reaches the router through the host's tunnel, at the store's port.
    let routed = manager.preview(&store, &request).unwrap();
    assert!(routed.content.contains("http://127.0.0.1:43127/clients/claude-code/p/test-relay"), "{}", routed.content);
    request.options.access_mode = ClientAccessMode::Native;
    apply(&manager, &store, &request);
    let written: Value = serde_json::from_str(&files.read(Path::new("/home/u/.claude/settings.json")).unwrap().unwrap()).unwrap();
    assert_eq!(written["theme"], "dark");
    assert_eq!(written["env"]["ANTHROPIC_BASE_URL"], "https://relay.example/gateway");
    assert_eq!(written["env"]["ANTHROPIC_AUTH_TOKEN"], "upstream-secret");
    assert!(journals.join("claude-code.dat").exists());
    let status = manager.status(TargetId::ClaudeCode, Some(&store));
    assert!(status.configured && !status.conflict && !status.stale);
    assert_eq!(status.path, "/home/u/.claude/settings.json");
    assert!(status.client_version.is_none());
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(files.read(Path::new("/home/u/.claude/settings.json")).unwrap().as_deref(), Some(r#"{"theme":"dark"}"#));
}
