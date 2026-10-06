//! Desktop routes only reference existing cards. Resolution is shared by preview and requests.
use super::{
    constants, provider,
    types::{
        self, AggregateRouteCandidateView, DesktopError, DesktopResult, DesktopRouteChoice, DesktopRouteRole, DesktopRouteView,
        ModelCandidateView, OneMMode, ProviderCandidateView,
    },
};
use crate::core::{
    aggregation,
    model_targets::{self, ModelTargetRef},
    models_dev, newapi,
    router::{Provider, Store},
};

#[derive(Clone)]
pub struct ResolvedRoute {
    pub choice: DesktopRouteChoice,
    pub provider: Option<Provider>,
    /// Only an explicit reference, never a member chosen during configuration.
    pub aggregate_id: Option<String>,
    pub view: DesktopRouteView,
}

impl DesktopRouteChoice {
    pub fn target(&self) -> ModelTargetRef {
        self.target_ref
            .clone()
            .unwrap_or_else(|| ModelTargetRef::provider(&self.provider_id, &self.model_id))
    }

    pub fn target_exists(&self, store: &Store) -> bool {
        match self.target() {
            ModelTargetRef::Provider { provider_id, .. } => {
                store.providers.iter().any(|p| p.id == provider_id)
            }
            ModelTargetRef::Aggregate { aggregate_id } => {
                store.aggregates.iter().any(|a| a.id == aggregate_id)
            }
        }
    }
}

pub fn aggregate_candidates(
    store: &Store,
    catalog: &models_dev::Catalog,
) -> Vec<AggregateRouteCandidateView> {
    store
        .aggregates
        .iter()
        .map(|aggregate| aggregate_candidate(store, aggregate, catalog))
        .collect()
}

fn aggregate_candidate(
    store: &Store,
    aggregate: &aggregation::AggregateModel,
    catalog: &models_dev::Catalog,
) -> AggregateRouteCandidateView {
    let target = ModelTargetRef::aggregate(&aggregate.id);
    let view = model_targets::resolve_client_target(
        store,
        &target,
        crate::core::protocol::ModelProtocol::AnthropicMessages,
        catalog,
    );
    let supports1m = model_targets::resolve_client_target_with_context(
        store,
        &target,
        crate::core::protocol::ModelProtocol::AnthropicMessages,
        catalog,
        Some(constants::ONE_M_CONTEXT),
    )
    .is_ok_and(|target| target.supports_one_m());
    let available = view.as_ref().is_ok_and(|view| view.available);
    let context_window = view
        .as_ref()
        .ok()
        .and_then(|view| view.projection.as_ref().map(|p| p.context_window));
    let reason = match view {
        Ok(view) => view.reason,
        Err(error) => Some(error),
    };
    let label = if aggregate.display_name.trim().is_empty() {
        aggregate.public_model_id.clone()
    } else {
        aggregate.display_name.clone()
    };
    AggregateRouteCandidateView {
        aggregate_id: aggregate.id.clone(),
        name: label.clone(),
        enabled: aggregate.enabled,
        eligible: available,
        reason: reason.clone(),
        model: ModelCandidateView {
            model_id: aggregate.public_model_id.clone(),
            label: label.clone(),
            display_name: label,
            protocol: "aggregate".into(),
            protocol_source: "aggregate".into(),
            protocol_verified: true,
            included: aggregate.enabled,
            supports1m,
            context_window,
            context_source: "aggregate".into(),
            selectable: available,
            exclude_reason: reason,
        },
    }
}

impl std::fmt::Debug for ResolvedRoute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedRoute")
            .field("choice", &self.choice)
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

pub fn roles(policy: provider::DesktopVersionPolicy) -> Vec<DesktopRouteRole> {
    constants::roles_for(policy)
        .iter()
        .filter_map(|role| {
            constants::route_alias(role, policy).map(|alias| DesktopRouteRole {
                role: (*role).into(),
                role_alias: alias.into(),
            })
        })
        .collect()
}

fn provider_reason(card: &Provider) -> Option<String> {
    if !card.enabled {
        return Some("provider_disabled".into());
    }
    if provider::require_inference_key(card).is_err() {
        return Some(types::reason::NO_INFERENCE_KEY.into());
    }
    if newapi::base_url(card.model_endpoint().base_url).is_err() {
        return Some("invalid_url".into());
    }
    None
}

