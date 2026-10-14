use super::*;
use crate::core::{
    aggregation,
    model_targets::{ClientModelSelection, ModelTargetRef},
    newapi,
};

fn aggregate_fixture(target: TargetId) -> (tempfile::TempDir, ConfigManager, Store, ConfigRequest) {
    let (temp, manager, mut store, mut request) = fixture();
    let provider = &mut store.providers[0];
    provider.model = "wire-model".into();
    provider.protocol = ModelProtocol::ChatCompletions;
    provider.models = ["wire-model", "subset-model", "excluded-model"]
        .into_iter()
        .map(|id| newapi::Model {
            id: id.into(),
            display_name: id.into(),
            included: true,
            protocol: Some(ModelProtocol::ChatCompletions),
            overrides: newapi::ModelOverrides {
                context_window: Some(128_000),
                input_limit: Some(120_000),
                max_output_tokens: Some(8192),
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    store.aggregates.push(aggregation::AggregateModel {
        id: "pool".into(),
        public_model_id: "official-model".into(),
        display_name: "Official model".into(),
        members: vec![aggregation::AggregateMember {
            id: "member".into(),
            provider_id: provider.id.clone(),
            actual_model_id: "wire-model".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    request.target_id = target;
    request.provider_id.clear();
    request.model.clear();
    request.context_window = None;
    request.max_output_tokens = None;
    request.options.model_selection = Some(ClientModelSelection::single(
        ModelTargetRef::aggregate("pool"),
    ));
    (temp, manager, store, request)
}

#[test]
fn claude_aggregate_saved_request_reapplies_public_identity_and_restores_original() {
    let (_temp, manager, mut store, request) = aggregate_fixture(TargetId::ClaudeCode);
    let original = "{\n \"env\":{\"CUSTOM\":\"kept\"},\"permissions\":{\"allow\":[\"Read\"]},\"model\":\"user-model\"\n}";
    storage::write_atomic(&manager.paths.claude, original.as_bytes()).unwrap();
    let preview = manager.preview(&store, &request).unwrap();
    assert!(!preview.content.contains("local-secret"));
    assert!(!preview.content.contains("upstream-secret"));
    manager.apply(&store, &request, &preview.revision).unwrap();
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["model"], "official-model");
    assert_eq!(
        installed["env"]["ANTHROPIC_BASE_URL"],
        "http://127.0.0.1:43127/clients/claude-code/aggregate"
    );
    assert_eq!(installed["env"]["ANTHROPIC_AUTH_TOKEN"], "local-secret");
    assert!(!installed.to_string().contains("upstream-secret"));
    let saved = manager
        .stored_request(TargetId::ClaudeCode)
        .unwrap()
        .unwrap();
    assert_eq!(saved.aggregate_target(), Some("pool"));
    assert_eq!(saved.model, "official-model");
    assert!(saved.provider_id.is_empty());
    let before = std::fs::read(&manager.paths.claude).unwrap();
    store.providers[0].api_key = "rotated-upstream-key".into();
    store.aggregates[0].members[0].weight = 9;
    manager
        .reapply(&store, TargetId::ClaudeCode, false)
        .unwrap();
    assert_eq!(std::fs::read(&manager.paths.claude).unwrap(), before);
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.claude).unwrap(),
        original
    );
}

#[test]
fn opencode_aggregate_and_exact_provider_subset_survive_reapply_and_restore() {
    let (_temp, manager, mut store, mut request) = aggregate_fixture(TargetId::OpenCode);
    request
        .options
        .model_selection
        .as_mut()
        .unwrap()
        .available_targets
        .push(ModelTargetRef::provider("test-relay", "subset-model"));
    let original = "{\n \"theme\":\"dark\",\"model\":\"user/model\",\"provider\":{\"user\":{\"models\":{\"model\":{\"name\":\"User\"}}}}\n}";
    storage::write_atomic(&manager.paths.opencode, original.as_bytes()).unwrap();
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.opencode);
    assert_eq!(
        installed["model"],
        "agentharbor_aggregate_responses/official-model"
    );
    assert_eq!(
        installed["provider"]["agentharbor_aggregate_responses"]["options"]["baseURL"],
        "http://127.0.0.1:43127/clients/opencode/aggregate/v1"
    );
    assert_eq!(
        installed["provider"]["agentharbor_aggregate_responses"]["npm"],
        "@ai-sdk/openai"
    );
    let regular = &installed["provider"]["agentharbor_test-relay"]["models"];
    assert!(regular.get("subset-model").is_some());
    assert!(regular.get("wire-model").is_none());
    assert!(regular.get("excluded-model").is_none());
    assert!(!installed.to_string().contains("upstream-secret"));
    let saved = manager.stored_request(TargetId::OpenCode).unwrap().unwrap();
    assert_eq!(
        saved.options.provider_model_ids["test-relay"],
        vec!["subset-model"]
    );
    let before = std::fs::read(&manager.paths.opencode).unwrap();
    store.aggregates[0].members[0].priority = 8;
    manager.reapply(&store, TargetId::OpenCode, false).unwrap();
    assert_eq!(std::fs::read(&manager.paths.opencode).unwrap(), before);
    manager.restore(TargetId::OpenCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.opencode).unwrap(),
        original
    );
}

#[test]
fn aggregate_requests_reject_native_mode_and_missing_targets_without_touching_files() {
    for target in [TargetId::ClaudeCode, TargetId::OpenCode] {
        let (_temp, manager, mut store, mut request) = aggregate_fixture(target);
        request.options.access_mode = ClientAccessMode::Native;
        assert!(manager.preview(&store, &request).is_err());
        assert!(!manager.paths.claude.exists());
        assert!(!manager.paths.opencode.exists());
        request.options.access_mode = ClientAccessMode::Router;
        apply(&manager, &store, &request);
        let path = if target == TargetId::ClaudeCode {
            &manager.paths.claude
        } else {
            &manager.paths.opencode
        };
        let installed = std::fs::read(path).unwrap();
        store.aggregates.clear();
        assert!(manager.reapply(&store, target, false).is_err());
        assert_eq!(std::fs::read(path).unwrap(), installed);
        manager.restore(target).unwrap();
        assert!(!path.exists());
    }
}

#[test]
fn opencode_messages_aggregate_uses_its_ingress_sdk_and_respects_pool_limits() {
    let (_temp, manager, store, mut request) = aggregate_fixture(TargetId::OpenCode);
    request.options.aggregate_ingress_protocol = Some(ModelProtocol::AnthropicMessages);
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.opencode);
    let group = &installed["provider"]["agentharbor_aggregate_anthropic"];
    assert_eq!(group["npm"], "@ai-sdk/anthropic");
    assert_eq!(
        group["models"]["official-model"]["limit"]["context"],
        128_000
    );
    request.context_window = Some(1_000_000);
    assert!(manager.preview(&store, &request).is_err());
}

#[test]
fn opencode_chat_aggregate_reapply_preserves_sdk_group_and_restore_withdraws_it() {
    let (_temp, manager, mut store, mut request) = aggregate_fixture(TargetId::OpenCode);
    request.options.aggregate_ingress_protocol = Some(ModelProtocol::ChatCompletions);
    let original = "{\"model\":\"user/model\",\"provider\":{\"user\":{\"npm\":\"user-sdk\"}}}";
    storage::write_atomic(&manager.paths.opencode, original.as_bytes()).unwrap();
    // One pool still exposes every compatible egress through its Chat SDK.
    for (id, protocol, context) in [
        ("responses", ModelProtocol::Responses, 64_000),
        ("messages", ModelProtocol::AnthropicMessages, 96_000),
    ] {
        let mut provider = store.providers[0].clone();
        provider.id = id.into();
        provider.protocol = protocol;
        provider.models[0].protocol = Some(protocol);
        provider.models[0].overrides.context_window = Some(context);
        store.providers.push(provider);
        store.aggregates[0]
            .members
            .push(aggregation::AggregateMember {
                id: format!("member-{id}"),
                provider_id: id.into(),
                actual_model_id: "wire-model".into(),
                ..Default::default()
            });
    }
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.opencode);
    let group = &installed["provider"]["agentharbor_aggregate_chat"];
    assert_eq!(group["npm"], "@ai-sdk/openai-compatible");
    assert_eq!(
        group["options"]["baseURL"],
        "http://127.0.0.1:43127/clients/opencode/aggregate/v1"
    );
    assert_eq!(
        group["models"]["official-model"]["limit"]["context"],
        64_000
    );
    assert_eq!(
        installed["model"],
        "agentharbor_aggregate_chat/official-model"
    );
    assert_eq!(installed["provider"]["user"]["npm"], "user-sdk");
    let saved = manager.stored_request(TargetId::OpenCode).unwrap().unwrap();
    assert_eq!(saved.aggregate_protocol(), ModelProtocol::ChatCompletions);
    let before = std::fs::read(&manager.paths.opencode).unwrap();
    store.providers[0].api_key = "new-upstream-secret".into();
    manager.reapply(&store, TargetId::OpenCode, false).unwrap();
    assert_eq!(std::fs::read(&manager.paths.opencode).unwrap(), before);
    request.options.aggregate_ingress_protocol = Some(ModelProtocol::AnthropicMessages);
    apply(&manager, &store, &request);
    let messages = read(&manager.paths.opencode);
    assert!(messages["provider"]
        .get("agentharbor_aggregate_chat")
        .is_none());
    assert_eq!(
        messages["provider"]["agentharbor_aggregate_anthropic"]["npm"],
        "@ai-sdk/anthropic"
    );
    request.options.aggregate_ingress_protocol = Some(ModelProtocol::ChatCompletions);
    apply(&manager, &store, &request);
    let chat_again = read(&manager.paths.opencode);
    assert!(chat_again["provider"]
        .get("agentharbor_aggregate_anthropic")
        .is_none());
    assert_eq!(
        chat_again["provider"]["agentharbor_aggregate_chat"]["npm"],
        "@ai-sdk/openai-compatible"
    );
    manager.restore(TargetId::OpenCode).unwrap();
    assert_eq!(
        std::fs::read_to_string(&manager.paths.opencode).unwrap(),
        original
    );
}

