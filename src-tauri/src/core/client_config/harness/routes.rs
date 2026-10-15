//! The provider routes, credentials and default model one request puts into
//! DeepSeek Harness. Harness runs these routes through pi-ai, so a route
//! speaks one protocol (`api`) and its models carry pi-ai's limits and
//! thinking levels, exactly as Pi's do.
use super::super::*;
use super::{AGGREGATION_NAME, ROUTER_CREDENTIAL};
use std::collections::BTreeSet;

pub(in super::super) const PROTOCOLS: [ModelProtocol; 3] = [
    ModelProtocol::Responses,
    ModelProtocol::ChatCompletions,
    ModelProtocol::AnthropicMessages,
];

pub(super) struct Route {
    pub key: String,
    pub value: Value,
}

#[derive(Default)]
pub(super) struct Routes {
    pub routes: Vec<Route>,
    /// Credential reference names and the values they hold.
    pub credentials: Vec<(String, String)>,
    /// The route and model new sessions start on.
    pub default: Option<(String, String)>,
}

/// A route key Harness accepts: lowercase and hyphenated. A provider id that
/// does not already read that way keeps a short hash, so two relays whose ids
/// differ only in punctuation stay two routes.
pub(super) fn route_key(provider_id: &str, protocol: ModelProtocol) -> String {
    format!(
        "agentharbor-{}-{}",
        slug(provider_id),
        super::super::pi::api(protocol)
    )
}

pub(super) fn aggregation_key(protocol: ModelProtocol) -> String {
    format!(
        "agentharbor-aggregation-{}",
        super::super::pi::api(protocol)
    )
}

fn slug(id: &str) -> String {
    let mut slug = String::new();
    for c in id.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-').to_owned();
    if slug == id && !slug.is_empty() {
        slug
    } else {
        format!(
            "{}{}{}",
            slug,
            if slug.is_empty() { "" } else { "-" },
            &short_hash(id)[..8]
        )
    }
}

/// The credential reference a direct relay's key is stored under.
pub(super) fn relay_credential(provider_id: &str) -> String {
    let name: String = provider_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "AGENTHARBOR_RELAY_{}_{}",
        name.trim_matches('_'),
        &short_hash(provider_id)[..8].to_ascii_uppercase()
    )
}

fn short_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn protocol_label(protocol: ModelProtocol) -> &'static str {
    match protocol {
        ModelProtocol::Responses => "Responses",
        ModelProtocol::ChatCompletions => "Chat",
        ModelProtocol::AnthropicMessages => "Messages",
    }
}

/// Route-level wire switches. Through relays and the local router, Chat
/// Completions takes the system role and `max_tokens` as they are; pi-ai
/// cannot infer that from a loopback or custom URL.
fn route_compat(protocol: ModelProtocol) -> Option<Value> {
    (protocol == ModelProtocol::ChatCompletions)
        .then(|| json!({"supportsDeveloperRole": false, "supportsStore": false, "maxTokensField": "max_tokens"}))
}

fn route_value(
    name: &str,
    protocol: ModelProtocol,
    base: String,
    credential: &str,
    models: Vec<Value>,
) -> Value {
    let mut value = json!({
        "displayName": name,
        "api": super::super::pi::api(protocol),
        "baseURL": base,
        "apiKeyEnv": credential,
        "models": models,
    });
    if let Some(compat) = route_compat(protocol) {
        value["compat"] = compat;
    }
    value
}