pub fn model_candidate(
    card: &Provider,
    model: &str,
    catalog: &models_dev::Catalog,
) -> ModelCandidateView {
    let mut view = provider::model_candidate(
        card,
        model,
        catalog,
        provider::DesktopVersionPolicy::conservative(),
    );
    let saved = card.models.iter().find(|entry| entry.id == model);
    let reason = provider_reason(card).or_else(|| {
        if provider::basic_model_id_error(model).is_some() {
            return Some(types::reason::UNSAFE_MODEL_NAME.into());
        }
        match saved {
            Some(entry) if !entry.included => {
                return Some(types::reason::MODEL_NOT_INCLUDED.into())
            }
            None if card.model.trim() != model => return Some(types::reason::MODEL_REMOVED.into()),
            _ => {}
        }
        if !card.supports_model(model) {
            return Some(types::reason::UNSUPPORTED_MODEL_ENDPOINT.into());
        }
        None
    });
    view.selectable = reason.is_none();
    view.exclude_reason = reason;
    view
}

pub fn candidates(store: &Store, catalog: &models_dev::Catalog) -> Vec<ProviderCandidateView> {
    store
        .providers
        .iter()
        .map(|card| {
            let mut ids: Vec<String> = card.models.iter().map(|entry| entry.id.clone()).collect();
            if !card.model.trim().is_empty() && !ids.iter().any(|id| id == card.model.trim()) {
                ids.insert(0, card.model.trim().into());
            }
            ids.sort();
            ids.dedup();
            let models: Vec<_> = ids
                .iter()
                .map(|id| model_candidate(card, id, catalog))
                .collect();
            let reason = provider_reason(card);
            ProviderCandidateView {
                provider_id: card.id.clone(),
                name: card.name.clone(),
                enabled: card.enabled,
                has_inference_key: provider::inference_key(card).is_some(),
                eligible: reason.is_none() && models.iter().any(|m| m.selectable),
                reason,
                endpoint: None,
                models,
            }
        })
        .collect()
}

