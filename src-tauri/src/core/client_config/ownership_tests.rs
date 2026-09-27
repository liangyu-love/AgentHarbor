use super::*;

#[test]
fn opencode_user_edits_within_managed_provider_survive_reapply_and_restore() {
    for reapply in [false, true] {
        let (_temp, manager, store, mut request) = fixture();
        request.target_id = TargetId::OpenCode;
        apply(&manager, &store, &request);
        let mut current = read(&manager.paths.opencode);
        let provider = &mut current["provider"]["agentharbor_test-relay"];
        provider["options"]["headers"] = json!({"X-Custom": "user-value"});
        provider["models"]["claude-test"]["options"] = json!({"temperature": 0.4});
        provider["models"]["manual"] = json!({"name": "User model"});
        write(&manager.paths.opencode, &current);
        assert!(!manager.list()[1].conflict);
        if reapply {
            apply(&manager, &store, &request);
            assert_eq!(read(&manager.paths.opencode), current);
        }
        manager.restore(request.target_id).unwrap();
        assert_eq!(
            read(&manager.paths.opencode),
            json!({"provider": {
                "agentharbor_test-relay": {
                    "options": {"headers": {"X-Custom": "user-value"}},
                    "models": {
                        "claude-test": {"options": {"temperature": 0.4}},
                        "manual": {"name": "User model"}
                    }
                }
            }})
        );
    }
}

#[test]
fn opencode_deselecting_a_provider_keeps_later_user_customizations() {
    let (_temp, manager, mut store, mut request) = fixture();
    store.providers.push(Provider {
        id: "second".into(),
        ..store.providers[0].clone()
    });
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.opencode);
    current["provider"]["agentharbor_second"]["options"]["headers"] = json!({"X-Custom": "keep"});
    write(&manager.paths.opencode, &current);
    request.options.provider_ids = vec![request.provider_id.clone()];
    apply(&manager, &store, &request);
    let expected = json!({"options": {"headers": {"X-Custom": "keep"}}});
    assert_eq!(
        read(&manager.paths.opencode)["provider"]["agentharbor_second"],
        expected
    );
    manager.restore(request.target_id).unwrap();
    assert_eq!(
        read(&manager.paths.opencode),
        json!({"provider": {"agentharbor_second": expected}})
    );
}

#[test]
fn legacy_whole_provider_sessions_can_be_reapplied_and_restored() {
    let (_temp, manager, store, mut request) = fixture();
    request.target_id = TargetId::OpenCode;
    apply(&manager, &store, &request);
    let mut session = manager.session(request.target_id).unwrap().unwrap();
    session
        .changes
        .retain(|change| change.path[0] != "provider");
    session.changes.push(FieldChange {
        path: vec!["provider".into(), "agentharbor_test-relay".into()],
        before: None,
        after: Some(session.installed["provider"]["agentharbor_test-relay"].clone()),
    });
    secret_file::write(
        &manager.session_path(request.target_id),
        &serde_json::to_vec(&session).unwrap(),
    )
    .unwrap();
    apply(&manager, &store, &request);
    let mut current = read(&manager.paths.opencode);
    current["provider"]["agentharbor_test-relay"]["options"]["headers"] =
        json!({"X-Custom": "keep"});
    write(&manager.paths.opencode, &current);
    manager.restore(request.target_id).unwrap();
    assert_eq!(
        read(&manager.paths.opencode),
        json!({"provider": {
            "agentharbor_test-relay": {"options": {"headers": {"X-Custom": "keep"}}}
        }})
    );
}

#[test]
fn opencode_owned_leaf_changes_still_block_writes() {
    for path in [
        vec!["options", "baseURL"],
        vec!["options", "apiKey"],
        vec!["models", "claude-test", "limit", "context"],
    ] {
        let (_temp, manager, store, mut request) = fixture();
        request.target_id = TargetId::OpenCode;
        apply(&manager, &store, &request);
        let mut current = read(&manager.paths.opencode);
        set_field(
            &mut current["provider"]["agentharbor_test-relay"],
            &path.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>(),
            Some(json!("user-edit")),
        )
        .unwrap();
        write(&manager.paths.opencode, &current);
        assert!(manager.list()[1].conflict);
        assert_eq!(
            manager.preview(&store, &request).err().as_deref(),
            Some("clients.errors.conflict")
        );
        assert_eq!(
            manager.restore(request.target_id).err().as_deref(),
            Some("clients.errors.conflict")
        );
        assert_eq!(read(&manager.paths.opencode), current);
    }
}

#[test]
fn restoring_after_independent_edits_prunes_only_created_empty_objects() {
    for target in [TargetId::ClaudeCode, TargetId::OpenCode] {
        for original in [
            json!({}),
            json!({"env": {}, "modelPicker": {}, "provider": {"agentharbor_test-relay": {"options": {}, "models": {}}}}),
        ] {
            let (_temp, manager, store, mut request) = fixture();
            request.target_id = target;
            let path = adapter(target).unwrap().path(&manager.paths);
            write(path, &original);
            apply(&manager, &store, &request);
            let mut current = read(path);
            current["userSetting"] = json!(true);
            write(path, &current);
            manager.restore(target).unwrap();
            let mut expected = original;
            expected["userSetting"] = json!(true);
            assert_eq!(read(path), expected);
        }
    }
}

#[test]
fn opencode_preview_does_not_copy_unmanaged_credentials() {
    let (_temp, manager, store, mut request) = fixture();
    request.target_id = TargetId::OpenCode;
    write(
        &manager.paths.opencode,
        &json!({"provider": {"agentharbor_test-relay": {
            "options": {"headers": {"Authorization": "header-secret", "X-Private": "private-value"}},
            "models": {"manual": {"options": {"accessToken": "model-secret"}}}
        }}}),
    );
    let preview = manager.preview(&store, &request).unwrap();
    for secret in [
        "header-secret",
        "private-value",
        "model-secret",
        "local-secret",
        "upstream-secret",
    ] {
        assert!(
            !preview.content.contains(secret),
            "preview exposed {secret}"
        );
    }
}

#[test]
fn redaction_covers_credential_spellings_and_nested_arrays() {
    let mut value = json!({"headers": {
        "Authorization": "secret-value", "Proxy-Authorization": "secret-value",
        "x-api-key": "secret-value", "Cookie": "secret-value", "Set-Cookie": "secret-value"
    }, "options": [{
        "accessToken": "secret-value", "refresh_token": "secret-value",
        "clientSecret": "secret-value", "password": "secret-value",
        "apiKey": "secret-value", "ANTHROPIC_AUTH_TOKEN": "secret-value",
        "maxTokens": 8192, "context": 200000
    }]});
    redact(&mut value);
    assert!(!value.to_string().contains("secret-value"));
    assert_eq!(value["options"][0]["maxTokens"], 8192);
    assert_eq!(value["options"][0]["context"], 200000);
}
