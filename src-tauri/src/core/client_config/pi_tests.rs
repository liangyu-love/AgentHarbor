use super::*;
use crate::core::{
    aggregation,
    model_targets::{ClientModelSelection, ModelTargetRef},
    newapi,
};

fn pi_fixture(mode: ClientAccessMode) -> (tempfile::TempDir, ConfigManager, Store, ConfigRequest) {
    let (temp, manager, mut store, mut request) = fixture();
    request.target_id = TargetId::Pi;
    request.options.access_mode = mode;
    request.model = "chat-model".into();
    request.context_window = None;
    request.max_output_tokens = None;
    let provider = &mut store.providers[0];
    provider.model = request.model.clone();
    provider.protocol = ModelProtocol::ChatCompletions;
    provider.models = [
        ModelProtocol::ChatCompletions,
        ModelProtocol::Responses,
        ModelProtocol::AnthropicMessages,
    ]
    .into_iter()
    .zip(["chat-model", "responses-model", "messages-model"])
    .map(|(protocol, id)| newapi::Model {
        id: id.into(),
        included: true,
        protocol: Some(protocol),
        overrides: newapi::ModelOverrides {
            context_window: Some(272_000),
            max_output_tokens: Some(8192),
            ..Default::default()
        },
        ..Default::default()
    })
    .collect();
    (temp, manager, store, request)
}

fn pi_provider<'a>(installed: &'a Value, protocol: &str) -> &'a Value {
    &installed["providers"][format!("agentharbor-relay-test-relay-{protocol}")]
}

#[test]
fn pi_comments_survive_injection_and_the_file_stays_readable_by_pi() {
    let (_temp, manager, store, request) = pi_fixture(ClientAccessMode::Native);
    let original = "{\n  // user note\n  \"custom\": true\n}\n";
    storage::write_atomic(&manager.paths.pi, original.as_bytes()).unwrap();
    apply(&manager, &store, &request);
    let installed = std::fs::read_to_string(&manager.paths.pi).unwrap();
    assert!(installed.contains("// user note"));
    // Pi strips comments and then calls JSON.parse, so the rest must be strict JSON.
    assert!(jsonc_parser::cst::CstRootNode::parse(&installed, &jsonc::options(false)).is_ok());
    let value: Value = json5::from_str(&installed).unwrap();
    assert_eq!(value["custom"], true);
    assert!(value["providers"].as_object().is_some_and(|providers| !providers.is_empty()));
    manager.restore(request.target_id).unwrap();
    assert_eq!(std::fs::read_to_string(&manager.paths.pi).unwrap(), original);
}

#[test]
fn pi_direct_protocols_preserve_external_config_and_exact_restore() {
    let (_temp, manager, mut store, request) = pi_fixture(ClientAccessMode::Native);
    store.providers[0].base_url = "https://api.commandcode.ai/provider/v1".into();
    let original = "{ // external expressions stay inert\n providers: { foreign: {apiKey: '!do-not-execute', models: [{id:'keep'}]} }, custom: true\n}";
    storage::write_atomic(&manager.paths.pi, original.as_bytes()).unwrap();
    let auth = manager.paths.pi.with_file_name("auth.json");
    let settings = manager.paths.pi.with_file_name("settings.json");
    write(
        &auth,
        &json!({"anthropic":{"type":"oauth","refresh":"keep-login"}}),
    );
    write(
        &settings,
        &json!({"defaultProvider":"anthropic","defaultModel":"original"}),
    );
    let preview = manager.preview(&store, &request).unwrap();
    assert!(!preview.content.contains("upstream-secret"));
    assert!(!preview.content.contains("do-not-execute"));
    manager.apply(&store, &request, &preview.revision).unwrap();
    let installed = read(&manager.paths.pi);
    assert_eq!(installed["providers"].as_object().unwrap().len(), 4);
    for api in ["openai-completions", "openai-responses"] {
        assert_eq!(
            pi_provider(&installed, api)["baseUrl"],
            "https://api.commandcode.ai/provider/v1"
        );
        assert_eq!(pi_provider(&installed, api)["authHeader"], true);
    }
    assert_eq!(
        pi_provider(&installed, "anthropic-messages")["baseUrl"],
        "https://api.commandcode.ai/provider"
    );
    assert_eq!(
        pi_provider(&installed, "anthropic-messages")["authHeader"],
        false
    );
    assert_eq!(
        pi_provider(&installed, "openai-completions")["models"][0]["id"],
        "chat-model"
    );
    assert_eq!(
        pi_provider(&installed, "openai-completions")["models"][0]["contextWindow"],
        272000
    );
    assert_eq!(
        pi_provider(&installed, "openai-completions")["models"][0]["maxTokens"],
        8192
    );
    assert!(pi_provider(&installed, "openai-completions")["models"][0]
        .get("cost")
        .is_none());
    assert!(installed.get("defaultModel").is_none());
    manager.reapply(&store, TargetId::Pi, false).unwrap();
    assert_eq!(read(&manager.paths.pi), installed);
    manager.restore(TargetId::Pi).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.pi).unwrap(),
        original
    );
    assert_eq!(read(&auth)["anthropic"]["refresh"], "keep-login");
    assert_eq!(read(&settings)["defaultModel"], "original");
}

