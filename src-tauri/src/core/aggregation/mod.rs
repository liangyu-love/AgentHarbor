//! Explicit model aggregation. Stored members refer to provider models; credentials
//! remain in the protected router store and never enter the configuration DTOs.
pub mod affinity;
pub mod config;
pub mod execution;
pub mod projection;
pub mod runtime;
pub mod selector;
pub mod types;

pub use config::{delete, save, set_external_enabled, state};
pub use projection::{project, resolve_member, target_key, ResolvedMember};
pub use types::*;

/// Configuration edits prune idle physical targets and rotation scopes. Active
/// leases keep their counters until the in-flight response ends.
pub fn reconcile_runtime(store: &crate::core::router::Store) {
    use crate::core::{protocol::ModelProtocol, router};
    use std::collections::HashSet;
    let mut targets = HashSet::new();
    let mut scopes = HashSet::new();
    for aggregate in store
        .aggregates
        .iter()
        .filter(|aggregate| aggregate.enabled)
    {
        scopes.insert(aggregate.id.clone());
        for member in aggregate.members.iter().filter(|member| member.enabled) {
            let Some(provider) = store.providers.iter().find(|provider| {
                provider.id == member.provider_id
                    && provider.can_route_model(&member.actual_model_id)
            }) else {
                continue;
            };
            let protocol = provider.protocol_for_model(&member.actual_model_id);
            targets.insert(target_key(provider, &member.actual_model_id, protocol));
            if protocol == ModelProtocol::Responses
                && router::supports_command_search(provider)
                && provider.supports_model_protocol(
                    &member.actual_model_id,
                    ModelProtocol::ChatCompletions,
                )
            {
                targets.insert(target_key(
                    provider,
                    &member.actual_model_id,
                    ModelProtocol::ChatCompletions,
                ));
            }
        }
    }
    runtime::shared().retain_targets(&targets);
    runtime::shared().retain_scopes(&scopes);
}

#[cfg(test)]
mod tests;
