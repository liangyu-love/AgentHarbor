use super::{model_targets::*, models_dev, protocol::ModelProtocol, router};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportBundle {
    pub connection: AgentConnectionExport,
    pub config_text: String,
}

pub fn export(
    store: &router::Store,
    catalog: &models_dev::Catalog,
    ingress: ModelProtocol,
    ids: &[String],
    selected_id: Option<&str>,
    profile: &str,
    reveal: bool,
) -> Result<ExportBundle, String> {
    if !matches!(profile, "generic" | "anthropicSdk" | "opencode")
        || (profile == "anthropicSdk" && ingress != ModelProtocol::AnthropicMessages)
    {
        return Err("aggregation.errors.consumerUnsupported".into());
    }
    let unique: HashSet<_> = ids.iter().collect();
    if unique.len() != ids.len() || ids.len() > super::aggregation::MAX_AGGREGATES {
        return Err("aggregation.errors.invalidSelection".into());
    }
    let mut models = Vec::new();
    for aggregate in store
        .aggregates
        .iter()
        .filter(|aggregate| ids.is_empty() || ids.contains(&aggregate.id))
    {
        let target = ModelTargetRef::aggregate(&aggregate.id);
        let resolved = resolve_client_target(store, &target, ingress, catalog)?;
        if !resolved.available {
            continue;
        }
        let projection = resolved
            .projection
            .as_ref()
            .ok_or("aggregation.errors.unavailable")?;
        models.push(ExportedModel {
            target_ref: target,
            public_model_id: aggregate.public_model_id.clone(),
            label: resolved.display_name.clone(),
            capabilities: ClientModelCapabilities {
                one_m: Some(resolved.supports_one_m()),
                ..Default::default()
            },
            limits: ClientModelLimits {
                context_window: Some(projection.context_window),
                input_limit: projection.input_limit,
                max_output_tokens: projection.output_limit,
            },
        });
    }
    if models.is_empty()
        || ids
            .iter()
            .any(|id| !store.aggregates.iter().any(|aggregate| aggregate.id == *id))
    {
        return Err("aggregation.errors.unavailable".into());
    }
    let selected = if let Some(id) = selected_id {
        models
            .iter()
            .find(|model| model.target_ref == ModelTargetRef::aggregate(id))
            .ok_or("aggregation.errors.unavailable")?
    } else {
        &models[0]
    };
    let root = format!("http://127.0.0.1:{}/aggregate", store.router.port);
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
    let token_version =
        super::request_diagnostics::state_revision(store.router.bearer_token.as_bytes());
    let config_revision = super::request_diagnostics::state_revision(
        &serde_json::to_vec(&(
            &base_url,
            &models,
            &selected.public_model_id,
            &token_version,
        ))
        .map_err(|_| "aggregation.errors.storage")?,
    );
    let connection = AgentConnectionExport {
        schema_version: CONNECTION_EXPORT_SCHEMA_VERSION,
        consumer_profile: ConsumerProfile {
            id: profile.into(),
            ingress_protocol: ingress,
            models_format: if ingress == ModelProtocol::AnthropicMessages {
                ModelsFormat::Anthropic
            } else {
                ModelsFormat::OpenAi
            },
        },
        base_url: base_url.clone(),
        request_path: request_path.into(),
        models_path: Some(format!(
            "{root}/v1/models{}",
            if ingress == ModelProtocol::AnthropicMessages {
                "?format=anthropic"
            } else {
                ""
            }
        )),
        selected_model_id: Some(selected.public_model_id.clone()),
        model_catalog: models.clone(),
        auth_scheme: LocalAuthScheme::Bearer,
        local_credential_ref: LocalCredentialRef::Router,
        listener_ready: super::router_proxy::is_running(),
        runtime_requirement: "localRouter".into(),
        config_revision,
        catalog_revision: catalog.snapshot.revision.clone(),
        token_version,
    };
    let token = if reveal {
        &store.router.bearer_token
    } else {
        "<local router token>"
    };
    let config = if profile == "opencode" {
        let (group, npm) = match ingress {
            ModelProtocol::Responses => ("agentharbor_aggregate_responses", "@ai-sdk/openai"),
            ModelProtocol::ChatCompletions => {
                ("agentharbor_aggregate_chat", "@ai-sdk/openai-compatible")
            }
            ModelProtocol::AnthropicMessages => {
                ("agentharbor_aggregate_messages", "@ai-sdk/anthropic")
            }
        };
        let model_entries: serde_json::Map<String, Value> = models.iter().map(|model| {
            (model.public_model_id.clone(), json!({"name":model.label,"limit":{"context":model.limits.context_window,"output":model.limits.max_output_tokens}}))
        }).collect();
        json!({"provider":{(group):{"name":"AgentHarbor","npm":npm,
            "options":{"baseURL":base_url,"apiKey":token},"models":model_entries}},"model":format!("{group}/{}",selected.public_model_id)})
    } else if profile == "anthropicSdk" {
        json!({"base_url":base_url,"api_key":token,"model":selected.public_model_id})
    } else {
        json!({"base_url":base_url,"request_path":request_path,"model":selected.public_model_id,"headers":{"Authorization":format!("Bearer {token}")}})
    };
    Ok(ExportBundle {
        connection,
        config_text: serde_json::to_string_pretty(&config)
            .map_err(|_| "aggregation.errors.storage")?,
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
        let store = router::Store {
            providers: vec![router::Provider {
                id: "p".into(),
                name: "Provider".into(),
                enabled: true,
                api_key: "upstream-secret".into(),
                base_url: "https://fixture.example/".into(),
                model: "wire".into(),
                protocol: ModelProtocol::ChatCompletions,
                models: vec![newapi::Model {
                    id: "wire".into(),
                    ..Default::default()
                }],
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
                ..Default::default()
            },
            ..Default::default()
        };
        let sdk = export(
            &store,
            &catalog,
            ModelProtocol::AnthropicMessages,
            &[],
            None,
            "anthropicSdk",
            true,
        )
        .unwrap();
        assert!(sdk.connection.base_url.ends_with("/aggregate"));
        assert_eq!(sdk.connection.request_path, "/v1/messages");
        assert!(sdk
            .connection
            .models_path
            .unwrap()
            .ends_with("?format=anthropic"));
        assert!(sdk.config_text.contains("local-secret"));
        assert!(!sdk.config_text.contains("upstream-secret"));
        let opencode = export(
            &store,
            &catalog,
            ModelProtocol::Responses,
            &[],
            None,
            "opencode",
            false,
        )
        .unwrap();
        assert!(opencode.connection.base_url.ends_with("/aggregate/v1"));
        assert!(opencode.config_text.contains("@ai-sdk/openai"));
        assert!(!opencode.config_text.contains("local-secret"));
        let chat = export(
            &store,
            &catalog,
            ModelProtocol::ChatCompletions,
            &[],
            None,
            "opencode",
            true,
        )
        .unwrap();
        assert_eq!(chat.connection.request_path, "/chat/completions");
        assert_eq!(
            chat.connection.consumer_profile.models_format,
            ModelsFormat::OpenAi
        );
        let config: Value = serde_json::from_str(&chat.config_text).unwrap();
        assert_eq!(
            config["provider"]["agentharbor_aggregate_chat"]["npm"],
            "@ai-sdk/openai-compatible"
        );
        assert_eq!(
            config["provider"]["agentharbor_aggregate_chat"]["options"]["baseURL"],
            chat.connection.base_url
        );
        assert_eq!(config["model"], "agentharbor_aggregate_chat/official-model");
        assert!(config["provider"]["agentharbor_aggregate_chat"]["models"]
            .get("official-model")
            .is_some());
        assert!(!chat.config_text.contains("upstream-secret"));
        let generic = export(
            &store,
            &catalog,
            ModelProtocol::ChatCompletions,
            &[],
            None,
            "generic",
            false,
        )
        .unwrap();
        assert_eq!(generic.connection.request_path, "/chat/completions");
        assert!(!generic.config_text.contains("local-secret"));
        assert!(export(
            &store,
            &catalog,
            ModelProtocol::ChatCompletions,
            &[],
            None,
            "anthropicSdk",
            true
        )
        .is_err());
    }
}