pub fn resolve_routes(
    store: &Store,
    choices: &[DesktopRouteChoice],
    policy: provider::DesktopVersionPolicy,
    catalog: &models_dev::Catalog,
) -> DesktopResult<Vec<ResolvedRoute>> {
    if choices.is_empty() {
        return Err(Box::new(DesktopError::new(
            types::error_code::NO_MODELS_SELECTED,
            "claudeDesktop.errors.noRoutesSelected",
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut resolved = Vec::new();
    for choice in choices {
        let Some(current_alias) = constants::route_alias(&choice.role, policy) else {
            return Err(Box::new(
                DesktopError::new(
                    types::error_code::MODEL_REJECTED,
                    "claudeDesktop.errors.invalidRouteRole",
                )
                .model(&choice.role_alias),
            ));
        };
        if !constants::route_alias_matches(&choice.role, &choice.role_alias, policy)
            || !seen.insert(choice.role.clone())
        {
            return Err(Box::new(
                DesktopError::new(
                    types::error_code::MODEL_REJECTED,
                    "claudeDesktop.errors.invalidRouteRole",
                )
                .model(&choice.role_alias),
            ));
        }
        let target = choice.target();
        if let ModelTargetRef::Aggregate { aggregate_id } = &target {
            let target_view = model_targets::resolve_client_target(
                store,
                &target,
                crate::core::router::ModelProtocol::AnthropicMessages,
                catalog,
            )
            .map_err(|reason| {
                Box::new(
                    DesktopError::new(
                        types::error_code::MODEL_REJECTED,
                        "claudeDesktop.errors.routeUnavailable",
                    )
                    .param(reason),
                )
            })?;
            if !target_view.available {
                return Err(Box::new(
                    DesktopError::new(
                        types::error_code::MODEL_REJECTED,
                        "claudeDesktop.errors.routeUnavailable",
                    )
                    .param(target_view.reason.unwrap_or_default()),
                ));
            }
            let aggregate = store
                .aggregates
                .iter()
                .find(|a| &a.id == aggregate_id)
                .ok_or_else(|| {
                    Box::new(DesktopError::new(
                        types::error_code::PROVIDER_MISSING,
                        "claudeDesktop.errors.providerMissing",
                    ))
                })?;
            let candidate = aggregate_candidate(store, aggregate, catalog);
            let label = match choice.label_override.as_deref() {
                Some(raw) => provider::normalize_label(raw).ok_or_else(|| {
                    Box::new(DesktopError::new(
                        types::error_code::MODEL_REJECTED,
                        "claudeDesktop.errors.invalidLabel",
                    ))
                })?,
                None => provider::normalize_label(&target_view.display_name)
                    .unwrap_or_else(|| target_view.logical_model_id.clone()),
            };
            let mut canonical_choice = choice.clone();
            canonical_choice.role_alias = current_alias.to_owned();
            canonical_choice.provider_id.clear();
            canonical_choice.model_id = target_view.logical_model_id.clone();
            let view = DesktopRouteView {
                role: choice.role.clone(),
                role_alias: current_alias.to_owned(),
                provider_id: String::new(),
                provider_name: target_view.display_name,
                model_id: target_view.logical_model_id,
                target_ref: Some(target.clone()),
                label,
                one_m: choice.one_m_mode == OneMMode::Auto && candidate.model.supports1m,
                protocol: "aggregate".into(),
                available: true,
                reason: None,
            };
            resolved.push(ResolvedRoute {
                choice: canonical_choice,
                provider: None,
                aggregate_id: Some(aggregate_id.clone()),
                view,
            });
            continue;
        }
        let ModelTargetRef::Provider {
            provider_id,
            model_id,
        } = target
        else {
            unreachable!()
        };
        let card = store
            .providers
            .iter()
            .find(|card| card.id == provider_id)
            .ok_or_else(|| {
                Box::new(DesktopError::new(
                    types::error_code::PROVIDER_MISSING,
                    "claudeDesktop.errors.providerMissing",
                ))
            })?;
        let candidate = model_candidate(card, &model_id, catalog);
        if let Some(reason) = candidate.exclude_reason {
            return Err(Box::new(
                DesktopError::new(
                    types::error_code::MODEL_REJECTED,
                    "claudeDesktop.errors.routeUnavailable",
                )
                .model(&choice.model_id)
                .param(reason),
            ));
        }
        let label = match choice.label_override.as_deref() {
            Some(raw) => provider::normalize_label(raw).ok_or_else(|| {
                Box::new(DesktopError::new(
                    types::error_code::MODEL_REJECTED,
                    "claudeDesktop.errors.invalidLabel",
                ))
            })?,
            None => {
                provider::normalize_label(&format!("{} · {}", card.name, candidate.display_name))
                    .unwrap_or_else(|| choice.model_id.clone())
            }
        };
        let mut canonical_choice = choice.clone();
        canonical_choice.role_alias = current_alias.to_owned();
        canonical_choice.provider_id = provider_id;
        canonical_choice.model_id = model_id.clone();
        let view = DesktopRouteView {
            role: choice.role.clone(),
            role_alias: current_alias.to_owned(),
            provider_id: card.id.clone(),
            provider_name: card.name.clone(),
            model_id,
            target_ref: choice.target_ref.clone(),
            label,
            one_m: choice.one_m_mode == OneMMode::Auto && candidate.supports1m,
            protocol: candidate.protocol,
            available: true,
            reason: None,
        };
        let mut frozen_provider = card.clone();
        // Qualification and forwarding must agree on the same inference credential.
        // Normalize only the request snapshot; the saved card is never rewritten here.
        frozen_provider.api_key = provider::require_inference_key(card)?;
        resolved.push(ResolvedRoute {
            choice: canonical_choice,
            provider: Some(frozen_provider),
            aggregate_id: None,
            view,
        });
    }
    Ok(resolved)
}

/// The file projection excludes credentials, prices and balances. It binds only file/eligibility inputs.
pub fn file_projection(resolved: &[ResolvedRoute]) -> String {
    use sha2::{Digest, Sha256};
    let inputs: Vec<_> = resolved
        .iter()
        .map(|r| {
            (
                &r.view.role,
                &r.view.role_alias,
                &r.view.provider_id,
                &r.view.model_id,
                &r.view.label,
                r.view.one_m,
                &r.view.protocol,
            )
        })
        .collect();
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(&inputs).expect("route projection serializes"));
    // Preserve the exact legacy provider projection across the schema upgrade.
    let aggregate_ids: Vec<_> = resolved
        .iter()
        .filter_map(|r| r.aggregate_id.as_ref())
        .collect();
    if !aggregate_ids.is_empty() {
        hash.update(serde_json::to_vec(&aggregate_ids).expect("aggregate IDs serialize"));
    }
    format!("{:x}", hash.finalize())
}
