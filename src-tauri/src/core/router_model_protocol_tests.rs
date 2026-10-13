use super::*;

fn provider() -> Provider {
    Provider {
        id: "mixed-fixture".into(),
        name: "Mixed fixture".into(),
        base_url: "https://fixture.invalid/".into(),
        api_key: "fixture-key".into(),
        enabled: true,
        protocol: ModelProtocol::Responses,
        model: "inherited".into(),
        models: vec![
            newapi::Model {
                id: "inherited".into(),
                ..Default::default()
            },
            newapi::Model {
                id: "responses-model".into(),
                protocol: Some(ModelProtocol::Responses),
                endpoint_types: vec!["responses".into()],
                ..Default::default()
            },
            newapi::Model {
                id: "chat-model".into(),
                protocol: Some(ModelProtocol::ChatCompletions),
                endpoint_types: vec!["openai-chat".into()],
                ..Default::default()
            },
            newapi::Model {
                id: "messages-model".into(),
                protocol: Some(ModelProtocol::AnthropicMessages),
                endpoint_types: vec!["anthropic-messages".into()],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

fn paths() -> (tempfile::TempDir, CodexPaths) {
    let directory = tempfile::tempdir().unwrap();
    let paths = CodexPaths::from_home(directory.path().join("codex"));
    std::fs::create_dir_all(&paths.codex_home).unwrap();
    (directory, paths)
}

#[test]
fn absent_and_null_model_protocols_inherit_without_overwriting_explicit_values() {
    for value in [
        serde_json::json!({"id":"inherited"}),
        serde_json::json!({"id":"inherited","protocol":null}),
    ] {
        let model: newapi::Model = serde_json::from_value(value).unwrap();
        assert_eq!(model.protocol, None);
    }
    let mut provider = provider();
    provider.protocol = ModelProtocol::ChatCompletions;
    assert_eq!(
        provider.protocol_for_model("inherited"),
        ModelProtocol::ChatCompletions
    );
    assert_eq!(
        provider.protocol_for_model("responses-model"),
        ModelProtocol::Responses
    );
    assert_eq!(
        provider.protocol_for_model("messages-model"),
        ModelProtocol::AnthropicMessages
    );
    provider.model = "messages-model".into();
    assert_eq!(
        provider.selected_protocol(),
        ModelProtocol::AnthropicMessages
    );
    assert_eq!(
        provider.model_endpoint().protocol,
        ModelProtocol::AnthropicMessages
    );
    assert!(provider.supports_model("messages-model"));
    let old: Store = serde_json::from_value(serde_json::json!({"providers":[]})).unwrap();
    assert_eq!(old.codex_access_mode, CodexAccessMode::Router);
    assert_eq!(
        serde_json::to_value(CodexAccessMode::Native).unwrap(),
        "native"
    );
}

#[test]
fn claude_model_ids_default_to_anthropic_messages_case_insensitively() {
    let mut provider = provider();
    provider.protocol = ModelProtocol::Responses;
    provider.models.push(newapi::Model {
        id: "Claude-vendor-alias".into(),
        ..Default::default()
    });
    provider.models.push(newapi::Model {
        id: "vendor-CLAUDE-sonnet".into(),
        ..Default::default()
    });
    provider.models.push(newapi::Model {
        id: "CLAUDE-openai-compatible".into(),
        endpoint_types: vec!["openai-chat".into()],
        ..Default::default()
    });

    assert_eq!(
        provider.protocol_for_model("Claude-vendor-alias"),
        ModelProtocol::AnthropicMessages
    );
    assert_eq!(
        provider.protocol_for_model("vendor-CLAUDE-sonnet"),
        ModelProtocol::AnthropicMessages
    );
    assert_eq!(
        provider.protocol_for_model("CLAUDE-openai-compatible"),
        ModelProtocol::ChatCompletions
    );
    assert!(provider.supports_model("CLAUDE-openai-compatible"));
    assert_eq!(
        provider.protocol_for_model("responses-model"),
        ModelProtocol::Responses
    );
}

#[test]
fn provider_protocol_inference_uses_the_selected_model() {
    let models = vec![
        newapi::Model {
            id: "claude-sonnet".into(),
            ..Default::default()
        },
        newapi::Model {
            id: "gpt-model".into(),
            ..Default::default()
        },
    ];
    assert_eq!(
        super::protocol::infer_protocol_from_models(&models, "gpt-model"),
        ModelProtocol::ChatCompletions
    );
    assert_eq!(
        super::protocol::infer_protocol_from_models(&models, "claude-sonnet"),
        ModelProtocol::AnthropicMessages
    );
}

#[test]
fn normalization_migration_and_export_keep_per_model_protocols() {
    let (directory, paths) = paths();
    let mut provider = provider();
    normalize_models(&mut provider.models, &provider.model).unwrap();
    let expected: Vec<_> = provider
        .models
        .iter()
        .map(|model| (model.id.clone(), model.protocol))
        .collect();
    let store = Store {
        providers: vec![provider],
        version: 1,
        ..Default::default()
    };
    let mut legacy = serde_json::to_value(&store).unwrap();
    legacy.as_object_mut().unwrap().remove("codexAccessMode");
    legacy["providers"][0]["wireApi"] = serde_json::json!("chat");
    secret_file::write(&store_path(&paths), &serde_json::to_vec(&legacy).unwrap()).unwrap();
    let migrated = load_store(&paths).unwrap();
    assert_eq!(migrated.codex_access_mode, CodexAccessMode::Router);
    assert_eq!(
        migrated.providers[0].protocol,
        ModelProtocol::ChatCompletions
    );
    assert_eq!(
        migrated.providers[0]
            .models
            .iter()
            .map(|model| (model.id.clone(), model.protocol))
            .collect::<Vec<_>>(),
        expected
    );
    let export = directory.path().join("export.json");
    export_providers(&paths, &export).unwrap();
    let target = CodexPaths::from_home(directory.path().join("import-target"));
    let imported = import_providers(&target, &export).unwrap();
    assert_eq!(imported.imported, 1);
    let imported = load_store(&target).unwrap();
    assert_eq!(
        imported.providers[0]
            .models
            .iter()
            .map(|model| (model.id.clone(), model.protocol))
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn catalog_and_pinned_selection_use_each_models_effective_protocol() {
    let (_directory, paths) = paths();
    let mut chosen = provider();
    chosen.model = "chat-model".into();
    let fallback = Provider {
        id: "responses-fallback".into(),
        ..provider()
    };
    let store = Store {
        active_provider_id: Some(chosen.id.clone()),
        providers: vec![chosen, fallback],
        ..Default::default()
    };
    assert_eq!(
        effective_active_for(&store, ModelProtocol::Responses)
            .unwrap()
            .id,
        "mixed-fixture"
    );
    let catalog = catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
    let models = catalog["models"].as_array().unwrap();
    for model in [
        "inherited",
        "responses-model",
        "chat-model",
        "messages-model",
    ] {
        assert!(models
            .iter()
            .any(|entry| entry["slug"] == format!("mixed-fixture::{model}")));
    }
}

#[test]
fn native_validation_and_save_reject_selected_non_responses_without_fallback_or_writes() {
    let (_directory, paths) = paths();
    std::fs::write(&paths.config_path, "theme = 'original'\n").unwrap();
    let chosen = provider();
    let mut store = Store {
        codex_access_mode: CodexAccessMode::Native,
        active_provider_id: Some(chosen.id.clone()),
        providers: vec![
            chosen,
            Provider {
                id: "responses-fallback".into(),
                ..provider()
            },
        ],
        version: STORE_VERSION,
        ..Default::default()
    };
    validate_native_selection(&store).unwrap();
    save_store_unlocked(&paths, &store).unwrap();
    // Raw fixtures use RouterSettings::default(), whose bearer token is empty.
    // Complete ordinary store initialization before measuring writes caused by
    // the rejected provider update, then retain exact encrypted-byte checks.
    store = load_store(&paths).unwrap();
    assert!(!store.router.bearer_token.is_empty());
    let before_store = std::fs::read(store_path(&paths)).unwrap();
    let before_config = std::fs::read(&paths.config_path).unwrap();
    let current = &store.providers[0];
    let input = ProviderInput {
        chat_search: None,
        gateway_slug: None,
        id: Some(current.id.clone()),
        name: current.name.clone(),
        base_url: current.base_url.clone(),
        user_id: None,
        access_token: String::new(),
        api_key: current.api_key.clone(),
        protocol: current.protocol,
        model: "chat-model".into(),
        models: current.models.clone(),
        context_window: None,
        auto_compact_limit: None,
        catalog_providers: None,
        price_catalog_provider: None,
        price_multiplier: None,
        compression: None,
        network_mode: "direct".into(),
    };
    assert_eq!(
        save_provider(&paths, input).unwrap_err(),
        "newapi.errors.nativeResponsesRequired"
    );
    assert_eq!(std::fs::read(store_path(&paths)).unwrap(), before_store);
    assert_eq!(std::fs::read(&paths.config_path).unwrap(), before_config);
    store.providers[0].protocol = ModelProtocol::ChatCompletions;
    store.providers[0].model = "responses-model".into();
    validate_native_selection(&store).unwrap();
    store.providers[0].model = "chat-model".into();
    assert_eq!(
        validate_native_selection(&store).unwrap_err(),
        "newapi.errors.nativeResponsesRequired"
    );
}

#[test]
fn model_refresh_preserves_overrides_and_connection_tests_use_each_models_protocol() {
    use std::io::{Read, Write};
    use std::time::{Duration, Instant};

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for (path, model, response) in [
            (
                "/v1/models",
                None,
                serde_json::json!({"data": [
                    {"id": "inherited"},
                    {"id": "chat-model", "supported_endpoint_types": ["openai-chat"]},
                    {"id": "messages-model", "supported_endpoint_types": ["anthropic-messages"]},
                    {"id": "new-chat", "supported_endpoint_types": ["openai-chat"]}
                ]}),
            ),
            (
                "/v1/chat/completions",
                Some("chat-model"),
                serde_json::json!({"choices": [{"message": {"content": "ok"}}]}),
            ),
            (
                "/v1/messages",
                Some("messages-model"),
                serde_json::json!({"type": "message"}),
            ),
            (
                "/v1/responses",
                Some("inherited"),
                serde_json::json!({"object": "response"}),
            ),
        ] {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "expected request to {path}");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = Vec::new();
            let header_end = loop {
                let mut chunk = [0u8; 4096];
                let count = socket.read(&mut chunk).unwrap();
                assert!(count > 0);
                buffer.extend_from_slice(&chunk[..count]);
                if let Some(end) = buffer.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .map(|value| value.trim().parse::<usize>().unwrap())
                        .unwrap_or(0);
                    if buffer.len() >= end + 4 + length {
                        break end;
                    }
                }
            };
            let headers = String::from_utf8_lossy(&buffer[..header_end]).to_lowercase();
            assert!(headers
                .lines()
                .next()
                .unwrap()
                .contains(&format!(" {path} ")));
            if path == "/v1/messages" {
                assert!(headers.contains("x-api-key: fixture-key"));
                assert!(headers.contains("anthropic-version: 2023-06-01"));
                assert!(!headers.contains("authorization:"));
            } else {
                assert!(headers.contains("authorization: bearer fixture-key"));
            }
            if let Some(model) = model {
                let payload: serde_json::Value =
                    serde_json::from_slice(&buffer[header_end + 4..]).unwrap();
                assert_eq!(payload["model"], model);
            }
            let response = response.to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let (_directory, paths) = paths();
    let mut selected = provider();
    selected.base_url = format!("http://127.0.0.1:{port}/");
    selected.network_mode = "direct".into();
    let store = Store {
        providers: vec![selected.clone()],
        version: STORE_VERSION,
        ..Default::default()
    };
    save_store_unlocked(&paths, &store).unwrap();
    let proxy = ApiProxyConfigPayload::default();
    let refreshed = fetch_models(
        &paths,
        Some(&selected.id),
        &selected.base_url,
        "",
        ModelProtocol::Responses,
        &proxy,
    )
    .unwrap();
    for (id, expected) in [
        ("inherited", None),
        ("chat-model", Some(ModelProtocol::ChatCompletions)),
        ("messages-model", Some(ModelProtocol::AnthropicMessages)),
        ("new-chat", None),
    ] {
        assert_eq!(
            refreshed
                .iter()
                .find(|model| model.id == id)
                .unwrap()
                .protocol,
            expected
        );
    }
    for model in ["chat-model", "messages-model", "inherited"] {
        let tested = test_provider(&paths, &selected.id, model, &proxy).unwrap();
        assert_eq!(tested.test_status, "ok");
    }
    server.join().unwrap();
}
