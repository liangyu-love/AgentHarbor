//! Additive Pi model providers. Authentication and Pi's native /model defaults
//! remain in auth.json/settings.json; only models.json is managed here.
use super::*;
use std::{collections::BTreeSet, ffi::OsString};

pub(super) struct Pi;

pub(super) fn models_path(home: &Path, override_dir: Option<OsString>) -> PathBuf {
    let root = override_dir
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let root = match root {
        Some(path) if path == Path::new("~") => home.to_owned(),
        Some(path) if path.starts_with("~/") || path.starts_with("~\\") => {
            home.join(path.strip_prefix("~").expect("tilde prefix"))
        }
        Some(path) if path.is_relative() => std::env::current_dir()
            .unwrap_or_else(|_| home.to_owned())
            .join(path),
        Some(path) => path,
        None => home.join(".pi/agent"),
    };
    root.join("models.json")
}

fn api(protocol: ModelProtocol) -> &'static str {
    match protocol {
        ModelProtocol::ChatCompletions => "openai-completions",
        ModelProtocol::Responses => "openai-responses",
        ModelProtocol::AnthropicMessages => "anthropic-messages",
    }
}

fn key(provider: &str, protocol: ModelProtocol) -> String {
    // Independent of the provider's default protocol and Pi's built-in auth IDs.
    format!("agentharbor-{provider}-{}", api(protocol))
}

/// Pi interprets leading ! as a shell command and $ as environment expansion.
/// Stored app credentials are literal values, never executable expressions.
fn literal(value: &str) -> String {
    let escaped = value.replace('$', "$$");
    if escaped.starts_with('!') {
        format!("${escaped}")
    } else {
        escaped
    }
}

fn model(
    id: &str,
    label: &str,
    projection: &super::super::model_policy::ClientProjection,
    request: &ConfigRequest,
    selected: bool,
    aggregate: bool,
) -> Result<Value, String> {
    validate_model(id)?;
    let context = if selected {
        request.context_window.unwrap_or(projection.context_window)
    } else {
        projection.context_window
    };
    let ceiling = projection.output_limit.unwrap_or(16_384).min(context);
    let output = if selected {
        request.max_output_tokens.unwrap_or(ceiling)
    } else {
        ceiling
    };
    if output == 0
        || output > context
        || (aggregate && (context > projection.context_window || output > ceiling))
    {
        return Err("clients.errors.limits".into());
    }
    Ok(json!({"id": id, "name": label, "contextWindow": context, "maxTokens": output}))
}

fn provider_edits(
    document: &Value,
    ctx: &EditContext<'_>,
    key: &str,
    name: &str,
    protocol: ModelProtocol,
    base: String,
    credential: &str,
    bearer: bool,
    models: Vec<Value>,
) -> Result<Vec<Edit>, String> {
    if let Some(existing) = document["providers"].get(key) {
        if !ctx.managed_provider_keys.iter().any(|id| id == key)
            && !existing.as_object().is_some_and(|object| object.is_empty())
        {
            return Err("clients.errors.piProviderConflict".into());
        }
        if !existing.is_object() {
            return Err("clients.errors.invalidConfig".into());
        }
    }
    // Only these leaves belong to this session. Other provider fields, foreign
    // providers and all top-level fields survive reapply and merged restore.
    let mut edits = vec![
        edit(&["providers", key, "name"], json!(name)),
        edit(&["providers", key, "api"], json!(api(protocol))),
        edit(&["providers", key, "baseUrl"], json!(base)),
        edit(&["providers", key, "apiKey"], json!(literal(credential))),
        edit(&["providers", key, "authHeader"], json!(bearer)),
        edit(&["providers", key, "models"], json!(models)),
    ];
    if protocol == ModelProtocol::ChatCompletions {
        // Native system role and max_tokens work on compatible relays and the
        // local gateway. Pi cannot infer these from a loopback/custom URL.
        for (field, value) in [
            ("supportsDeveloperRole", json!(false)),
            ("supportsStore", json!(false)),
            ("maxTokensField", json!("max_tokens")),
        ] {
            edits.push(edit(&["providers", key, "compat", field], value));
        }
    }
    Ok(edits)
}

