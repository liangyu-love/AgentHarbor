//! Claude Code combining sources: models of several relays and account
//! sources, and aggregates, behind one base URL. Every model goes by its id in
//! the gateway's id space (`relay-a/glm-5.3`, `pool/gpt-5.5`), which the
//! router's `/mix` namespace resolves with the client's own token.
use super::*;
use crate::core::model_targets::ModelTargetRef;

pub(super) struct MixedClientModel {
    pub id: String,
    pub label: String,
    /// The model's known context reaches 1M, or (aggregates) its pool does.
    pub one_m: bool,
}

/// Claude Code in router mode with models of more than one provider, or of
/// providers and aggregates together.
pub(super) fn is_mixed(request: &ConfigRequest) -> bool {
    if request.target_id != TargetId::ClaudeCode || !request.needs_router() {
        return false;
    }
    let Some(selection) = &request.options.model_selection else {
        return false;
    };
    let mut providers: Vec<&str> = selection
        .available_targets
        .iter()
        .filter_map(|target| match target {
            ModelTargetRef::Provider { provider_id, .. } => Some(provider_id.as_str()),
            _ => None,
        })
        .collect();
    providers.sort_unstable();
    providers.dedup();
    let aggregates = selection
        .available_targets
        .iter()
        .any(|target| matches!(target, ModelTargetRef::Aggregate { .. }));
    providers.len() > 1 || (!providers.is_empty() && aggregates)
}

/// A target's id in the gateway's id space.
pub(super) fn target_id(store: &Store, target: &ModelTargetRef) -> Result<String, String> {
    match target {
        ModelTargetRef::Provider {
            provider_id,
            model_id,
        } => {
            let provider = store
                .providers
                .iter()
                .find(|provider| provider.id == *provider_id && provider.has_route_credentials())
                .ok_or("clients.errors.provider")?;
            let slug = provider
                .gateway_slug
                .as_deref()
                .ok_or("clients.errors.provider")?;
            Ok(format!("{slug}/{model_id}"))
        }
        ModelTargetRef::Aggregate { aggregate_id } => {
            let aggregate = store
                .aggregates
                .iter()
                .find(|aggregate| aggregate.id == *aggregate_id)
                .ok_or("clients.errors.provider")?;
            Ok(crate::core::gateway::aggregate_id(store, aggregate))
        }
    }
}

/// The selected models in the order chosen, each checked the way a single
/// provider's or an aggregate's would be.
pub(super) fn models(
    store: &Store,
    request: &ConfigRequest,
    catalog: &models_dev::Catalog,
    official_models: &[Value],
) -> Result<Vec<MixedClientModel>, String> {
    let selection = request
        .options
        .model_selection
        .as_ref()
        .ok_or("clients.errors.modelSelection")?;
    let aggregates = super::aggregate_models::models(store, request, catalog)?;
    let mut output: Vec<MixedClientModel> = Vec::new();
    for target in &selection.available_targets {
        let id = target_id(store, target)?;
        validate_model(&id)?;
        if output.iter().any(|model| model.id == id) {
            continue;
        }
        let entry = match target {
            ModelTargetRef::Provider {
                provider_id,
                model_id,
            } => {
                let provider = store
                    .providers
                    .iter()
                    .find(|provider| provider.id == *provider_id)
                    .ok_or("clients.errors.provider")?;
                if !request.models(provider).contains(model_id) {
                    return Err("clients.errors.modelSelection".into());
                }
                let official =
                    crate::core::router::matched_official_model(official_models, model_id)
                        .filter(|_| crate::core::router::official_endpoint_applies(provider));
                let projection = crate::core::router::model_projection(
                    provider,
                    model_id,
                    official,
                    catalog,
                    &store.global_compression(),
                )
                .1;
                let label = provider
                    .models
                    .iter()
                    .find(|model| model.id == *model_id)
                    .map(|model| model.display_name.trim())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(model_id);
                MixedClientModel {
                    id,
                    label: format!("{} / {label}", provider.name),
                    one_m: projection.context_is_fallback || projection.context_window >= 1_000_000,
                }
            }
            ModelTargetRef::Aggregate { aggregate_id } => {
                let public_id = &store
                    .aggregates
                    .iter()
                    .find(|aggregate| aggregate.id == *aggregate_id)
                    .ok_or("clients.errors.provider")?
                    .public_model_id;
                let aggregate = aggregates
                    .iter()
                    .find(|model| model.id == *public_id)
                    .ok_or("clients.errors.provider")?;
                MixedClientModel {
                    id,
                    label: aggregate.label.clone(),
                    one_m: aggregate.one_m,
                }
            }
        };
        output.push(entry);
    }
    Ok(output)
}

/// The base URL and credential of the combined menu.
pub(super) fn endpoint(store: &Store) -> (String, &str) {
    (
        format!(
            "http://127.0.0.1:{}{}/mix",
            store.router.port,
            super::options::router_source_prefix(TargetId::ClaudeCode)
        ),
        &store.router.bearer_token,
    )
}