#[test]
fn claude_aggregate_one_m_marker_uses_capacity_instead_of_the_egress_protocol() {
    for protocol in [
        ModelProtocol::Responses,
        ModelProtocol::ChatCompletions,
        ModelProtocol::AnthropicMessages,
    ] {
        let (_temp, manager, mut store, mut request) = aggregate_fixture(TargetId::ClaudeCode);
        store.providers[0].models[0].protocol = Some(protocol);
        store.providers[0].models[0].overrides.context_window = Some(1_000_000);
        request.options.claude_one_m.startup = true;
        apply(&manager, &store, &request);
        assert_eq!(read(&manager.paths.claude)["model"], "official-model[1m]");
        store.providers[0].models[0].overrides.context_window = Some(128_000);
        manager
            .reapply(&store, TargetId::ClaudeCode, false)
            .unwrap();
        assert_eq!(read(&manager.paths.claude)["model"], "official-model");
    }
}

#[test]
fn claude_combines_relay_models_and_aggregates_under_gateway_ids() {
    let (_temp, manager, mut store, mut request) = aggregate_fixture(TargetId::ClaudeCode);
    crate::core::gateway::assign_slugs(&mut store);
    let relay = format!("{}/subset-model", store.providers[0].gateway_slug.as_deref().unwrap());
    let selection = request.options.model_selection.as_mut().unwrap();
    selection.available_targets.push(ModelTargetRef::provider("test-relay", "subset-model"));
    selection.default_target = Some(ModelTargetRef::provider("test-relay", "subset-model"));
    request.provider_id = "test-relay".into();
    request.model = relay.clone();
    request.options.claude_model_ids = Some(vec![relay.clone(), "pool/official-model".into()]);
    request.options.claude_models.opus = "pool/official-model".into();
    let original = "{\n \"permissions\":{\"allow\":[\"Read\"]}\n}";
    storage::write_atomic(&manager.paths.claude, original.as_bytes()).unwrap();
    apply(&manager, &store, &request);
    let installed = read(&manager.paths.claude);
    assert_eq!(installed["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:43127/clients/claude-code/mix");
    assert_eq!(installed["env"]["ANTHROPIC_AUTH_TOKEN"], "local-secret");
    assert_eq!(installed["model"], relay.as_str());
    assert_eq!(installed["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "pool/official-model");
    assert_eq!(installed["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"], "Official model");
    let mut picker: Vec<_> = installed["modelPicker"]["options"].as_array().unwrap().iter()
        .map(|row| (row["model"].as_str().unwrap().to_owned(), row["label"].as_str().unwrap().to_owned())).collect();
    picker.sort();
    assert_eq!(picker, [("pool/official-model".to_owned(), "Official model".to_owned()), (relay.clone(), "Relay / subset-model".to_owned())]);
    assert!(!installed.to_string().contains("upstream-secret"));
    let saved = manager.stored_request(TargetId::ClaudeCode).unwrap().unwrap();
    assert_eq!(saved.model, relay, "the startup model keeps its gateway id");
    assert!(!manager.list()[0].conflict);
    manager.restore(TargetId::ClaudeCode).unwrap();
    assert_eq!(std::fs::read_to_string(&manager.paths.claude).unwrap(), original);

    // A model the selection does not hold is refused, as for a single relay.
    request.options.claude_model_ids = Some(vec![relay.clone(), "relay/excluded-elsewhere".into()]);
    assert_eq!(manager.preview(&store, &request).err().as_deref(), Some("clients.errors.modelSelection"));
}
