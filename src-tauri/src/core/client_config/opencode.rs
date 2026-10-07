use super::*;

pub(super) struct OpenCode;

/// Shared resolver projection, so OpenCode limits match the router catalog.
fn projection_for(
    store: &Store,
    provider: &Provider,
    model: &str,
    catalog: &super::models_dev::Catalog,
    official_models: &[Value],
) -> super::super::model_policy::ClientProjection {
    let global = store.global_compression();
    let official = super::super::router::matched_official_model(official_models, model)
        .filter(|_| super::super::router::official_endpoint_applies(provider));
    super::super::router::model_projection(provider, model, official, catalog, &global).1
}

fn provider_key(provider: &Provider, protocol: ModelProtocol) -> String {
    let base = format!("agentharbor_{}", provider.id);
    if protocol == provider.protocol {
        base
    } else {
        let suffix = match protocol {
            ModelProtocol::Responses => "responses",
            ModelProtocol::ChatCompletions => "chat",
            ModelProtocol::AnthropicMessages => "anthropic",
        };
        format!("{base}_{suffix}")
    }
}

fn credential_edits(
    document: &Value,
    provider: &Provider,
    request: &ConfigRequest,
    protocol: ModelProtocol,
    key: &str,
    credential: &str,
) -> Result<Vec<Edit>, String> {
    let mut edits = Vec::new();
    let bearer = if !request.needs_router() && protocol == ModelProtocol::AnthropicMessages {
        let (header, value) =
            super::super::newapi::model_auth_header(&provider.base_url, protocol, credential)?;
        if header == "authorization" {
            Some(
                value
                    .to_str()
                    .map_err(|_| "newapi.errors.token")?
                    .strip_prefix("Bearer ")
                    .ok_or("newapi.errors.token")?
                    .to_owned(),
            )
        } else {
            None
        }
    } else {
        None
    };
    if let Some(token) = bearer {
        // OpenCode fills apiKey only when undefined. AI SDK Anthropic 3.0.111
        // accepts an empty key with authToken and emits Bearer without x-api-key;
        // keeping this explicit prevents a saved login from adding a second key.
        edits.push(edit(&["provider", key, "options", "apiKey"], json!("")));
        edits.push(edit(
            &["provider", key, "options", "authToken"],
            json!(token),
        ));
    } else {
        edits.push(edit(
            &["provider", key, "options", "apiKey"],
            json!(credential),
        ));
        edits.push(remove(&["provider", key, "options", "authToken"]));
    }
    // SDK headers override its authentication options. Withdraw only auth
    // headers, with field-level backups preserving the user's other headers.
    if let Some(headers) = document["provider"][key]["options"].get("headers") {
        for header in headers
            .as_object()
            .ok_or("clients.errors.invalidConfig")?
            .keys()
        {
            if header.eq_ignore_ascii_case("authorization")
                || header.eq_ignore_ascii_case("x-api-key")
            {
                edits.push(remove(&["provider", key, "options", "headers", header]));
            }
        }
    }
    Ok(edits)
}

