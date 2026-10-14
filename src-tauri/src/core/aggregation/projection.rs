use super::types::*;
use crate::core::{model_metadata, model_policy, models_dev, newapi, router};
use sha2::{Digest, Sha256};

/// Internal execution snapshot. Never serialize this structure: Provider holds
/// the protected upstream credential, which is absent from every public view.
#[derive(Clone)]
pub struct ResolvedMember {
    pub member: AggregateMember,
    pub provider: router::Provider,
    pub protocol: router::ModelProtocol,
    pub metadata: model_metadata::ResolvedMetadata,
    pub projection: model_policy::ClientProjection,
    pub target_key: String,
    pub tools: Option<bool>,
    pub vision: Option<bool>,
}

pub fn target_key(
    provider: &router::Provider,
    actual_model: &str,
    protocol: router::ModelProtocol,
) -> String {
    let path = match protocol {
        router::ModelProtocol::Responses => "v1/responses",
        router::ModelProtocol::ChatCompletions => "v1/chat/completions",
        router::ModelProtocol::AnthropicMessages => "v1/messages",
    };
    let endpoint = newapi::provider_endpoint(&provider.base_url, path)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| provider.base_url.trim_end_matches('/').to_owned());
    let protocol = match protocol {
        router::ModelProtocol::Responses => "responses",
        router::ModelProtocol::ChatCompletions => "chat",
        router::ModelProtocol::AnthropicMessages => "messages",
    };
    let mut hash = Sha256::new();
    for value in [
        endpoint.as_str(),
        provider.model_credential().api_key,
        actual_model,
        protocol,
    ] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

pub fn resolve_member(
    store: &router::Store,
    aggregate: &AggregateModel,
    member: &AggregateMember,
    catalog: &models_dev::Catalog,
) -> Result<ResolvedMember, String> {
    if !aggregate.enabled {
        return Err("aggregate_disabled".into());
    }
    if !member.enabled {
        return Err("member_disabled".into());
    }
    let provider = store
        .providers
        .iter()
        .find(|p| p.id == member.provider_id)
        .ok_or("provider_missing")?;
    if !provider.enabled {
        return Err("provider_disabled".into());
    }
    if !provider.router_only() && provider.model_credential().api_key.trim().is_empty() {
        return Err("credential_missing".into());
    }
    let entry = provider
        .models
        .iter()
        .find(|m| m.id == member.actual_model_id);
    if entry.is_none() && provider.model != member.actual_model_id {
        return Err("model_missing".into());
    }
    if entry.is_some_and(|m| !m.included) || !provider.can_route_model(&member.actual_model_id) {
        return Err("model_unavailable".into());
    }
    newapi::provider_endpoint(&provider.base_url, "v1").map_err(|_| "endpoint_invalid")?;
    let protocol = provider.protocol_for_model(&member.actual_model_id);
    if !provider.router_only() {
        newapi::model_auth_header(
            &provider.base_url,
            protocol,
            provider.model_credential().api_key,
        )
        .map_err(|_| "credential_invalid")?;
    }
    let (metadata, projection) = router::model_projection(
        provider,
        &member.actual_model_id,
        None,
        catalog,
        &store.global_compression(),
    );
    let capability = metadata
        .matched
        .as_ref()
        .and_then(|reference| catalog.exact_in(&reference.provider_id, &reference.model_id));
    let tools = capability.as_ref().and_then(|entry| entry.model.tool_call);
    let vision = capability
        .as_ref()
        .and_then(|entry| entry.model.modalities.as_ref())
        .map(|modalities| modalities.input.iter().any(|input| input == "image"));
    Ok(ResolvedMember {
        member: member.clone(),
        provider: provider.clone(),
        protocol,
        metadata,
        projection,
        target_key: target_key(provider, &member.actual_model_id, protocol),
        tools,
        vision,
    })
}

/// Route topology diagnostics, independent of pricing, balances and labels.
/// Binding identity is target_key, so harmless topology changes never evict it.
pub fn route_revision(store: &router::Store, aggregate: &AggregateModel) -> String {
    let mut hash = Sha256::new();
    let topology = (
        &aggregate.id,
        &aggregate.public_model_id,
        aggregate.enabled,
        aggregate.strategy,
        aggregate.affinity,
        aggregate.retry_policy.max_attempts,
        aggregate.retry_policy.failure_threshold,
    );
    hash.update(serde_json::to_vec(&topology).unwrap_or_default());
    for member in &aggregate.members {
        hash.update(serde_json::to_vec(member).unwrap_or_default());
        if let Some(provider) = store.providers.iter().find(|p| p.id == member.provider_id) {
            hash.update([u8::from(provider.can_route_model(&member.actual_model_id))]);
            hash.update(
                serde_json::to_vec(
                    &provider
                        .models
                        .iter()
                        .find(|m| m.id == member.actual_model_id)
                        .map(|m| m.included),
                )
                .unwrap_or_default(),
            );
            hash.update(
                target_key(
                    provider,
                    &member.actual_model_id,
                    provider.protocol_for_model(&member.actual_model_id),
                )
                .as_bytes(),
            );
        }
    }
    format!("{:x}", hash.finalize())
}