#[test]
fn pi_router_models_use_source_scoped_addresses_and_local_credentials() {
    let (_temp, manager, store, request) = pi_fixture(ClientAccessMode::Router);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.pi);
    for api in [
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
    ] {
        let node = pi_provider(&installed, api);
        assert_eq!(node["apiKey"], "local-secret");
        assert_eq!(node["authHeader"], true);
        assert_eq!(
            node["baseUrl"],
            format!(
                "http://127.0.0.1:43127/clients/pi/p/test-relay{}",
                if api == "anthropic-messages" {
                    ""
                } else {
                    "/v1"
                }
            )
        );
    }
    assert!(!installed.to_string().contains("upstream-secret"));
    assert!(has_router_configs(&manager.app_data_dir).unwrap());
    assert!(manager.restore_router_configs_on_exit().is_empty());
    assert!(!has_router_configs(&manager.app_data_dir).unwrap());
    assert!(!manager.paths.pi.exists());
}

#[test]
fn pi_literals_cannot_execute_shell_commands_or_expand_environment() {
    let (_temp, manager, mut store, request) = pi_fixture(ClientAccessMode::Native);
    store.providers[0].api_key = "!do-not-execute-$SECRET-${OTHER}".into();
    apply(&manager, &store, &request);
    assert_eq!(
        pi_provider(&read(&manager.paths.pi), "openai-completions")["apiKey"],
        "$!do-not-execute-$$SECRET-$${OTHER}"
    );
    assert!(!manager
        .preview(&store, &request)
        .unwrap()
        .content
        .contains("do-not-execute"));
}

#[test]
fn pi_reapply_tracks_keys_protocols_and_model_subsets_without_dropping_foreign_nodes() {
    let (_temp, manager, mut store, mut request) = pi_fixture(ClientAccessMode::Native);
    let mut second = store.providers[0].clone();
    second.id = "another".into();
    second.name = "Other".into();
    store.providers.push(second);
    request.options.provider_ids = vec!["test-relay".into(), "another".into()];
    request.options.provider_model_ids.insert(
        "test-relay".into(),
        vec!["chat-model".into(), "responses-model".into()],
    );
    request
        .options
        .provider_model_ids
        .insert("another".into(), vec!["messages-model".into()]);
    apply(&manager, &store, &request);
    let mut document = read(&manager.paths.pi);
    assert_eq!(document["providers"].as_object().unwrap().len(), 3);
    assert!(document["providers"]
        .get("agentharbor-relay-test-relay-anthropic-messages")
        .is_none());
    document["providers"]["foreign"] = json!({"apiKey":"$KEEP","models":[{"id":"other"}]});
    document["providers"]["agentharbor-relay-test-relay-openai-completions"]["custom"] =
        json!("keep");
    write(&manager.paths.pi, &document);
    store.providers[0].api_key = "updated".into();
    store.providers[0].protocol = ModelProtocol::Responses;
    manager.reapply(&store, TargetId::Pi, false).unwrap();
    let document = read(&manager.paths.pi);
    assert_eq!(document["providers"].as_object().unwrap().len(), 4);
    assert_eq!(
        pi_provider(&document, "openai-completions")["apiKey"],
        "updated"
    );
    assert_eq!(
        pi_provider(&document, "openai-completions")["custom"],
        "keep"
    );
    manager.restore(TargetId::Pi).unwrap();
    let restored = read(&manager.paths.pi);
    assert_eq!(restored["providers"]["foreign"]["apiKey"], "$KEEP");
    assert_eq!(
        pi_provider(&restored, "openai-completions")["custom"],
        "keep"
    );
    assert!(pi_provider(&restored, "openai-completions")
        .get("apiKey")
        .is_none());
}

#[test]
fn pi_foreign_collision_and_invalid_structure_leave_original_untouched() {
    let (_temp, manager, store, request) = pi_fixture(ClientAccessMode::Native);
    for value in [
        json!({"providers":[]}),
        json!({"providers":{"agentharbor-relay-test-relay-openai-completions":{"apiKey":"external"}}}),
    ] {
        write(&manager.paths.pi, &value);
        let before = std::fs::read(&manager.paths.pi).unwrap();
        assert!(manager.preview(&store, &request).is_err());
        assert_eq!(std::fs::read(&manager.paths.pi).unwrap(), before);
        assert!(manager.stored_request(TargetId::Pi).unwrap().is_none());
    }
}