impl ClientAdapter for OpenCode {
    fn id(&self) -> TargetId {
        TargetId::OpenCode
    }
    fn name(&self) -> &'static str {
        "OpenCode"
    }
    fn protocols(&self) -> &'static [ModelProtocol] {
        &[
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ]
    }
    fn path<'a>(&self, paths: &'a ClientPaths) -> &'a Path {
        &paths.opencode
    }

    fn edits(
        &self,
        document: &Value,
        store: &Store,
        request: &ConfigRequest,
        catalog: &super::models_dev::Catalog,
        official_models: &[Value],
        _ctx: &mut EditContext<'_>,
    ) -> Result<Vec<Edit>, String> {
        let mut edits = Vec::new();
        let providers: Vec<_> = store
            .providers
            .iter()
            .filter(|p| {
                p.has_route_credentials()
                    && if request.options.model_selection.is_some() {
                        request.options.provider_ids.contains(&p.id)
                    } else if request.options.provider_ids.is_empty() {
                        request.needs_router() || p.id == request.provider_id
                    } else {
                        request.options.provider_ids.contains(&p.id)
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
        let mut keys = Vec::new();
        for provider in providers {
            for protocol in [
                ModelProtocol::Responses,
                ModelProtocol::ChatCompletions,
                ModelProtocol::AnthropicMessages,
            ] {
                let key = provider_key(provider, protocol);
                let npm = match protocol {
                    ModelProtocol::Responses => "@ai-sdk/openai",
                    ModelProtocol::ChatCompletions => "@ai-sdk/openai-compatible",
                    ModelProtocol::AnthropicMessages => "@ai-sdk/anthropic",
                };
                let mut models = serde_json::Map::new();
                let configured_ids = request.options.provider_model_ids.get(&provider.id);
                let mut ids: Vec<_> = if let Some(configured_ids) = configured_ids {
                    configured_ids.iter().map(String::as_str).collect()
                } else {
                    provider
                        .models
                        .iter()
                        .filter(|m| m.included)
                        .map(|m| m.id.as_str())
                        .chain(std::iter::once(provider.model.as_str()))
                        .collect()
                };
                if provider.id == request.provider_id && configured_ids.is_none() {
                    ids.push(&request.model);
                }
                if let Some(configured_ids) = configured_ids {
                    if configured_ids.is_empty()
                        || configured_ids
                            .iter()
                            .any(|id| !request.accepts_model(provider, id))
                        || configured_ids.iter().any(|id| {
                            id != &provider.model
                                && !provider
                                    .models
                                    .iter()
                                    .any(|model| model.id == *id && model.included)
                        })
                        || (provider.id == request.provider_id
                            && !configured_ids.iter().any(|id| id == &request.model))
                    {
                        return Err("clients.errors.modelSelection".into());
                    }
                }
                for id in ids.into_iter().filter(|id| {
                    !id.is_empty()
                        && provider.supports_model(id)
                        && provider.protocol_for_model(id) == protocol
                }) {
                    let metadata = provider.models.iter().find(|m| m.id == id);
                    let selected = provider.id == request.provider_id && id == request.model;
                    let projection = projection_for(store, provider, id, catalog, official_models);
                    // The selected model honors explicit user input; every
                    // other model uses the same resolver projection as the
                    // router and Codex catalogs.
                    let context = if selected {
                        request.context_window.unwrap_or(projection.context_window)
                    } else {
                        projection.context_window
                    };
                    let output = if selected {
                        request
                            .max_output_tokens
                            .unwrap_or_else(|| projection.output_limit.unwrap_or(context))
                            .min(context)
                    } else {
                        projection.output_limit.unwrap_or(context).min(context)
                    };
                    models.insert(
                        id.to_owned(),
                        json!({"name": metadata.map(|model| model.display_name.as_str()).filter(|name| !name.is_empty()).unwrap_or(id), "limit": {"context": context, "output": output}}),
                    );
                }
                if models.is_empty() {
                    continue;
                }
                keys.push(json!(key));
                let (base, credential) = options::client_endpoint_for_protocol(
                    store, provider, request, true, protocol,
                )?;
                // Own only generated leaves, so later user edits to headers, variants
                // and custom models remain independent of this configuration session.
                for (path, value) in [
                    (vec!["provider", &key, "name"], json!(provider.name)),
                    (vec!["provider", &key, "npm"], json!(npm)),
                    (vec!["provider", &key, "options", "baseURL"], json!(base)),
                ] {
                    edits.push(edit(&path, value));
                }
                edits.extend(credential_edits(
                    document, provider, request, protocol, &key, credential,
                )?);
                for (id, model) in models {
                    edits.push(edit(
                        &["provider", &key, "models", &id, "name"],
                        model["name"].clone(),
                    ));
                    for limit in ["context", "output"] {
                        edits.push(edit(
                            &["provider", &key, "models", &id, "limit", limit],
                            model["limit"][limit].clone(),
                        ));
                    }
                }
            }
        }
        let aggregates = aggregate_models::models(store, request, catalog)?;
        if !aggregates.is_empty() {
            let protocol = request.aggregate_protocol();
            let key = aggregate_provider_key(protocol)?;
            let npm = match protocol {
                ModelProtocol::Responses => "@ai-sdk/openai",
                ModelProtocol::ChatCompletions => "@ai-sdk/openai-compatible",
                ModelProtocol::AnthropicMessages => "@ai-sdk/anthropic",
            };
            let (base, credential) = aggregate_models::endpoint(store, true, request.target_id);
            keys.push(json!(key));
            edits.extend([
                edit(&["provider", key, "name"], json!("AgentHarbor Aggregation")),
                edit(&["provider", key, "npm"], json!(npm)),
                edit(&["provider", key, "options", "baseURL"], json!(base)),
                edit(&["provider", key, "options", "apiKey"], json!(credential)),
                remove(&["provider", key, "options", "authToken"]),
            ]);
            if let Some(headers) = document["provider"][key]["options"].get("headers") {
                for header in headers
                    .as_object()
                    .ok_or("clients.errors.invalidConfig")?
                    .keys()
                {
                    if header.eq_ignore_ascii_case("authorization")
                        || header.eq_ignore_ascii_case("x-api-key")
                    {
                        edits.push(remove(&["provider", key, "options", "headers", header]));
                    }
                }
            }
            for model in aggregates {
                let selected = request.aggregate_target().is_some() && request.model == model.id;
                let context = if selected {
                    request
                        .context_window
                        .unwrap_or(model.projection.context_window)
                } else {
                    model.projection.context_window
                };
                let max_output = model
                    .projection
                    .output_limit
                    .unwrap_or(context)
                    .min(context);
                let output = if selected {
                    request.max_output_tokens.unwrap_or(max_output)
                } else {
                    max_output
                };
                if context > model.projection.context_window
                    || output > max_output
                    || output > context
                {
                    return Err("clients.errors.limits".into());
                }
                edits.extend([
                    edit(
                        &["provider", key, "models", &model.id, "name"],
                        json!(model.label),
                    ),
                    edit(
                        &["provider", key, "models", &model.id, "limit", "context"],
                        json!(context),
                    ),
                    edit(
                        &["provider", key, "models", &model.id, "limit", "output"],
                        json!(output),
                    ),
                ]);
            }
        }
        let default_key = if request.aggregate_target().is_some() {
            aggregate_provider_key(request.aggregate_protocol())?.to_owned()
        } else {
            let selected = selected_provider(store, request)?;
            provider_key(selected, selected.protocol_for_model(&request.model))
        };
        edits.push(edit(
            &["model"],
            json!(format!("{}/{}", default_key, request.model)),
        ));
        if let Some(enabled) = document.get("enabled_providers") {
            let mut enabled = enabled
                .as_array()
                .ok_or("clients.errors.invalidConfig")?
                .clone();
            for key in &keys {
                if !enabled.contains(key) {
                    enabled.push(key.clone());
                }
            }
            edits.push(edit(&["enabled_providers"], json!(enabled)));
        }
        if let Some(disabled) = document.get("disabled_providers") {
            let disabled = disabled.as_array().ok_or("clients.errors.invalidConfig")?;
            edits.push(edit(
                &["disabled_providers"],
                json!(disabled
                    .iter()
                    .filter(|id| !keys.contains(id))
                    .collect::<Vec<_>>()),
            ));
        }
        Ok(edits)
    }
}

fn aggregate_provider_key(protocol: ModelProtocol) -> Result<&'static str, String> {
    match protocol {
        ModelProtocol::Responses => Ok("agentharbor_aggregate_responses"),
        ModelProtocol::ChatCompletions => Ok("agentharbor_aggregate_chat"),
        ModelProtocol::AnthropicMessages => Ok("agentharbor_aggregate_anthropic"),
    }
}