pub fn project(
    store: &router::Store,
    aggregate: &AggregateModel,
    catalog: &models_dev::Catalog,
) -> AggregateView {
    let resolved: Vec<_> = aggregate
        .members
        .iter()
        .map(|member| resolve_member(store, aggregate, member, catalog))
        .collect();
    let member_states = aggregate
        .members
        .iter()
        .zip(&resolved)
        .map(|(member, result)| MemberView {
            member_id: member.id.clone(),
            provider_name: store
                .providers
                .iter()
                .find(|p| p.id == member.provider_id)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            protocol: store
                .providers
                .iter()
                .find(|p| p.id == member.provider_id)
                .map(|p| p.protocol_for_model(&member.actual_model_id)),
            available: result.is_ok(),
            reason: result.as_ref().err().cloned(),
            projection: result.as_ref().ok().map(|r| r.projection.clone()),
        })
        .collect();
    let projections: Vec<_> = resolved
        .iter()
        .filter_map(|r| r.as_ref().ok().map(|r| &r.projection))
        .collect();
    let projection = conservative_projection(store, aggregate, &projections);
    AggregateView {
        model: aggregate.clone(),
        available: projection.is_some(),
        reason: if !aggregate.enabled {
            Some("aggregate_disabled".into())
        } else if projection.is_none() {
            Some("no_available_members".into())
        } else {
            None
        },
        projection,
        member_states,
        route_revision: route_revision(store, aggregate),
        gateway_id: crate::core::gateway::aggregate_id(store, aggregate),
    }
}

fn conservative_projection(
    store: &router::Store,
    aggregate: &AggregateModel,
    members: &[&model_policy::ClientProjection],
) -> Option<model_policy::ClientProjection> {
    let mut result: model_policy::ClientProjection = (**members.first()?).clone();
    result.context_window = members.iter().map(|p| p.context_window).min()?;
    result.context_is_fallback = members.iter().any(|p| p.context_is_fallback);
    result.context_source = if result.context_is_fallback {
        model_metadata::source::FALLBACK.into()
    } else {
        "aggregate".into()
    };
    result.budget = members.iter().map(|p| p.budget).min()?;
    result.budget_source = if members
        .iter()
        .any(|p| p.budget_source == model_metadata::source::FALLBACK)
    {
        model_metadata::source::FALLBACK.into()
    } else {
        "aggregate".into()
    };
    result.input_limit = if members.iter().all(|p| p.input_limit.is_some()) {
        members.iter().filter_map(|p| p.input_limit).min()
    } else {
        None
    };
    result.output_limit = if members.iter().all(|p| p.output_limit.is_some()) {
        members.iter().filter_map(|p| p.output_limit).min()
    } else {
        None
    };
    // Member projections carry numeric client defaults even when no capacity
    // was declared. Aggregating them must preserve that fallback provenance.
    result.output_source = if result.output_limit.is_none() {
        model_metadata::source::UNKNOWN.into()
    } else if members
        .iter()
        .any(|p| p.output_source == model_metadata::source::FALLBACK)
    {
        model_metadata::source::FALLBACK.into()
    } else {
        "aggregate".into()
    };
    result.reasoning.retain(|option| {
        members.iter().all(|p| {
            p.reasoning
                .iter()
                .any(|other| other.get("effort") == option.get("effort"))
        })
    });
    result.reasoning_source = "aggregate".into();
    if !result
        .reasoning
        .iter()
        .any(|r| r.get("effort").and_then(|e| e.as_str()) == result.default_reasoning.as_deref())
    {
        result.default_reasoning = result
            .reasoning
            .first()
            .and_then(|r| r.get("effort"))
            .and_then(|e| e.as_str())
            .map(str::to_owned);
    }
    let global = store.global_compression();
    let (policy, source) = model_policy::resolve(Some(&aggregate.compression), None, &global);
    result.compression =
        model_policy::plan(policy, source, Some(result.budget), &result.budget_source);
    Some(result)
}