pub(super) fn build(
    store: &Store,
    request: &ConfigRequest,
    catalog: &models_dev::Catalog,
    official_models: &[Value],
) -> Result<Routes, String> {
    let providers: Vec<_> = store
        .providers
        .iter()
        .filter(|provider| {
            provider.has_route_credentials()
                && if request.options.model_selection.is_some()
                    || !request.options.provider_ids.is_empty()
                {
                    request.options.provider_ids.contains(&provider.id)
                } else {
                    provider.id == request.provider_id
                }
        })
        .collect();
    if (request.aggregate_target().is_none()
        && !providers.iter().any(|p| p.id == request.provider_id))
        || request
            .options
            .provider_ids
            .iter()
            .any(|id| !providers.iter().any(|p| &p.id == id))
    {
        return Err("clients.errors.provider".into());
    }
    let mut output = Routes::default();
    let mut credential = |name: String, value: &str| {
        if !output
            .credentials
            .iter()
            .any(|(existing, _)| *existing == name)
        {
            output.credentials.push((name.clone(), value.to_owned()));
        }
        name
    };
    let mut routes = Vec::new();
    for provider in providers {
        let ids: BTreeSet<String> =
            if let Some(ids) = request.options.provider_model_ids.get(&provider.id) {
                if ids.is_empty()
                    || ids.iter().any(|id| {
                        !request.accepts_model(provider, id)
                            || (id != &provider.model
                                && !provider.models.iter().any(|m| m.id == *id && m.included))
                    })
                    || (provider.id == request.provider_id && !ids.contains(&request.model))
                {
                    return Err("clients.errors.modelSelection".into());
                }
                ids.iter().cloned().collect()
            } else {
                request.models(provider).into_iter().collect()
            };
        let mut provider_routes = Vec::new();
        for protocol in PROTOCOLS {
            let mut models = Vec::new();
            for id in ids
                .iter()
                .filter(|id| provider.protocol_for_model(id) == protocol)
            {
                let facts =
                    super::super::model_facts::facts(store, provider, id, catalog, official_models);
                let mut entry = super::super::pi::model(
                    id,
                    &facts.label,
                    &facts.projection,
                    request,
                    provider.id == request.provider_id && id == &request.model,
                    false,
                )?;
                if let Some(image) = facts.image_input {
                    entry["input"] = if image {
                        json!(["text", "image"])
                    } else {
                        json!(["text"])
                    };
                }
                if facts.reasoning == Some(false) {
                    entry["reasoningEfforts"] = json!(false);
                } else {
                    // Only levels the deployment declares are offered, each
                    // sent with its declared spelling.
                    let mut levels = serde_json::Map::new();
                    for level in super::super::model_facts::THINKING_LEVELS {
                        if let Some(effort) = facts.effort_for(level) {
                            levels.insert(level.into(), json!(effort));
                        }
                    }
                    if !levels.is_empty() {
                        entry["reasoningEfforts"] = Value::Object(levels);
                    }
                }
                if protocol == ModelProtocol::ChatCompletions {
                    // No effort field unless this exact deployment declares one.
                    entry["compat"] = json!({"supportsReasoningEffort": !facts.efforts.is_empty()});
                }
                models.push(entry);
            }
            if models.is_empty() {
                continue;
            }
            let (base, key) = options::client_endpoint_for_protocol(
                store,
                provider,
                request,
                protocol != ModelProtocol::AnthropicMessages,
                protocol,
            )?;
            // pi-ai sends an Anthropic key as `x-api-key`; a relay that only
            // takes a bearer token is reached through the router instead.
            if !request.needs_router()
                && protocol == ModelProtocol::AnthropicMessages
                && super::super::super::newapi::model_auth_header(
                    &provider.base_url,
                    protocol,
                    key,
                )?
                .0 == "authorization"
            {
                return Err("clients.errors.harnessAnthropicBearer".into());
            }
            let reference = if request.needs_router() {
                credential(ROUTER_CREDENTIAL.into(), key)
            } else {
                credential(relay_credential(&provider.id), key)
            };
            provider_routes.push((protocol, base, reference, models));
        }
        let several = provider_routes.len() > 1;
        for (protocol, base, reference, models) in provider_routes {
            let name = if several {
                format!("{} · {}", provider.name, protocol_label(protocol))
            } else {
                provider.name.clone()
            };
            routes.push(Route {
                key: route_key(&provider.id, protocol),
                value: route_value(&name, protocol, base, &reference, models),
            });
        }
    }
    let aggregates = aggregate_models::models(store, request, catalog)?;
    if !aggregates.is_empty() {
        let protocol = request.aggregate_protocol();
        let mut models = Vec::new();
        for aggregate in aggregates {
            models.push(super::super::pi::model(
                &aggregate.id,
                &aggregate.label,
                &aggregate.projection,
                request,
                request.aggregate_target().is_some() && request.model == aggregate.id,
                true,
            )?);
        }
        let (base, key) = aggregate_models::endpoint(
            store,
            protocol != ModelProtocol::AnthropicMessages,
            TargetId::Harness,
        );
        let reference = credential(ROUTER_CREDENTIAL.into(), key);
        routes.push(Route {
            key: aggregation_key(protocol),
            value: route_value(AGGREGATION_NAME, protocol, base, &reference, models),
        });
    }
    let mut keys = BTreeSet::new();
    if routes.iter().any(|route| !keys.insert(route.key.clone())) {
        return Err("clients.errors.harnessProviderConflict".into());
    }
    if request.options.harness_default_model != Some(false) {
        let key = match request.aggregate_target() {
            Some(_) => aggregation_key(request.aggregate_protocol()),
            None => {
                let provider = selected_provider(store, request)?;
                route_key(&provider.id, provider.protocol_for_model(&request.model))
            }
        };
        let serves = routes.iter().any(|route| {
            route.key == key
                && route.value["models"].as_array().is_some_and(|models| {
                    models
                        .iter()
                        .any(|model| model["id"] == request.model.as_str())
                })
        });
        if !serves {
            return Err("clients.errors.modelSelection".into());
        }
        output.default = Some((key, request.model.clone()));
    }
    output.routes = routes;
    Ok(output)
}