impl ClientAdapter for Pi {
    fn id(&self) -> TargetId {
        TargetId::Pi
    }
    fn name(&self) -> &'static str {
        "Pi"
    }
    fn protocols(&self) -> &'static [ModelProtocol] {
        &[
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ]
    }
    fn path<'a>(&self, paths: &'a ClientPaths) -> &'a Path {
        &paths.pi
    }

    fn edits(
        &self,
        document: &Value,
        store: &Store,
        request: &ConfigRequest,
        catalog: &models_dev::Catalog,
        official_models: &[Value],
        ctx: &mut EditContext<'_>,
    ) -> Result<Vec<Edit>, String> {
        if document
            .get("providers")
            .is_some_and(|value| !value.is_object())
        {
            return Err("clients.errors.invalidConfig".into());
        }
        let providers: Vec<_> = store
            .providers
            .iter()
            .filter(|provider| {
                provider.has_route_credentials()
                    && if request.options.model_selection.is_some() {
                        request.options.provider_ids.contains(&provider.id)
                    } else if request.options.provider_ids.is_empty() {
                        request.needs_router() || provider.id == request.provider_id
                    } else {
                        request.options.provider_ids.contains(&provider.id)
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
        let mut edits = Vec::new();
        for provider in providers {
            let ids: BTreeSet<_> =
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
            for protocol in self.protocols() {
                let mut models = Vec::new();
                for id in ids
                    .iter()
                    .filter(|id| provider.protocol_for_model(id) == *protocol)
                {
                    let official =
                        super::super::router::matched_official_model(official_models, id)
                            .filter(|_| super::super::router::official_endpoint_applies(provider));
                    let (metadata, projection) = super::super::router::model_projection(
                        provider,
                        id,
                        official,
                        catalog,
                        &store.global_compression(),
                    );
                    let label = provider
                        .models
                        .iter()
                        .find(|m| &m.id == id)
                        .map(|m| m.display_name.as_str())
                        .filter(|name| !name.is_empty())
                        .unwrap_or(&metadata.display_name);
                    let mut entry = model(
                        id,
                        label,
                        &projection,
                        request,
                        provider.id == request.provider_id && id == &request.model,
                        false,
                    )?;
                    // Explicit effort levels are also a capability declaration:
                    // a manually configured, uncatalogued model must remain usable.
                    let reasoning = metadata
                        .reasoning
                        .levels
                        .value
                        .as_ref()
                        .filter(|levels| !levels.is_empty())
                        .map(|_| true)
                        .or(metadata.reasoning.supports);
                    if let Some(reasoning) = reasoning {
                        entry["reasoning"] = json!(reasoning);
                    }
                    if let Some(levels) = metadata
                        .reasoning
                        .levels
                        .value
                        .as_ref()
                        .filter(|levels| !levels.is_empty())
                    {
                        let mut map = serde_json::Map::new();
                        for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
                            let effort = levels.iter().find(|declared| {
                                declared.effort == level
                                    || (level == "off" && declared.effort == "none")
                            });
                            map.insert(
                                level.into(),
                                effort
                                    .map(|declared| json!(declared.effort))
                                    .unwrap_or(Value::Null),
                            );
                        }
                        entry["thinkingLevelMap"] = json!(map);
                    }
                    // A context-only Fast/preview match does not advertise vision.
                    if !metadata.steps.iter().any(|step| {
                        matches!(
                            step.step.as_str(),
                            "speedVariantReference" | "previewContextReference"
                        )
                    }) {
                        if let Some(reference) = metadata.matched.as_ref() {
                            if let Some(matched) =
                                catalog.exact_in(&reference.provider_id, &reference.model_id)
                            {
                                if let Some(modalities) = matched.model.modalities.as_ref() {
                                    entry["input"] =
                                        if modalities.input.iter().any(|input| input == "image") {
                                            json!(["text", "image"])
                                        } else {
                                            json!(["text"])
                                        };
                                }
                            }
                        }
                    }
                    let price = metadata.price;
                    if price.complete && price.currency == "USD" {
                        entry["cost"] = json!({"input": price.input.unwrap_or(0.0), "output": price.output.unwrap_or(0.0),
                            "cacheRead": price.cache_read.unwrap_or(0.0), "cacheWrite": price.cache_write.unwrap_or(0.0)});
                    }
                    if *protocol == ModelProtocol::ChatCompletions {
                        // Do not send an effort field unless metadata explicitly
                        // declares an effort control for this exact deployment.
                        entry["compat"] = json!({"supportsReasoningEffort": metadata.reasoning.levels.value.as_ref().is_some_and(|levels| !levels.is_empty())});
                    }
                    models.push(entry);
                }
                if models.is_empty() {
                    continue;
                }
                let (base, credential) = options::client_endpoint_for_protocol(
                    store,
                    provider,
                    request,
                    *protocol != ModelProtocol::AnthropicMessages,
                    *protocol,
                )?;
                let bearer = request.needs_router()
                    || *protocol != ModelProtocol::AnthropicMessages
                    || super::super::newapi::model_auth_header(
                        &provider.base_url,
                        *protocol,
                        credential,
                    )?
                    .0 == "authorization";
                edits.extend(provider_edits(
                    document,
                    ctx,
                    &key(&format!("relay-{}", provider.id), *protocol),
                    &provider.name,
                    *protocol,
                    base,
                    credential,
                    bearer,
                    models,
                )?);
            }
        }
        let aggregates = aggregate_models::models(store, request, catalog)?;
        if !aggregates.is_empty() {
            let protocol = request.aggregate_protocol();
            let mut models = Vec::new();
            for aggregate in aggregates {
                models.push(model(
                    &aggregate.id,
                    &aggregate.label,
                    &aggregate.projection,
                    request,
                    request.aggregate_target().is_some() && request.model == aggregate.id,
                    true,
                )?);
            }
            let (base, credential) = aggregate_models::endpoint(
                store,
                protocol != ModelProtocol::AnthropicMessages,
                TargetId::Pi,
            );
            edits.extend(provider_edits(
                document,
                ctx,
                &key("aggregation", protocol),
                "AgentHarbor Aggregation",
                protocol,
                base,
                credential,
                true,
                models,
            )?);
        }
        Ok(edits)
    }
}