#[test]
fn pi_preview_revision_and_external_model_array_edits_are_guarded() {
    let (_temp, manager, store, request) = pi_fixture(ClientAccessMode::Native);
    let preview = manager.preview(&store, &request).unwrap();
    write(&manager.paths.pi, &json!({"external":true}));
    assert_eq!(
        manager
            .apply(&store, &request, &preview.revision)
            .err()
            .as_deref(),
        Some("clients.errors.changed")
    );
    apply(&manager, &store, &request);
    let mut document = read(&manager.paths.pi);
    document["providers"]["agentharbor-relay-test-relay-openai-completions"]["models"][0]
        ["maxTokens"] = json!(7);
    write(&manager.paths.pi, &document);
    assert!(
        manager
            .list()
            .iter()
            .find(|s| s.target_id == TargetId::Pi)
            .unwrap()
            .conflict
    );
    assert_eq!(
        manager
            .reapply(&store, TargetId::Pi, false)
            .err()
            .as_deref(),
        Some("clients.errors.conflict")
    );
    assert_eq!(read(&manager.paths.pi), document);
}

#[test]
fn pi_aggregate_uses_public_ids_and_restores_all_protocols() {
    for protocol in [
        ModelProtocol::Responses,
        ModelProtocol::ChatCompletions,
        ModelProtocol::AnthropicMessages,
    ] {
        let (_temp, manager, mut store, mut request) = pi_fixture(ClientAccessMode::Router);
        store.aggregates.push(aggregation::AggregateModel {
            id: "pool".into(),
            public_model_id: "public-deepseek".into(),
            display_name: "DeepSeek".into(),
            members: vec![aggregation::AggregateMember {
                id: "member".into(),
                provider_id: "test-relay".into(),
                actual_model_id: "chat-model".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        request.options.aggregate_ingress_protocol = Some(protocol);
        request.options.model_selection = Some(ClientModelSelection {
            default_target: Some(ModelTargetRef::aggregate("pool")),
            available_targets: vec![
                ModelTargetRef::aggregate("pool"),
                ModelTargetRef::provider("test-relay", "responses-model"),
            ],
        });
        apply(&manager, &store, &request);
        let installed = read(&manager.paths.pi);
        assert_eq!(installed["providers"].as_object().unwrap().len(), 2);
        let key = format!(
            "agentharbor-aggregation-{}",
            match protocol {
                ModelProtocol::Responses => "openai-responses",
                ModelProtocol::ChatCompletions => "openai-completions",
                ModelProtocol::AnthropicMessages => "anthropic-messages",
            }
        );
        let node = &installed["providers"][key];
        assert_eq!(node["models"][0]["id"], "public-deepseek");
        assert_eq!(node["models"][0]["contextWindow"], 272000);
        assert!(node["models"][0].get("cost").is_none());
        assert_eq!(
            node["baseUrl"],
            format!(
                "http://127.0.0.1:43127/clients/pi/aggregate{}",
                if protocol == ModelProtocol::AnthropicMessages {
                    ""
                } else {
                    "/v1"
                }
            )
        );
        manager.restore(TargetId::Pi).unwrap();
        assert!(!manager.paths.pi.exists());
    }
}

#[test]
fn pi_paths_expand_tilde_and_honor_isolated_home() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(
        super::super::pi::models_path(home.path(), None),
        home.path().join(".pi/agent/models.json")
    );
    assert_eq!(
        super::super::pi::models_path(home.path(), Some("~/custom-pi".into())),
        home.path().join("custom-pi/models.json")
    );
    assert_eq!(
        super::super::pi::models_path(home.path(), Some("~".into())),
        home.path().join("models.json")
    );
    assert_eq!(
        ClientPaths::from_home(home.path()).pi,
        home.path().join(".pi/agent/models.json")
    );
}

#[test]
fn pi_catalog_capabilities_prices_and_effort_levels_share_the_router_resolver() {
    let (_temp, manager, mut store, request) = pi_fixture(ClientAccessMode::Native);
    store.providers[0].models[0].catalog_ref = Some(models_dev::ModelRef {
        provider_id: "test-catalog".into(),
        model_id: "core-model".into(),
    });
    store.providers[0].models[0].overrides.context_window = None;
    store.providers[0].models[0].overrides.max_output_tokens = None;
    store.providers[0].price_multiplier = Some(2.0);
    let catalog = models_dev::Catalog::parse(&serde_json::to_vec(&json!({
        "schemaVersion":1,"sourceUrl":"https://models.dev/api.json","fetchedAt":1700000000,"revision":"pi-test",
        "providers":[{"id":"test-catalog","name":"Test","models":[{"id":"core-model","name":"Core","limit":{"context":1000000,"output":32768},
            "reasoning":true,"reasoningOptions":[{"type":"effort","values":["low","high","max"]}],
            "modalities":{"input":["text","image"],"output":["text"]},"cost":{"input":2,"output":8,"cacheRead":0.1,"cacheWrite":1}}]}]
    })).unwrap(), models_dev::Origin::Embedded).unwrap();
    let mut warnings = Vec::new();
    let mut ctx = EditContext {
        legacy_picker_cleanup: false,
        warnings: &mut warnings,
        managed_provider_keys: Vec::new(),
        proxy: None,
        bypass: None,
    };
    let edits = PI
        .edits(&json!({}), &store, &request, &catalog, &[], &mut ctx)
        .unwrap();
    let mut installed = json!({});
    for edit in edits {
        set_field(&mut installed, &edit.path, edit.value).unwrap();
    }
    let model = &pi_provider(&installed, "openai-completions")["models"][0];
    assert_eq!(model["contextWindow"], 1000000);
    assert_eq!(model["maxTokens"], 32768);
    assert_eq!(model["reasoning"], true);
    assert_eq!(model["input"], json!(["text", "image"]));
    assert_eq!(model["thinkingLevelMap"]["high"], "high");
    assert_eq!(model["thinkingLevelMap"]["medium"], Value::Null);
    assert_eq!(model["thinkingLevelMap"]["xhigh"], Value::Null);
    assert_eq!(
        model["cost"],
        json!({"input":4.0,"output":16.0,"cacheRead":0.2,"cacheWrite":2.0})
    );
    assert!(!manager.paths.pi.exists(), "projection is read-only");
}

#[test]
fn pi_unknown_output_uses_the_clients_bounded_default_and_invalid_limits_do_not_write() {
    let (_temp, manager, mut store, mut request) = pi_fixture(ClientAccessMode::Native);
    store.providers[0].models[0].overrides.max_output_tokens = None;
    store.providers[0].models[0].overrides.reasoning =
        Some(vec![crate::core::model_metadata::ReasoningLevel {
            effort: "high".into(),
            description: "Manual high".into(),
        }]);
    apply(&manager, &store, &request);
    assert_eq!(
        pi_provider(&read(&manager.paths.pi), "openai-completions")["models"][0]["maxTokens"],
        8192
    );
    let installed = read(&manager.paths.pi);
    let model = &pi_provider(&installed, "openai-completions")["models"][0];
    assert_eq!(model["reasoning"], true);
    assert_eq!(model["thinkingLevelMap"]["high"], "high");
    assert_eq!(model["thinkingLevelMap"]["medium"], Value::Null);
    let original = std::fs::read(&manager.paths.pi).unwrap();
    request.context_window = Some(2048);
    request.max_output_tokens = Some(4096);
    assert_eq!(
        manager.preview(&store, &request).err().as_deref(),
        Some("clients.errors.limits")
    );
    assert_eq!(std::fs::read(&manager.paths.pi).unwrap(), original);
}

#[test]
#[ignore = "Local Pi CLI smoke fixture export requires explicit isolated output and loopback port"]
fn pi_export_cli_smoke_fixtures() {
    let root =
        PathBuf::from(std::env::var_os("AGENT_HARBOR_PI_SMOKE_ROOT").expect("explicit root"));
    assert!(root.is_absolute());
    let port: u16 = std::env::var("AGENT_HARBOR_PI_SMOKE_PORT")
        .unwrap()
        .parse()
        .unwrap();
    for (name, mode) in [
        ("native", ClientAccessMode::Native),
        ("router", ClientAccessMode::Router),
        ("aggregate", ClientAccessMode::Router),
    ] {
        let (_temp, manager, mut store, mut request) = pi_fixture(mode);
        store.providers[0].base_url = format!("http://127.0.0.1:{port}/gateway/v1");
        store.providers[0].api_key = "!synthetic-$NO_SUCH_PI_SECRET-${NO_SUCH_PI_SECRET}".into();
        store.router.port = port;
        if name == "aggregate" {
            store.aggregates.push(aggregation::AggregateModel {
                id: "pool".into(),
                public_model_id: "public-pi-model".into(),
                display_name: "Pi pool".into(),
                members: vec![aggregation::AggregateMember {
                    id: "m".into(),
                    provider_id: "test-relay".into(),
                    actual_model_id: "chat-model".into(),
                    ..Default::default()
                }],
                ..Default::default()
            });
            request.options.model_selection = Some(ClientModelSelection::single(
                ModelTargetRef::aggregate("pool"),
            ));
        }
        apply(&manager, &store, &request);
        storage::write_atomic(
            &root.join(name).join("models.json"),
            &std::fs::read(&manager.paths.pi).unwrap(),
        )
        .unwrap();
    }
}
