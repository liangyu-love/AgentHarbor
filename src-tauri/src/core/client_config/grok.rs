//! Additive Grok Build custom models. Every relay model and aggregate is its
//! own `[model."<slug>/<id>"]` table in `$GROK_HOME/config.toml`, and
//! `[models] default` names the chosen one. Sign-in, xAI's own models and the
//! rest of the file stay Grok's.
use super::*;
use crate::core::model_targets::ModelTargetRef;
use std::collections::BTreeSet;

pub(super) struct Grok;

/// An environment variable nobody sets. A model naming it sends no key of its
/// own, and Grok then never sends the xAI sign-in to that model's endpoint.
const NO_BEARER: &str = "AGENTHARBOR_GROK_NO_BEARER";

fn backend(protocol: ModelProtocol) -> &'static str {
    match protocol {
        ModelProtocol::ChatCompletions => "chat_completions",
        ModelProtocol::Responses => "responses",
        ModelProtocol::AnthropicMessages => "messages",
    }
}

/// A relay model's key, in the gateway's id space (`relay-a/glm-5.3`). The
/// slash keeps it apart from xAI's own model ids, which a bare id would override.
fn provider_key(provider: &Provider, model: &str) -> String {
    let slug = provider.gateway_slug.as_deref().unwrap_or(&provider.id);
    format!("{slug}/{model}")
}

/// Picking one of this session's models as Grok's default (its picker writes
/// `models.default`) is normal client activity, not an ownership conflict.
pub(super) fn is_default_selection(document: &Value, session: &Session, path: &[String]) -> bool {
    if session.request.target_id != TargetId::Grok || path != ["models", "default"] {
        return false;
    }
    document
        .pointer("/models/default")
        .and_then(Value::as_str)
        .is_some_and(|current| {
            session
                .changes
                .iter()
                .any(|change| change.path.len() == 3 && change.path[0] == "model" && change.path[1] == current)
        })
}

/// Grok's effort menu: each declared spelling, sent as written.
fn efforts(facts: &model_facts::ModelFacts) -> Value {
    json!(facts
        .efforts
        .iter()
        .map(|effort| {
            let mut label = effort.clone();
            if let Some(first) = label.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            json!({
                "id": effort,
                "value": effort,
                "label": label,
                "default": facts.default_effort.as_deref().is_some_and(|default| default.eq_ignore_ascii_case(effort)),
            })
        })
        .collect::<Vec<_>>())
}

struct Entry {
    key: String,
    model: String,
    name: String,
    protocol: ModelProtocol,
    base: String,
    credential: String,
    /// The endpoint takes the key as `x-api-key`, never as a bearer token.
    x_api_key: bool,
    limits: Value,
    facts: Option<model_facts::ModelFacts>,
}

fn entry_edits(document: &Value, ctx: &EditContext<'_>, entry: Entry) -> Result<Vec<Edit>, String> {
    let key = entry.key.as_str();
    if let Some(existing) = document["model"].get(key) {
        if !ctx.managed_provider_keys.iter().any(|id| id == key)
            && !existing.as_object().is_some_and(|object| object.is_empty())
        {
            return Err("clients.errors.grokModelConflict".into());
        }
        if !existing.is_object() {
            return Err("clients.errors.invalidConfig".into());
        }
    }
    // Only these leaves belong to this session; any other field the user adds
    // to the table (temperature, notice, ...) survives reapply and restore.
    let at = |field: &'static str| ["model", key, field];
    let mut edits = vec![
        edit(&at("model"), json!(entry.model)),
        edit(&at("name"), json!(entry.name)),
        edit(&at("base_url"), json!(entry.base)),
        edit(&at("api_backend"), json!(backend(entry.protocol))),
        edit(&at("context_window"), entry.limits["contextWindow"].clone()),
        edit(&at("max_completion_tokens"), entry.limits["maxTokens"].clone()),
    ];
    if entry.x_api_key {
        edits.push(edit(&at("env_key"), json!(NO_BEARER)));
        edits.push(edit(&at("extra_headers"), json!({"x-api-key": entry.credential})));
    } else {
        edits.push(edit(&at("api_key"), json!(entry.credential)));
    }
    if let Some(facts) = entry.facts {
        if facts.efforts.is_empty() {
            // Without it the model would inherit a same-named catalog entry's
            // effort menu and send a field the deployment never declared.
            edits.push(edit(&at("supports_reasoning_effort"), json!(false)));
        } else {
            edits.push(edit(&at("reasoning_efforts"), efforts(&facts)));
        }
        if let Some(image) = facts.image_input {
            edits.push(edit(&at("vision"), json!(image)));
        }
    }
    Ok(edits)
}

