use super::{projection, types::*};
use crate::{
    core::{models_dev, router},
    platform::paths::CodexPaths,
};
use std::collections::HashSet;

fn text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn identifier(value: &str) -> bool {
    text(value, 128)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn model_id(value: &str) -> bool {
    text(value, 256) && !value.chars().any(char::is_whitespace) && !value.contains("::")
}

fn wire_model_id(value: &str) -> bool {
    text(value, 256) && !value.chars().any(char::is_whitespace)
}

/// Structural validation allows broken references already present in a store to
/// remain visible. New/changed member references are checked by save separately.
pub fn validate(store: &router::Store) -> Result<(), String> {
    if store.aggregates.len() > MAX_AGGREGATES {
        return Err("aggregation.errors.tooManyModels".into());
    }
    if store
        .aggregates
        .iter()
        .map(|a| a.members.len())
        .sum::<usize>()
        > MAX_MEMBER_REFERENCES
    {
        return Err("aggregation.errors.tooManyMembers".into());
    }
    let mut ids = HashSet::new();
    let mut public_ids = HashSet::new();
    for model in &store.aggregates {
        if !identifier(&model.id)
            || !model_id(&model.public_model_id)
            || !text(&model.display_name, 200)
        {
            return Err("aggregation.errors.invalidIdentity".into());
        }
        if model.canonical_ref.as_ref().is_some_and(|reference| {
            !text(&reference.provider_id, 128)
                || !model_id(&reference.model_id)
                || reference.model_id != model.public_model_id
        }) {
            return Err("aggregation.errors.invalidIdentity".into());
        }
        if !ids.insert(&model.id) || !public_ids.insert(&model.public_model_id) {
            return Err("aggregation.errors.duplicateIdentity".into());
        }
        if model.members.is_empty() || model.members.len() > MAX_MEMBERS {
            return Err("aggregation.errors.memberCount".into());
        }
        if !(1..=2).contains(&model.retry_policy.max_attempts) {
            return Err("aggregation.errors.retryLimit".into());
        }
        if !(2..=10).contains(&model.retry_policy.failure_threshold) {
            return Err("aggregation.errors.failureThreshold".into());
        }
        if model.first_token_timeout_secs != 0
            && !(MIN_FIRST_TOKEN_SECS..=MAX_FIRST_TOKEN_SECS).contains(&model.first_token_timeout_secs)
        {
            return Err("aggregation.errors.firstTokenTimeout".into());
        }
        model
            .compression
            .validate()
            .map_err(|_| "aggregation.errors.invalidCompression")?;
        let mut member_ids = HashSet::new();
        let mut targets = HashSet::new();
        for member in &model.members {
            if !identifier(&member.id)
                || !identifier(&member.provider_id)
                || !wire_model_id(&member.actual_model_id)
                || member.priority > MAX_PRIORITY
                || !(1..=MAX_WEIGHT).contains(&member.weight)
            {
                return Err("aggregation.errors.invalidMember".into());
            }
            if !member_ids.insert(&member.id)
                || !targets.insert((&member.provider_id, &member.actual_model_id))
            {
                return Err("aggregation.errors.duplicateMember".into());
            }
        }
    }
    Ok(())
}

pub fn state_from_store(store: &router::Store, paths: &CodexPaths) -> AggregationState {
    let catalog = models_dev::load(&paths.app_data_dir);
    AggregationState {
        models: store
            .aggregates
            .iter()
            .map(|a| projection::project(store, a, &catalog))
            .collect(),
        external_enabled: store.router.aggregate_external_enabled,
        listener_ready: crate::core::router_proxy::is_running(),
    }
}

pub fn state(paths: &CodexPaths) -> Result<AggregationState, String> {
    Ok(state_from_store(&router::load_store(paths)?, paths))
}

pub fn save(paths: &CodexPaths, mut model: AggregateModel) -> Result<AggregationState, String> {
    if model.id.is_empty() {
        model.id = uuid::Uuid::new_v4().to_string();
    }
    if model.display_name.is_empty() {
        model.display_name = model.public_model_id.clone();
    }
    for member in &mut model.members {
        if member.id.is_empty() {
            member.id = uuid::Uuid::new_v4().to_string();
        }
    }
    let store = router::mutate_store(paths, |store| {
        let old = store.aggregates.iter().find(|a| a.id == model.id).cloned();
        if old
            .as_ref()
            .is_some_and(|old| old.public_model_id != model.public_model_id)
        {
            return Err("aggregation.errors.publicIdentityImmutable".into());
        }
        for member in &model.members {
            let unchanged = old.as_ref().is_some_and(|a| {
                a.members.iter().any(|m| {
                    m.id == member.id
                        && m.provider_id == member.provider_id
                        && m.actual_model_id == member.actual_model_id
                })
            });
            if !unchanged {
                let provider = store
                    .providers
                    .iter()
                    .find(|p| p.id == member.provider_id)
                    .ok_or("aggregation.errors.providerMissing")?;
                if !provider
                    .models
                    .iter()
                    .any(|m| m.id == member.actual_model_id)
                    && provider.model != member.actual_model_id
                {
                    return Err("aggregation.errors.modelMissing".into());
                }
            }
        }
        if let Some(existing) = store.aggregates.iter_mut().find(|a| a.id == model.id) {
            *existing = model;
        } else {
            store.aggregates.push(model);
        }
        validate(store)
    })?;
    Ok(state_from_store(&store, paths))
}

pub fn delete(paths: &CodexPaths, id: &str) -> Result<AggregationState, String> {
    let store = router::mutate_store(paths, |store| {
        if !store.aggregates.iter().any(|a| a.id == id) {
            return Err("aggregation.errors.notFound".into());
        }
        store.aggregates.retain(|a| a.id != id);
        // Retain a selected target reference as unavailable so recovery and the
        // original injection baseline remain reachable; never select a substitute.
        Ok(())
    })?;
    Ok(state_from_store(&store, paths))
}

pub fn set_external_enabled(paths: &CodexPaths, enabled: bool) -> Result<AggregationState, String> {
    let store = router::mutate_store(paths, |store| {
        store.router.aggregate_external_enabled = enabled;
        Ok(())
    })?;
    Ok(state_from_store(&store, paths))
}
