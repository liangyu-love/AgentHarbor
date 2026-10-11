//! Stable model selections shared by configuration adapters and connection export.
//!
//! A target references saved data; it never duplicates endpoint or credentials.
//! Resolving that target is separate from writing a client's configuration files.
use super::protocol::ModelProtocol;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const CONNECTION_EXPORT_SCHEMA_VERSION: u32 = 1;
const MAX_SELECTION_TARGETS: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ModelTargetRef {
    Provider {
        #[serde(rename = "providerId")]
        provider_id: String,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    Aggregate {
        #[serde(rename = "aggregateId")]
        aggregate_id: String,
    },
}

impl ModelTargetRef {
    pub fn provider(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self::Provider {
            provider_id: provider_id.into(),
            model_id: model_id.into(),
        }
    }

    pub fn aggregate(aggregate_id: impl Into<String>) -> Self {
        Self::Aggregate {
            aggregate_id: aggregate_id.into(),
        }
    }

    /// Validate the stored reference without normalizing its exact model ID.
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Provider {
                provider_id,
                model_id,
            } => {
                if !valid_text(provider_id, 128) || !valid_text(model_id, 256) {
                    return Err("modelTargets.errors.invalidTarget");
                }
            }
            Self::Aggregate { aggregate_id } => {
                if !valid_text(aggregate_id, 128) {
                    return Err("modelTargets.errors.invalidTarget");
                }
            }
        }
        Ok(())
    }
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

/// Multiple available targets with an optional default. OpenCode and agent
/// catalog exports retain their selected model subsets through this structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientModelSelection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_target: Option<ModelTargetRef>,
    pub available_targets: Vec<ModelTargetRef>,
}