impl ClientAdapter for Grok {
    fn id(&self) -> TargetId {
        TargetId::Grok
    }
    fn name(&self) -> &'static str {
        "Grok Build"
    }
    fn protocols(&self) -> &'static [ModelProtocol] {
        &[
            ModelProtocol::Responses,
            ModelProtocol::ChatCompletions,
            ModelProtocol::AnthropicMessages,
        ]
    }
    fn path<'a>(&self, paths: &'a ClientPaths) -> &'a Path {
        &paths.grok
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
        if ["model", "models"]
            .iter()
            .any(|key| document.get(key).is_some_and(|value| !value.is_object()))
        {
            return Err("clients.errors.invalidConfig".into());
        }
        let providers: Vec<_> = store
            .providers
            .iter()
            .filter(|provider| {
                provider.has_route_credentials()
                    && if request.options.model_selection.is_some() || !request.options.provider_ids.is_empty() {
                        request.options.provider_ids.contains(&provider.id)
                    } else {
                        request.needs_router() || provider.id == request.provider_id
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
        let aggregates = aggregate_models::models(store, request, catalog)?;
        // Names stay bare unless the picker mixes sources.
        let mixed = providers.len() > 1 || (!providers.is_empty() && !aggregates.is_empty());
        let mut entries = Vec::new();
        for provider in &providers {
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
            for id in ids {
                let protocol = provider.protocol_for_model(&id);
                let facts = model_facts::facts(store, provider, &id, catalog, official_models);
                let limits = pi::model(
                    &id,
                    &facts.label,
                    &facts.projection,
                    request,
                    provider.id == request.provider_id && id == request.model,
                    false,
                )?;
                let (base, credential) =
                    options::client_endpoint_for_protocol(store, provider, request, true, protocol)?;
                let x_api_key = !request.needs_router()
                    && super::super::newapi::model_auth_header(&provider.base_url, protocol, credential)?.0
                        == "x-api-key";
                entries.push(Entry {
                    key: provider_key(provider, &id),
                    name: if mixed { format!("{} / {}", provider.name, facts.label) } else { facts.label.clone() },
                    model: id,
                    protocol,
                    base,
                    credential: credential.to_owned(),
                    x_api_key,
                    limits,
                    facts: Some(facts),
                });
            }
        }
        if !aggregates.is_empty() {
            let protocol = request.aggregate_protocol();
            let (base, credential) = aggregate_models::endpoint(store, true, TargetId::Grok);
            let targets = request
                .options
                .model_selection
                .iter()
                .flat_map(|selection| &selection.available_targets)
                .filter(|target| matches!(target, ModelTargetRef::Aggregate { .. }));
            for (target, aggregate) in targets.zip(aggregates) {
                let limits = pi::model(
                    &aggregate.id,
                    &aggregate.label,
                    &aggregate.projection,
                    request,
                    request.aggregate_target().is_some() && request.model == aggregate.id,
                    true,
                )?;
                entries.push(Entry {
                    key: mixed_models::target_id(store, target)?,
                    model: aggregate.id,
                    name: aggregate.label,
                    protocol,
                    base: base.clone(),
                    credential: credential.to_owned(),
                    x_api_key: false,
                    limits,
                    facts: None,
                });
            }
        }
        let default = match request.aggregate_target() {
            Some(aggregate_id) => mixed_models::target_id(
                store,
                &ModelTargetRef::Aggregate { aggregate_id: aggregate_id.to_owned() },
            )?,
            None => provider_key(
                providers
                    .iter()
                    .find(|provider| provider.id == request.provider_id)
                    .ok_or("clients.errors.provider")?,
                &request.model,
            ),
        };
        if !entries.iter().any(|entry| entry.key == default) {
            return Err("clients.errors.modelSelection".into());
        }
        let mut edits = vec![edit(&["models", "default"], json!(default))];
        for entry in entries {
            edits.extend(entry_edits(document, ctx, entry)?);
        }
        Ok(edits)
    }
}
