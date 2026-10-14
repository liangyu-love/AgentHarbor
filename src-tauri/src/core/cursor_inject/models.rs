//! The models Cursor is offered: the gateway's models of the kinds turned on,
//! each reached through the router's `/mix` namespace in its own protocol.
use cursor_engine::model::{ModelConfig, ModelType, OPENAI_CHAT_ENDPOINT, OPENAI_RESPONSES_ENDPOINT};
use sha2::{Digest, Sha256};

use crate::core::{
    account_sources::SourceKind,
    gateway::{self, GatewayModel, ModelKind},
    models_dev,
    router::{ModelProtocol, ModelSources, Store},
};

/// The models of the kinds `sources` turns on. Cursor's own account source is
/// left out: offering Cursor its own models back through AgentHarbor would
/// only add a detour.
pub fn offered(store: &Store, catalog: &models_dev::Catalog, sources: ModelSources) -> Vec<GatewayModel> {
    gateway::all_models(store, catalog)
        .into_iter()
        .filter(|model| match model.kind {
            ModelKind::Relay => sources.relay,
            ModelKind::Account => sources.account,
            ModelKind::Aggregate => sources.aggregate,
        })
        .filter(|model| model.account_source != Some(SourceKind::Cursor))
        .collect()
}

/// The id Cursor knows a model by: short, stable, and free of the `/` that
/// gateway ids hold.
pub fn cursor_id(gateway_id: &str) -> String {
    let digest = Sha256::digest(gateway_id.as_bytes());
    format!("ah-{}", &hex(&digest)[..10])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// How the engine calls one model: the router's `/mix` endpoint for its
/// protocol, the router's token, and the model's gateway id.
pub fn engine_model(model: &GatewayModel, order: usize, router_port: u16, token: &str) -> ModelConfig {
    let (model_type, endpoint, path) = match model.protocol {
        ModelProtocol::AnthropicMessages => (ModelType::Anthropic, String::new(), "messages"),
        ModelProtocol::ChatCompletions => (ModelType::OpenAi, OPENAI_CHAT_ENDPOINT.to_owned(), "chat/completions"),
        ModelProtocol::Responses => (ModelType::OpenAi, OPENAI_RESPONSES_ENDPOINT.to_owned(), "responses"),
    };
    let tooltip = format!("**{}** · {}\n\n{} via AgentHarbor", model.label, model.source_name, model.model_id);
    ModelConfig {
        model_hash: cursor_id(&model.id),
        sort_order: order as i64,
        display_name: model.label.clone(),
        group_name: Some(model.source_name.clone()),
        model_type,
        base_url: format!("http://127.0.0.1:{router_port}/clients/cursor/mix/v1/{path}"),
        use_full_url: true,
        api_key: token.to_owned(),
        tooltip_data: tooltip,
        model_id: model.id.clone(),
        reasoning_effort: None,
        openai_endpoint: endpoint,
        openai_extra_params_enabled: false,
        openai_extra_params: serde_json::json!({}),
        custom_headers_enabled: false,
        custom_headers: serde_json::json!({}),
        anthropic_extra_params_enabled: false,
        anthropic_extra_params: serde_json::json!({}),
        context_window_tokens: model.context_window,
        max_completion_tokens: model.output_limit,
        anthropic_max_tokens: model.output_limit,
        anthropic_thinking_effort: None,
        thinking_budget_tokens: None,
        supports_images: true,
        efforts: model.reasoning.clone(),
        default_effort: model.default_reasoning.clone(),
        created_at_ms: 0,
        updated_at_ms: 0,
    }
}

/// Every offered model as the engine calls it.
pub fn engine_models(store: &Store, catalog: &models_dev::Catalog, sources: ModelSources) -> Vec<ModelConfig> {
    offered(store, catalog, sources)
        .iter()
        .enumerate()
        .map(|(order, model)| engine_model(model, order, store.router.port, &store.router.bearer_token))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gateway_model(id: &str, protocol: ModelProtocol) -> GatewayModel {
        GatewayModel {
            id: id.into(),
            kind: ModelKind::Relay,
            source_name: "Relay A".into(),
            source_id: "relay-a".into(),
            model_id: "glm-5.3".into(),
            label: "GLM 5.3".into(),
            protocol,
            context_window: Some(200_000),
            reasoning: vec!["none".into(), "high".into()],
            default_reasoning: Some("high".into()),
            output_limit: Some(32_000),
            account_source: None,
        }
    }

    #[test]
    fn cursor_ids_are_stable_and_plain() {
        let id = cursor_id("relay-a/vendor/glm-5.3");
        assert_eq!(id, cursor_id("relay-a/vendor/glm-5.3"));
        assert_ne!(id, cursor_id("relay-b/vendor/glm-5.3"));
        assert!(id.starts_with("ah-") && id.len() == 13 && !id.contains('/'));
    }

    #[test]
    fn each_protocol_reaches_its_mix_endpoint() {
        let chat = engine_model(&gateway_model("relay-a/glm-5.3", ModelProtocol::ChatCompletions), 0, 4567, "token");
        assert_eq!(chat.base_url, "http://127.0.0.1:4567/clients/cursor/mix/v1/chat/completions");
        assert_eq!(chat.request_url().unwrap(), chat.base_url);
        assert_eq!(chat.model_id, "relay-a/glm-5.3");
        assert_eq!(chat.api_key, "token");
        assert_eq!(chat.efforts, vec!["none".to_owned(), "high".to_owned()]);

        let messages = engine_model(&gateway_model("relay-a/claude", ModelProtocol::AnthropicMessages), 1, 4567, "token");
        assert_eq!(messages.model_type, ModelType::Anthropic);
        assert_eq!(messages.request_url().unwrap(), "http://127.0.0.1:4567/clients/cursor/mix/v1/messages");

        let responses = engine_model(&gateway_model("pool/gpt", ModelProtocol::Responses), 2, 4567, "token");
        assert_eq!(responses.request_url().unwrap(), "http://127.0.0.1:4567/clients/cursor/mix/v1/responses");
    }
}
