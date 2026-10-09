//! Non-secret aggregate catalog for the existing reversible client adapters.
use super::*;
use crate::core::model_targets::{self, ModelTargetRef};

#[derive(Clone)]
pub(super) struct AggregateClientModel {
    pub id: String,
    pub label: String,
    pub projection: crate::core::model_policy::ClientProjection,
    pub one_m: bool,
}

pub(super) fn canonical_request(
    store: &Store,
    request: &ConfigRequest,
) -> Result<ConfigRequest, String> {
    let mut result = request.clone();
    if let Some(selection) = &request.options.model_selection {
        selection
            .validate()
            .map_err(|_| "clients.errors.modelSelection")?;
        if selection
            .available_targets
            .iter()
            .any(|t| matches!(t, ModelTargetRef::Aggregate { .. }))
            && !request.needs_router()
        {
            return Err("clients.errors.protocol".into());
        }
        if matches!(request.target_id, TargetId::OpenCode | TargetId::Pi) {
            result.options.provider_ids.clear();
            result.options.provider_model_ids.clear();
            for target in &selection.available_targets {
                if let ModelTargetRef::Provider {
                    provider_id,
                    model_id,
                } = target
                {
                    if !result.options.provider_ids.contains(provider_id) {
                        result.options.provider_ids.push(provider_id.clone());
                    }
                    result
                        .options
                        .provider_model_ids
                        .entry(provider_id.clone())
                        .or_default()
                        .push(model_id.clone());
                }
            }
        }
        match selection.default_target.as_ref() {
            Some(ModelTargetRef::Provider {
                provider_id,
                model_id,
            }) => {
                result.provider_id = provider_id.clone();
                result.model = model_id.clone();
            }
            Some(ModelTargetRef::Aggregate { aggregate_id }) => {
                let aggregate = store
                    .aggregates
                    .iter()
                    .find(|a| &a.id == aggregate_id)
                    .ok_or("clients.errors.provider")?;
                result.provider_id.clear();
                result.model = aggregate.public_model_id.clone();
            }
            None => {}
        }
    }
    Ok(result)
}

pub(super) fn models(
    store: &Store,
    request: &ConfigRequest,
    catalog: &models_dev::Catalog,
) -> Result<Vec<AggregateClientModel>, String> {
    let protocol = request.aggregate_protocol();
    let Some(selection) = &request.options.model_selection else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    for target in &selection.available_targets {
        let ModelTargetRef::Aggregate { aggregate_id } = target else {
            continue;
        };
        let aggregate = store
            .aggregates
            .iter()
            .find(|a| &a.id == aggregate_id)
            .ok_or("clients.errors.provider")?;
        let resolved =
            crate::core::model_targets::resolve_client_target(store, target, protocol, catalog)?;
        if !resolved.available {
            return Err("clients.errors.provider".into());
        }
        let projection = resolved.projection.ok_or("clients.errors.provider")?;
        validate_model(&aggregate.public_model_id)?;
        // The client's marker requests a qualified subset of the pool. It is
        // independent of each member's egress protocol, just like Desktop.
        let one_m = model_targets::resolve_client_target_with_context(
            store,
            target,
            protocol,
            catalog,
            Some(1_000_000),
        )
        .is_ok_and(|target| target.supports_one_m());
        output.push(AggregateClientModel {
            id: resolved.logical_model_id,
            label: resolved.display_name,
            projection,
            one_m,
        });
    }
    Ok(output)
}

pub(super) fn endpoint<'a>(
    store: &'a Store,
    with_version: bool,
    target: TargetId,
) -> (String, &'a str) {
    let source = super::options::router_source_prefix(target);
    (
        format!(
            "http://127.0.0.1:{}{source}/aggregate{}",
            store.router.port,
            if with_version { "/v1" } else { "" }
        ),
        &store.router.bearer_token,
    )
}