impl ClientModelSelection {
    pub fn single(target: ModelTargetRef) -> Self {
        Self {
            default_target: Some(target.clone()),
            available_targets: vec![target],
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.available_targets.len() > MAX_SELECTION_TARGETS {
            return Err("modelTargets.errors.tooManyTargets");
        }
        let mut unique = HashSet::new();
        for target in &self.available_targets {
            target.validate()?;
            if !unique.insert(target) {
                return Err("modelTargets.errors.duplicateTarget");
            }
        }
        if let Some(default) = &self.default_target {
            default.validate()?;
            if !unique.contains(default) {
                return Err("modelTargets.errors.defaultUnavailable");
            }
        }
        Ok(())
    }
}

/// Public credentials are references to app-managed local tokens, never tokens
/// or provider keys. Actual configuration export resolves these in the backend.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LocalCredentialRef {
    Router,
    ClaudeDesktop,
    /// One of the gateway's named keys.
    Gateway,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ModelsFormat {
    OpenAi,
    Anthropic,
    InternalCatalog,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LocalAuthScheme {
    Bearer,
    ApiKey,
    AgentHarborHeader,
}

/// The consumer's protocol is independent of every aggregate member's egress.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerProfile {
    pub id: String,
    pub ingress_protocol: ModelProtocol,
    pub models_format: ModelsFormat,
}

/// Unknown support remains `None`; protocol names never imply hosted search.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientModelCapabilities {
    pub tools: Option<bool>,
    pub vision: Option<bool>,
    pub reasoning: Option<bool>,
    pub search: Option<bool>,
    pub one_m: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ClientModelLimits {
    pub context_window: Option<u64>,
    pub input_limit: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

/// Read-only public resolution. It chooses no member and contains no endpoint or
/// upstream key, so previews and saved client profiles cannot freeze a rotation.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedClientTarget {
    pub target_ref: ModelTargetRef,
    pub logical_model_id: String,
    pub display_name: String,
    pub ingress_protocol: ModelProtocol,
    pub projection: Option<super::model_policy::ClientProjection>,
    pub available: bool,
    pub reason: Option<String>,
    pub route_revision: String,
    pub catalog_revision: String,
    pub file_projection_revision: String,
}

impl ResolvedClientTarget {
    pub fn supports_one_m(&self) -> bool {
        self.available
            && self
                .projection
                .as_ref()
                .is_some_and(|p| !p.context_is_fallback && p.context_window >= 1_000_000)
    }
}

pub fn resolve_client_target(
    store: &super::router::Store,
    target: &ModelTargetRef,
    ingress: ModelProtocol,
    catalog: &super::models_dev::Catalog,
) -> Result<ResolvedClientTarget, String> {
    resolve_client_target_with_context(store, target, ingress, catalog, None)
}

/// Resolve only members able to serve the requested context. The regular
/// catalog remains conservative across the whole pool; a 1M menu item uses the
/// qualified subset and cannot advertise an unavailable fallback-sized member.
pub fn resolve_client_target_with_context(
    store: &super::router::Store,
    target: &ModelTargetRef,
    ingress: ModelProtocol,
    catalog: &super::models_dev::Catalog,
    minimum_context: Option<u64>,
) -> Result<ResolvedClientTarget, String> {
    target.validate().map_err(str::to_owned)?;
    use super::{aggregation, newapi, protocol::Translation, router};
    let (logical_model_id, display_name, projection, reason, route_revision) = match target {
        ModelTargetRef::Provider {
            provider_id,
            model_id,
        } => {
            let provider = store
                .providers
                .iter()
                .find(|p| p.id == *provider_id)
                .ok_or("modelTargets.errors.providerMissing")?;
            let protocol = provider.protocol_for_model(model_id);
            let credential = provider.model_credential().api_key;
            let (metadata, projection) = router::model_projection(
                provider,
                model_id,
                None,
                catalog,
                &store.global_compression(),
            );
            let reason = if !provider.enabled {
                Some("provider_disabled")
            } else if !provider.router_only() && credential.trim().is_empty() {
                Some("credential_missing")
            } else if newapi::provider_endpoint(&provider.base_url, "v1").is_err() {
                Some("endpoint_invalid")
            } else if !provider.router_only() && newapi::model_auth_header(&provider.base_url, protocol, credential).is_err() {
                Some("credential_invalid")
            } else if !provider.can_route_model(model_id)
                || provider
                    .models
                    .iter()
                    .find(|model| model.id == *model_id)
                    .is_some_and(|model| !model.included)
            {
                Some("model_unavailable")
            } else if !Translation::can_serve(ingress, protocol) {
                Some("protocol_unavailable")
            } else if minimum_context.is_some_and(|minimum| {
                projection.context_is_fallback || projection.context_window < minimum
            }) {
                Some("context_unavailable")
            } else {
                None
            };
            (
                model_id.clone(),
                metadata.display_name,
                Some(projection),
                reason.map(str::to_owned),
                aggregation::target_key(provider, model_id, protocol),
            )
        }
        ModelTargetRef::Aggregate { aggregate_id } => {
            let aggregate = store
                .aggregates
                .iter()
                .find(|a| a.id == *aggregate_id)
                .ok_or("modelTargets.errors.aggregateMissing")?;
            let mut compatible = aggregate.clone();
            compatible.members.retain(|member| {
                aggregation::resolve_member(store, aggregate, member, catalog)
                    .ok()
                    .is_some_and(|resolved| {
                        Translation::can_serve(ingress, resolved.protocol)
                            && (resolved.provider.router_only() || newapi::model_auth_header(
                                &resolved.provider.base_url,
                                resolved.protocol,
                                resolved.provider.model_credential().api_key,
                            )
                            .is_ok())
                            && minimum_context.is_none_or(|minimum| {
                                !resolved.projection.context_is_fallback
                                    && resolved.projection.context_window >= minimum
                            })
                    })
            });
            let view = aggregation::project(store, &compatible, catalog);
            (
                aggregate.public_model_id.clone(),
                if aggregate.display_name.is_empty() {
                    aggregate.public_model_id.clone()
                } else {
                    aggregate.display_name.clone()
                },
                view.projection,
                view.reason,
                aggregation::projection::route_revision(store, aggregate),
            )
        }
    };
    // Only client-visible fields contribute to the file revision. Routing
    // weights/credentials can change without moving an injector's baseline.
    let file_projection_revision = super::request_diagnostics::state_revision(
        &serde_json::to_vec(&(
            &logical_model_id,
            &display_name,
            ingress,
            &projection,
            &reason,
        ))
        .map_err(|_| "modelTargets.errors.invalidTarget")?,
    );
    Ok(ResolvedClientTarget {
        target_ref: target.clone(),
        logical_model_id,
        display_name,
        ingress_protocol: ingress,
        available: reason.is_none() && projection.is_some(),
        projection,
        reason,
        route_revision,
        catalog_revision: catalog.snapshot.revision.clone(),
        file_projection_revision,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportedModel {
    pub target_ref: ModelTargetRef,
    /// Exact identity exposed at this connection's model namespace.
    pub public_model_id: String,
    pub label: String,
    pub capabilities: ClientModelCapabilities,
    pub limits: ClientModelLimits,
}

/// Versioned, non-secret input for both managed injectors and generic templates.
/// Client adapters retain ownership, backups, and their file transactions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentConnectionExport {
    pub schema_version: u32,
    pub consumer_profile: ConsumerProfile,
    pub base_url: String,
    pub request_path: String,
    pub models_path: Option<String>,
    pub selected_model_id: Option<String>,
    pub model_catalog: Vec<ExportedModel>,
    pub auth_scheme: LocalAuthScheme,
    pub local_credential_ref: LocalCredentialRef,
    pub listener_ready: bool,
    pub runtime_requirement: String,
    pub config_revision: String,
    pub catalog_revision: String,
    pub token_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> super::super::models_dev::Catalog {
        use super::super::models_dev;
        models_dev::Catalog::from_snapshot(
            models_dev::Snapshot {
                schema_version: models_dev::SCHEMA_VERSION,
                revision: "model-target-test".into(),
                providers: vec![models_dev::Provider {
                    id: "fixture".into(),
                    name: "Fixture".into(),
                    models: vec![models_dev::Model {
                        id: "unrelated".into(),
                        name: "Unrelated".into(),
                        limit: models_dev::Limit {
                            context: Some(128_000),
                            input: None,
                            output: Some(8192),
                        },
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            models_dev::Origin::Embedded,
        )
        .unwrap()
    }

    fn provider(id: &str, protocol: ModelProtocol, context: u64) -> super::super::router::Provider {
        use super::super::{newapi, router};
        router::Provider {
            id: id.into(),
            name: format!("Provider {id}"),
            enabled: true,
            api_key: "fixture-upstream-secret".into(),
            base_url: format!("https://{id}.example/"),
            model: "actual-model".into(),
            protocol,
            models: vec![newapi::Model {
                id: "actual-model".into(),
                included: true,
                protocol: Some(protocol),
                overrides: newapi::ModelOverrides {
                    context_window: Some(context),
                    input_limit: Some(context - 8192),
                    max_output_tokens: Some(8192),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn public_aggregate_resolution_uses_conservative_projection_without_selecting_a_member() {
        use super::super::{aggregation, router};
        let store = router::Store {
            providers: vec![
                provider("a", ModelProtocol::ChatCompletions, 128_000),
                provider("b", ModelProtocol::Responses, 1_000_000),
            ],
            aggregates: vec![aggregation::AggregateModel {
                id: "pool".into(),
                public_model_id: "official-model".into(),
                display_name: "Official".into(),
                members: ["a", "b"]
                    .into_iter()
                    .map(|id| aggregation::AggregateMember {
                        id: format!("member-{id}"),
                        provider_id: id.into(),
                        actual_model_id: "actual-model".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let target = ModelTargetRef::aggregate("pool");
        let resolved = resolve_client_target(
            &store,
            &target,
            ModelProtocol::AnthropicMessages,
            &catalog(),
        )
        .unwrap();
        assert!(resolved.available);
        assert_eq!(resolved.logical_model_id, "official-model");
        assert_eq!(resolved.display_name, "Official");
        assert_eq!(
            resolved.projection.as_ref().unwrap().context_window,
            128_000
        );
        assert!(!resolved.supports_one_m());
        let encoded = serde_json::to_string(&resolved).unwrap();
        assert!(!encoded.contains("fixture-upstream-secret"));
        assert!(!encoded.contains("actual-model"));
        assert!(!encoded.contains("member-a"));
        let chat =
            resolve_client_target(&store, &target, ModelProtocol::ChatCompletions, &catalog())
                .unwrap();
        assert_eq!(chat.projection.as_ref().unwrap().context_window, 128_000);
        // Chat ingress sees the same mixed pool, including a Responses-only
        // member when the native Chat member is turned off.
        let mut responses_only = store.clone();
        responses_only.aggregates[0].members[0].enabled = false;
        let chat = resolve_client_target(
            &responses_only,
            &target,
            ModelProtocol::ChatCompletions,
            &catalog(),
        )
        .unwrap();
        assert!(chat.available);
        assert_eq!(chat.projection.unwrap().context_window, 1_000_000);
    }

    #[test]
    fn provider_resolution_retains_identity_and_reports_unavailable_without_key() {
        use super::super::router;
        let mut store = router::Store {
            providers: vec![provider("a", ModelProtocol::Responses, 1_000_000)],
            ..Default::default()
        };
        let target = ModelTargetRef::provider("a", "actual-model");
        let resolved =
            resolve_client_target(&store, &target, ModelProtocol::Responses, &catalog()).unwrap();
        assert_eq!(resolved.logical_model_id, "actual-model");
        assert!(resolved.supports_one_m());
        let original_revision = resolved.route_revision;
        store.providers[0].name = "A renamed card".into();
        store.providers[0].updated_at += 1;
        assert_eq!(
            resolve_client_target(&store, &target, ModelProtocol::Responses, &catalog())
                .unwrap()
                .route_revision,
            original_revision
        );
        store.providers[0].api_key.clear();
        let unavailable =
            resolve_client_target(&store, &target, ModelProtocol::Responses, &catalog()).unwrap();
        assert!(!unavailable.available);
        assert_eq!(unavailable.reason.as_deref(), Some("credential_missing"));
    }

    #[test]
    fn provider_resolution_rejects_an_explicitly_excluded_model() {
        let mut provider = provider("a", ModelProtocol::Responses, 1_000_000);
        provider.models[0].included = false;
        let store = super::super::router::Store {
            providers: vec![provider],
            ..Default::default()
        };
        let resolved = resolve_client_target(
            &store,
            &ModelTargetRef::provider("a", "actual-model"),
            ModelProtocol::Responses,
            &catalog(),
        )
        .unwrap();
        assert!(!resolved.available);
        assert_eq!(resolved.reason.as_deref(), Some("model_unavailable"));
    }

    #[test]
    fn one_m_resolution_qualifies_a_subset_without_inflating_the_normal_pool() {
        use super::super::{aggregation, router};
        let mut store = router::Store {
            providers: vec![
                provider("a", ModelProtocol::ChatCompletions, 128_000),
                provider("b", ModelProtocol::Responses, 1_000_000),
            ],
            aggregates: vec![aggregation::AggregateModel {
                id: "pool".into(),
                public_model_id: "official-model".into(),
                members: ["a", "b"]
                    .into_iter()
                    .map(|id| aggregation::AggregateMember {
                        id: format!("member-{id}"),
                        provider_id: id.into(),
                        actual_model_id: "actual-model".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let target = ModelTargetRef::aggregate("pool");
        let catalog = catalog();
        let ordinary =
            resolve_client_target(&store, &target, ModelProtocol::AnthropicMessages, &catalog)
                .unwrap();
        assert_eq!(ordinary.projection.unwrap().context_window, 128_000);
        let large = resolve_client_target_with_context(
            &store,
            &target,
            ModelProtocol::AnthropicMessages,
            &catalog,
            Some(1_000_000),
        )
        .unwrap();
        assert!(large.supports_one_m());
        assert_eq!(large.projection.unwrap().context_window, 1_000_000);
        store.providers[1].api_key = " \n ".into();
        let no_large = resolve_client_target_with_context(
            &store,
            &target,
            ModelProtocol::AnthropicMessages,
            &catalog,
            Some(1_000_000),
        )
        .unwrap();
        assert!(!no_large.available);
        assert!(no_large.projection.is_none());
        assert!(
            resolve_client_target(&store, &target, ModelProtocol::AnthropicMessages, &catalog)
                .unwrap()
                .available
        );
    }

    #[test]
    fn inference_credential_validation_does_not_substitute_the_balance_token() {
        use super::super::router;
        let mut store = router::Store {
            providers: vec![provider("a", ModelProtocol::Responses, 1_000_000)],
            ..Default::default()
        };
        let target = ModelTargetRef::provider("a", "actual-model");
        store.providers[0].access_token = "balance-only-token".into();
        for (key, reason) in [
            (" \t ", "credential_missing"),
            ("invalid key with spaces", "credential_invalid"),
        ] {
            store.providers[0].api_key = key.into();
            let resolved =
                resolve_client_target(&store, &target, ModelProtocol::Responses, &catalog())
                    .unwrap();
            assert!(!resolved.available);
            assert_eq!(resolved.reason.as_deref(), Some(reason));
        }
        store.providers[0].api_key = "Bearer fixture-token".into();
        assert!(
            resolve_client_target(&store, &target, ModelProtocol::Responses, &catalog())
                .unwrap()
                .available
        );
    }

    #[test]
    fn target_round_trip_keeps_provider_and_aggregate_identity_distinct() {
        for (target, wire) in [
            (
                ModelTargetRef::provider("saved-card", "platform/deepseek-fast"),
                json!({"kind":"provider", "providerId":"saved-card", "modelId":"platform/deepseek-fast"}),
            ),
            (
                ModelTargetRef::aggregate("aggregate-uuid"),
                json!({"kind":"aggregate", "aggregateId":"aggregate-uuid"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&target).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<ModelTargetRef>(wire).unwrap(),
                target
            );
            target.validate().unwrap();
        }
    }

    #[test]
    fn selection_rejects_duplicate_targets_and_default_outside_subset() {
        let first = ModelTargetRef::provider("card", "model-fast");
        let second = ModelTargetRef::aggregate("pool");
        let mut selection = ClientModelSelection {
            default_target: Some(first.clone()),
            available_targets: vec![first.clone(), second],
        };
        selection.validate().unwrap();
        selection.available_targets.push(first.clone());
        assert_eq!(
            selection.validate(),
            Err("modelTargets.errors.duplicateTarget")
        );
        selection.available_targets = vec![first];
        selection.default_target = Some(ModelTargetRef::provider("card", "other"));
        assert_eq!(
            selection.validate(),
            Err("modelTargets.errors.defaultUnavailable")
        );
    }

    #[test]
    fn model_ids_are_not_normalized_or_inferred() {
        let target = ModelTargetRef::provider("card", " vendor/model-fast ");
        target.validate().unwrap();
        assert_eq!(
            serde_json::to_value(target).unwrap()["modelId"],
            " vendor/model-fast "
        );
        assert!(ModelTargetRef::aggregate(" ").validate().is_err());
        assert!(ModelTargetRef::provider("card", "model\n")
            .validate()
            .is_err());
    }

    #[test]
    fn connection_export_contains_only_local_credential_reference() {
        let export = AgentConnectionExport {
            schema_version: CONNECTION_EXPORT_SCHEMA_VERSION,
            consumer_profile: ConsumerProfile {
                id: "claudeCode".into(),
                ingress_protocol: ModelProtocol::AnthropicMessages,
                models_format: ModelsFormat::InternalCatalog,
            },
            base_url: "http://127.0.0.1:43127/aggregate".into(),
            request_path: "/v1/messages".into(),
            models_path: None,
            selected_model_id: Some("official-model".into()),
            model_catalog: vec![ExportedModel {
                target_ref: ModelTargetRef::aggregate("pool"),
                public_model_id: "official-model".into(),
                label: "Official model".into(),
                capabilities: Default::default(),
                limits: Default::default(),
            }],
            auth_scheme: LocalAuthScheme::Bearer,
            local_credential_ref: LocalCredentialRef::Router,
            listener_ready: true,
            runtime_requirement: "localRouter".into(),
            config_revision: "file-projection".into(),
            catalog_revision: "catalog".into(),
            token_version: "token-version".into(),
        };
        let encoded = serde_json::to_value(export).unwrap();
        assert_eq!(encoded["localCredentialRef"], "router");
        assert_eq!(
            encoded["consumerProfile"]["ingressProtocol"],
            "anthropicMessages"
        );
        assert!(encoded.get("apiKey").is_none());
        assert!(encoded.get("token").is_none());
        assert!(encoded["modelCatalog"][0]["capabilities"]["search"].is_null());
    }
}
