//! Desktop routes only reference existing cards. Resolution is shared by preview and requests.
use super::{
    constants, provider,
    types::{
        self, DesktopError, DesktopResult, DesktopRouteChoice, DesktopRouteRole, DesktopRouteView,
        ModelCandidateView, OneMMode, ProviderCandidateView,
    },
};
use crate::core::{
    models_dev, newapi,
    router::{Provider, Store},
};

#[derive(Clone)]
pub struct ResolvedRoute {
    pub choice: DesktopRouteChoice,
    pub provider: Provider,
    pub view: DesktopRouteView,
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
        if constants::route_alias(&choice.role, policy) != Some(choice.role_alias.as_str())
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
        let card = store
            .providers
            .iter()
            .find(|card| card.id == choice.provider_id)
            .ok_or_else(|| {
                Box::new(DesktopError::new(
                    types::error_code::PROVIDER_MISSING,
                    "claudeDesktop.errors.providerMissing",
                ))
            })?;
        let candidate = model_candidate(card, &choice.model_id, catalog);
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
        let view = DesktopRouteView {
            role: choice.role.clone(),
            role_alias: choice.role_alias.clone(),
            provider_id: card.id.clone(),
            provider_name: card.name.clone(),
            model_id: choice.model_id.clone(),
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
            choice: choice.clone(),
            provider: frozen_provider,
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
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&inputs).expect("route projection serializes"))
    )
}
