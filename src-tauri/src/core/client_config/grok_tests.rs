use super::*;
use crate::core::{
    aggregation,
    model_targets::{ClientModelSelection, ModelTargetRef},
    newapi,
};

fn grok_fixture(mode: ClientAccessMode) -> (tempfile::TempDir, ConfigManager, Store, ConfigRequest) {
    let (temp, manager, mut store, mut request) = fixture();
    request.target_id = TargetId::Grok;
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

fn grok(manager: &ConfigManager) -> Value {
    toml_doc::read(&std::fs::read_to_string(&manager.paths.grok).unwrap()).unwrap()
}

const ORIGINAL: &str = "# my grok\n[ui]\ntheme = \"oscura-midnight\" # dark\n\n[models]\ndefault = \"grok-build\"\nweb_search = \"grok-4.6\"\n\n[mcp_servers.context7]\ncommand = \"npx\"\n";

#[test]
fn grok_direct_models_keep_the_file_and_restore_it_exactly() {
    let (_temp, manager, store, request) = grok_fixture(ClientAccessMode::Native);
    storage::write_atomic(&manager.paths.grok, ORIGINAL.as_bytes()).unwrap();
    let preview = manager.preview(&store, &request).unwrap();
    assert!(!preview.content.contains("upstream-secret"));
    manager.apply(&store, &request, &preview.revision).unwrap();
    let text = std::fs::read_to_string(&manager.paths.grok).unwrap();
    assert!(text.starts_with("# my grok\n[ui]\ntheme = \"oscura-midnight\" # dark\n"), "{text}");
    let installed = grok(&manager);
    assert_eq!(installed["models"]["default"], "test-relay/chat-model");
    assert_eq!(installed["models"]["web_search"], "grok-4.6");
    assert_eq!(installed["mcp_servers"]["context7"]["command"], "npx");
    let models = installed["model"].as_object().unwrap();
    assert_eq!(models.len(), 3);
    for (key, backend) in [
        ("test-relay/chat-model", "chat_completions"),
        ("test-relay/responses-model", "responses"),
    ] {
        let model = &models[key];
        assert_eq!(model["api_backend"], backend);
        assert_eq!(model["base_url"], "https://relay.example/gateway/v1");
        assert_eq!(model["api_key"], "upstream-secret");
        assert_eq!(model["context_window"], 272000);
        assert_eq!(model["max_completion_tokens"], 8192);
        assert_eq!(model["supports_reasoning_effort"], false);
        assert_eq!(model["name"], key.trim_start_matches("test-relay/"), "one source: bare names");
    }
    // A relay that takes x-api-key never gets a bearer, nor Grok's own sign-in.
    let messages = &models["test-relay/messages-model"];
    assert_eq!(messages["api_backend"], "messages");
    assert!(messages.get("api_key").is_none());
    assert_eq!(messages["env_key"], "AGENTHARBOR_GROK_NO_BEARER");
    assert_eq!(messages["extra_headers"]["x-api-key"], "upstream-secret");

    manager.reapply(&store, TargetId::Grok, false).unwrap();
    assert_eq!(grok(&manager), installed);
    manager.restore(TargetId::Grok).unwrap();
    assert_eq!(std::fs::read_to_string(&manager.paths.grok).unwrap(), ORIGINAL);
}

#[test]
fn grok_router_mixes_relays_and_aggregates_under_the_router() {
    let (_temp, manager, mut store, mut request) = grok_fixture(ClientAccessMode::Router);
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
    request.options.aggregate_ingress_protocol = Some(ModelProtocol::Responses);
    request.options.model_selection = Some(ClientModelSelection {
        default_target: Some(ModelTargetRef::aggregate("pool")),
        available_targets: vec![
            ModelTargetRef::aggregate("pool"),
            ModelTargetRef::provider("test-relay", "messages-model"),
        ],
    });
    apply(&manager, &store, &request);
    let installed = grok(&manager);
    let models = installed["model"].as_object().unwrap();
    assert_eq!(models.len(), 2);
    let pool = crate::core::gateway::aggregate_id(&store, &store.aggregates[0]);
    assert_eq!(installed["models"]["default"], pool);
    assert_eq!(models[&pool]["model"], "public-deepseek");
    assert_eq!(models[&pool]["api_backend"], "responses");
    assert_eq!(models[&pool]["base_url"], "http://127.0.0.1:43127/clients/grok/aggregate/v1");
    let relay = &models["test-relay/messages-model"];
    assert_eq!(relay["name"], "Relay / messages-model", "mixed sources name the relay");
    assert_eq!(relay["base_url"], "http://127.0.0.1:43127/clients/grok/p/test-relay/v1");
    assert_eq!(relay["api_key"], "local-secret", "the router takes its token as a bearer");
    assert!(relay.get("extra_headers").is_none());
    assert!(manager.stored_request(TargetId::Grok).unwrap().unwrap().needs_router());

    // Router configurations are withdrawn on exit and resumed on start.
    assert!(manager.restore_router_configs_on_exit().is_empty());
    assert!(!manager.paths.grok.exists());
    assert!(manager.resume_parked(&store).is_empty());
    assert_eq!(grok(&manager), installed);
}

#[test]
fn grok_picking_its_own_default_is_not_a_conflict_but_another_model_is() {
    let (_temp, manager, store, request) = grok_fixture(ClientAccessMode::Native);
    storage::write_atomic(&manager.paths.grok, ORIGINAL.as_bytes()).unwrap();
    apply(&manager, &store, &request);
    let installed = std::fs::read_to_string(&manager.paths.grok).unwrap();
    let pick = |model: &str| {
        let text = installed.replace(
            "default = \"test-relay/chat-model\"",
            &format!("default = \"{model}\""),
        );
        assert_ne!(text, installed);
        storage::write_atomic(&manager.paths.grok, text.as_bytes()).unwrap();
    };
    pick("test-relay/responses-model");
    let status = manager.status(TargetId::Grok, None);
    assert!(!status.conflict, "{:?}", status.conflict_fields);
    pick("grok-4.6");
    assert_eq!(manager.status(TargetId::Grok, None).conflict_fields, ["models.default"]);
    assert_eq!(manager.restore(TargetId::Grok).err().as_deref(), Some("clients.errors.conflict"));
}

#[test]
fn grok_refuses_to_take_over_a_model_table_it_did_not_write() {
    let (_temp, manager, store, request) = grok_fixture(ClientAccessMode::Native);
    let original = "[model.\"test-relay/chat-model\"]\nmodel = \"mine\"\n";
    storage::write_atomic(&manager.paths.grok, original.as_bytes()).unwrap();
    assert_eq!(
        manager.preview(&store, &request).err().as_deref(),
        Some("clients.errors.grokModelConflict")
    );
    storage::write_atomic(&manager.paths.grok, b"model = 1\n").unwrap();
    assert_eq!(
        manager.preview(&store, &request).err().as_deref(),
        Some("clients.errors.invalidConfig")
    );
}

#[test]
fn grok_declared_efforts_become_its_effort_menu() {
    let (_temp, manager, mut store, request) = grok_fixture(ClientAccessMode::Native);
    let model = &mut store.providers[0].models[0];
    model.overrides.reasoning = Some(
        ["low", "high"]
            .into_iter()
            .map(|effort| crate::core::model_metadata::ReasoningLevel {
                effort: effort.into(),
                description: String::new(),
            })
            .collect(),
    );
    model.overrides.default_effort = Some("high".into());
    apply(&manager, &store, &request);
    let installed = grok(&manager);
    let model = &installed["model"]["test-relay/chat-model"];
    assert!(model.get("supports_reasoning_effort").is_none());
    assert_eq!(
        model["reasoning_efforts"],
        json!([
            {"id": "low", "value": "low", "label": "Low", "default": false},
            {"id": "high", "value": "high", "label": "High", "default": true},
        ])
    );
}
