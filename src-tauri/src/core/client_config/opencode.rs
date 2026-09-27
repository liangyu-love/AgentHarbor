use super::*;

pub(super) struct OpenCode;

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
    ) -> Result<Vec<Edit>, String> {
        let mut edits = Vec::new();
        let providers: Vec<_> = store
            .providers
            .iter()
            .filter(|p| {
                p.has_route_credentials()
                    && if request.options.provider_ids.is_empty() {
                        request.needs_router() || p.id == request.provider_id
                    } else {
                        request.options.provider_ids.contains(&p.id)
                    }
            })
            .collect();
        if !providers.iter().any(|p| p.id == request.provider_id)
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
                let mut ids: Vec<_> = provider
                    .models
                    .iter()
                    .filter(|m| m.included)
                    .map(|m| m.id.as_str())
                    .collect();
                ids.push(&provider.model);
                if provider.id == request.provider_id {
                    ids.push(&request.model);
                }
                for id in ids.into_iter().filter(|id| {
                    !id.is_empty()
                        && provider.supports_model(id)
                        && provider.protocol_for_model(id) == protocol
                }) {
                    let metadata = provider.models.iter().find(|m| m.id == id);
                    let selected = provider.id == request.provider_id && id == request.model;
                    let context = if selected {
                        request.context_window
                    } else {
                        metadata
                            .and_then(|m| m.context_window)
                            .or_else(|| {
                                if id == provider.model {
                                    provider.context_window
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| {
                                super::super::model_defaults::for_model(id).context_window
                            })
                    };
                    let output = if selected {
                        request.max_output_tokens
                    } else {
                        metadata
                            .and_then(|m| m.max_output_tokens)
                            .unwrap_or_else(|| {
                                super::super::model_defaults::for_model(id).max_output_tokens
                            })
                            .min(context)
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
                let (base, credential) = if request.needs_router() {
                    (
                        format!(
                            "http://127.0.0.1:{}/p/{}/v1",
                            store.router.port, provider.id
                        ),
                        &store.router.bearer_token,
                    )
                } else {
                    (native_base(provider, true)?, &provider.api_key)
                };
                // Own only generated leaves, so later user edits to headers, variants
                // and custom models remain independent of this configuration session.
                for (path, value) in [
                    (vec!["provider", &key, "name"], json!(provider.name)),
                    (vec!["provider", &key, "npm"], json!(npm)),
                    (vec!["provider", &key, "options", "baseURL"], json!(base)),
                    (
                        vec!["provider", &key, "options", "apiKey"],
                        json!(credential),
                    ),
                ] {
                    edits.push(edit(&path, value));
                }
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
        let selected = selected_provider(store, request)?;
        edits.push(edit(
            &["model"],
            json!(format!(
                "{}/{}",
                provider_key(selected, selected.protocol_for_model(&request.model)),
                request.model
            )),
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
