use super::{model_targets::*, models_dev, protocol::ModelProtocol, router};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBundle {
    pub connection: AgentConnectionExport,
    pub config_text: String,
}

/// Connection settings for one agent: the gateway's address, the chosen
/// key (shown only when `reveal`), every gateway model the ingress can serve
/// with its limits, and the default model.
pub fn export(
    store: &router::Store,
    catalog: &models_dev::Catalog,
    ingress: ModelProtocol,
    selected_id: Option<&str>,
    profile: &str,
    key_id: &str,
    reveal: bool,
) -> Result<ExportBundle, String> {
    if !matches!(profile, "generic" | "anthropicSdk" | "opencode")
        || (profile == "anthropicSdk" && ingress != ModelProtocol::AnthropicMessages)
    {
        return Err("gateway.errors.consumerUnsupported".into());
    }
    let key = store.router.gateway_keys.iter().find(|key| key.id == key_id)
        .ok_or("gateway.errors.keyNotFound")?;
    let mut models = Vec::new();
    for model in super::gateway::models(store, catalog) {
        let target = match model.kind {
            super::gateway::ModelKind::Aggregate => ModelTargetRef::aggregate(&model.source_id),
            _ => ModelTargetRef::Provider { provider_id: model.source_id.clone(), model_id: model.model_id.clone() },
        };
        let resolved = resolve_client_target(store, &target, ingress, catalog)?;
        let (true, Some(projection)) = (resolved.available, resolved.projection.as_ref()) else { continue };
        models.push(ExportedModel {
            target_ref: target,
            public_model_id: model.id.clone(),
            label: model.label.clone(),
            capabilities: ClientModelCapabilities { one_m: Some(resolved.supports_one_m()), ..Default::default() },
            limits: ClientModelLimits {
                context_window: Some(projection.context_window),
                input_limit: projection.input_limit,
                max_output_tokens: projection.output_limit,
            },
        });
    }
    let selected = match selected_id {
        Some(id) => models.iter().find(|model| model.public_model_id == id),
        None => models.first(),
    }
    .ok_or("gateway.errors.unavailable")?;
    let root = format!("http://127.0.0.1:{}/gateway", store.router.port);
    let (base_url, request_path) = if profile == "anthropicSdk" {
        (root.clone(), "/v1/messages")
    } else {
        (
            format!("{root}/v1"),
            match ingress {
                ModelProtocol::Responses => "/responses",
                ModelProtocol::ChatCompletions => "/chat/completions",
                ModelProtocol::AnthropicMessages => "/messages",
            },
        )
    };
    let token_version = super::request_diagnostics::state_revision(key.key.as_bytes());
    let config_revision = super::request_diagnostics::state_revision(
        &serde_json::to_vec(&(&base_url, &models, &selected.public_model_id, &token_version))
            .map_err(|_| "gateway.errors.storage")?,
    );
    let models_path = format!("{root}/v1/models{}",
        if ingress == ModelProtocol::AnthropicMessages { "?format=anthropic" } else { "" });
    let connection = AgentConnectionExport {
        schema_version: CONNECTION_EXPORT_SCHEMA_VERSION,
        consumer_profile: ConsumerProfile {
            id: profile.into(),
            ingress_protocol: ingress,
            models_format: if ingress == ModelProtocol::AnthropicMessages { ModelsFormat::Anthropic } else { ModelsFormat::OpenAi },
        },
        base_url: base_url.clone(),
        request_path: request_path.into(),
        models_path: Some(models_path.clone()),
        selected_model_id: Some(selected.public_model_id.clone()),
        model_catalog: models.clone(),
        auth_scheme: LocalAuthScheme::Bearer,
        local_credential_ref: LocalCredentialRef::Gateway,
        listener_ready: super::router_proxy::is_running(),
        runtime_requirement: "localRouter".into(),
        config_revision,
        catalog_revision: catalog.snapshot.revision.clone(),
        token_version,
    };
    let token = if reveal { key.key.as_str() } else { "<gateway key>" };
    let config = if profile == "opencode" {
        let npm = match ingress {
            ModelProtocol::Responses => "@ai-sdk/openai",
            ModelProtocol::ChatCompletions => "@ai-sdk/openai-compatible",
            ModelProtocol::AnthropicMessages => "@ai-sdk/anthropic",
        };
        let model_entries: serde_json::Map<String, Value> = models.iter().map(|model| {
            (model.public_model_id.clone(), json!({"name":model.label,"limit":{"context":model.limits.context_window,"output":model.limits.max_output_tokens}}))
        }).collect();
        json!({"provider":{"agentharbor":{"name":"AgentHarbor","npm":npm,
            "options":{"baseURL":base_url,"apiKey":token},"models":model_entries}},"model":format!("agentharbor/{}",selected.public_model_id)})
    } else if profile == "anthropicSdk" {
        json!({"base_url":base_url,"api_key":token,"model":selected.public_model_id})
    } else {
        json!({"base_url":base_url,"request_path":request_path,"model":selected.public_model_id,
            "models_path":models_path,"headers":{"Authorization":format!("Bearer {token}")}})
    };
    Ok(ExportBundle {
        connection,
        config_text: serde_json::to_string_pretty(&config).map_err(|_| "gateway.errors.storage")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_uses_ingress_sdk_paths_and_never_exports_upstream_credentials() {
        use crate::core::{aggregation, newapi};
        let catalog = models_dev::Catalog::from_snapshot(
            models_dev::Snapshot {
                schema_version: models_dev::SCHEMA_VERSION,
                revision: "export-fixture".into(),
                providers: vec![models_dev::Provider {
                    id: "fixture".into(),
                    name: "Fixture".into(),
                    models: vec![models_dev::Model {
                        id: "wire".into(),
                        name: "Wire".into(),
                        limit: models_dev::Limit {
                            context: Some(128_000),
                            input: None,
                            output: Some(8_192),
                        },
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            models_dev::Origin::Embedded,
        )
        .unwrap();
        let mut store = router::Store {
            providers: vec![router::Provider {
                id: "p".into(),
                name: "Provider".into(),
                enabled: true,
                api_key: "upstream-secret".into(),
                base_url: "https://fixture.example/".into(),
                model: "wire".into(),
                protocol: ModelProtocol::ChatCompletions,
                models: vec![newapi::Model { id: "wire".into(), ..Default::default() }],
                ..Default::default()
            }],
            aggregates: vec![aggregation::AggregateModel {
                id: "pool".into(),
                public_model_id: "official-model".into(),
                display_name: "Official".into(),
                members: vec![aggregation::AggregateMember {
                    id: "m".into(),
                    provider_id: "p".into(),
                    actual_model_id: "wire".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            router: router::RouterSettings {
                bearer_token: "local-secret".into(),
                gateway_keys: vec![router::GatewayKey { id: "k".into(), name: "Agent".into(), key: "sk-ah-gateway-secret".into(), created_at: 0 }],
                ..Default::default()
            },
            ..Default::default()
        };
        crate::core::gateway::assign_slugs(&mut store);
        let sdk = export(&store, &catalog, ModelProtocol::AnthropicMessages, Some("pool/official-model"), "anthropicSdk", "k", true).unwrap();
        assert!(sdk.connection.base_url.ends_with("/gateway"));
        assert_eq!(sdk.connection.request_path, "/v1/messages");
        assert!(sdk.connection.models_path.unwrap().ends_with("?format=anthropic"));
        assert!(sdk.config_text.contains("sk-ah-gateway-secret"));
        assert!(!sdk.config_text.contains("upstream-secret") && !sdk.config_text.contains("local-secret"));
        let opencode = export(&store, &catalog, ModelProtocol::Responses, None, "opencode", "k", false).unwrap();
        assert!(opencode.connection.base_url.ends_with("/gateway/v1"));
        assert!(opencode.config_text.contains("@ai-sdk/openai"));
        assert!(!opencode.config_text.contains("sk-ah-gateway-secret"), "masked until revealed");
        let chat = export(&store, &catalog, ModelProtocol::ChatCompletions, Some("provider/wire"), "opencode", "k", true).unwrap();
        assert_eq!(chat.connection.request_path, "/chat/completions");
        assert_eq!(chat.connection.consumer_profile.models_format, ModelsFormat::OpenAi);
        let config: Value = serde_json::from_str(&chat.config_text).unwrap();
        assert_eq!(config["provider"]["agentharbor"]["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(config["provider"]["agentharbor"]["options"]["baseURL"], chat.connection.base_url);
        assert_eq!(config["model"], "agentharbor/provider/wire");
        let models = config["provider"]["agentharbor"]["models"].as_object().unwrap();
        assert!(models.contains_key("provider/wire") && models.contains_key("pool/official-model"), "relay models and aggregates");
        let generic = export(&store, &catalog, ModelProtocol::ChatCompletions, None, "generic", "k", false).unwrap();
        assert_eq!(generic.connection.request_path, "/chat/completions");
        assert!(export(&store, &catalog, ModelProtocol::ChatCompletions, None, "anthropicSdk", "k", true).is_err());
        assert!(export(&store, &catalog, ModelProtocol::ChatCompletions, None, "generic", "missing", true).is_err());
        assert!(export(&store, &catalog, ModelProtocol::ChatCompletions, Some("nobody/x"), "generic", "k", true).is_err(),
            "an unavailable choice is never replaced by another model");
    }
}
