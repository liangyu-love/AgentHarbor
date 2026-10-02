//! Capability preview: serializes the exact resolver output and client
//! projection that injection uses, so the panel can never disagree with the
//! generated configuration. No value is recomputed here.
use super::model_metadata::{
    Field, FieldConflict, MatchKind, MatchStep, PriceInfo, ReasoningOptionInfo,
};
use super::{model_catalog, model_defaults, model_policy, models_dev, newapi, router};
use crate::platform::paths::CodexPaths;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub provider_id: String,
    pub provider_name: String,
    pub display_name: String,
    pub upstream_model: String,
    /// Human-readable identity the resolver settled on (or the normalized
    /// fallback when nothing matched).
    pub matched_model: String,
    pub match_kind: MatchKind,
    pub matched: Option<models_dev::ModelRef>,
    pub steps: Vec<MatchStep>,
    pub conflicts: Vec<FieldConflict>,
    pub reasoning: Field<Vec<newapi::ReasoningLevel>>,
    pub reasoning_supports: Option<bool>,
    pub reasoning_options: Vec<ReasoningOptionInfo>,
    pub default_reasoning: Option<String>,
    pub default_reasoning_source: String,
    pub context: Field<u64>,
    pub input_limit: Field<u64>,
    pub output: Field<u64>,
    pub price: PriceInfo,
    /// Effective values the client projection uses, including compression.
    pub projection: model_policy::ClientProjection,
    /// Whether the threshold in the projection is the one currently applied to
    /// a client. `Some(false)` means it still needs a config sync.
    pub compression_applied: Option<bool>,
    /// catalog | config | none
    pub compression_applied_source: String,
    pub compression_applied_threshold: Option<u64>,
    pub compression_shared_threshold: Option<u64>,
    pub compression_limiting_models: Vec<String>,
    pub compression_scope: String,
    pub compression_shared_applied: Option<bool>,
    pub compression_shared_plan: Option<model_policy::SharedCompressionPlan>,
    pub compression_config_active: bool,
    pub compression_written_threshold: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    /// Codex official snapshot status.
    pub stale: bool,
    pub error: Option<String>,
    pub updated_at: Option<i64>,
    /// models.dev catalog status (revision, age, errors).
    pub catalog: models_dev::CatalogStatus,
    pub models: Vec<Preview>,
}

pub fn preview(paths: &CodexPaths, store: &router::Store) -> CatalogView {
    let snapshot = model_catalog::official(&paths.codex_home, &paths.app_data_dir);
    let models_dev = models_dev::load(&paths.app_data_dir);
    let global_compression = store.global_compression();
    let now = chrono::Utc::now().timestamp();
    // Applied state: the written router catalog for relay entries, and the
    // native configuration when that mode owns Codex. Both are read-only.
    let applied_thresholds = router::applied_catalog_thresholds(paths);
    let native_mode = store.codex_access_mode == router::CodexAccessMode::Native;
    let router_active = !native_mode && store.router.enabled && router::routing_config_intact(paths);
    let native_state = native_mode
        .then(|| router::native_compression_state(paths, store, &models_dev, &snapshot.models))
        .flatten();
    let mut models = Vec::new();
    for provider in &store.providers {
        let mut ids: Vec<&str> = provider.models.iter().map(|m| m.id.as_str()).collect();
        if !provider.model.is_empty() && !ids.contains(&provider.model.as_str()) {
            ids.push(&provider.model);
        }
        for id in ids {
            let official = router::matched_official_model(&snapshot.models, id)
                .filter(|_| router::official_endpoint_applies(provider));
            let (metadata, projection) = if native_mode { router::model_projection(
                provider,
                id,
                official,
                &models_dev,
                &global_compression,
            ) } else { router::relay_model_projection(store, provider, id, official, &models_dev, &global_compression) };
            let matched_model = metadata
                .matched
                .as_ref()
                .map(|reference| format!("{}/{}", reference.provider_id, reference.model_id))
                .unwrap_or_else(|| model_defaults::model_key(id));
            let native_scope = native_state.as_ref().filter(|state|
                state.provider_id == provider.id && state.models.iter().any(|model| model == id));
            let (compression_applied, applied_source) = if native_mode {
                (Some(native_scope.is_some_and(|state| state.applied == Some(true)
                    && state.applied_threshold == projection.compression.threshold)), "config".to_owned())
            } else if !router_active {
                (Some(false), "none".to_owned())
            } else { match &applied_thresholds {
                    Some(thresholds) => {
                        let slug = format!("{}::{id}", provider.id);
                        let applied = thresholds.get(&slug) == Some(&projection.compression.threshold);
                        (Some(applied), "catalog".to_owned())
                    }
                    None => (None, "none".to_owned()),
                }
            };
            models.push(Preview {
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                display_name: metadata.display_name.clone(),
                upstream_model: id.to_owned(),
                matched_model,
                match_kind: metadata.match_kind,
                matched: metadata.matched.clone(),
                steps: metadata.steps.clone(),
                conflicts: metadata.conflicts.clone(),
                reasoning: metadata.reasoning.levels.clone(),
                reasoning_supports: metadata.reasoning.supports,
                reasoning_options: metadata.reasoning.options.clone(),
                default_reasoning: metadata.default_effort.clone(),
                default_reasoning_source: metadata.default_effort_source.clone(),
                context: metadata.context_window.clone(),
                input_limit: metadata.input_limit.clone(),
                output: metadata.output_limit.clone(),
                price: metadata.price.clone(),
                projection,
                compression_applied,
                compression_applied_source: applied_source,
                compression_applied_threshold: if native_mode { native_scope.and_then(|state| state.applied_threshold) }
                    else { router_active.then(|| applied_thresholds.as_ref().and_then(|entries| entries.get(&format!("{}::{id}", provider.id)).copied().flatten())).flatten() },
                compression_shared_threshold: native_scope.and_then(|state| state.threshold),
                compression_limiting_models: native_scope.map(|state| state.limiting_models.clone()).unwrap_or_default(),
                compression_scope: if native_mode {
                    if native_scope.is_some() { "nativeShared" } else { "inactive" }
                } else if router_active { "routerModel" } else { "inactive" }.into(),
                compression_shared_applied: native_scope.and_then(|state| state.applied),
                compression_shared_plan: native_scope.map(|state| state.shared_plan.clone()),
                compression_config_active: if native_mode { native_scope.is_some_and(|state| state.config_active) } else { router_active },
                compression_written_threshold: if native_mode { None } else {
                    applied_thresholds.as_ref().and_then(|entries| entries.get(&format!("{}::{id}", provider.id)).copied().flatten())
                },
            });
        }
    }
    CatalogView {
        stale: snapshot.stale,
        error: snapshot.error,
        updated_at: snapshot.updated_at,
        catalog: models_dev.status(now),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str, model: &str) -> router::Provider {
        router::Provider {
            id: id.into(),
            name: id.into(),
            model: model.into(),
            enabled: true,
            api_key: "fixture".into(),
            models: vec![newapi::Model {
                id: model.into(),
                display_name: "Alias".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn preview_serializes_the_same_projection_injection_uses() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        let store = router::Store {
            providers: vec![provider("relay", "claude-opus-4-5")],
            ..Default::default()
        };
        let view = preview(&paths, &store);
        let row = &view.models[0];
        assert_eq!(row.upstream_model, "claude-opus-4-5");
        assert_eq!(row.display_name, "Alias");
        // The preview and the generated catalog entry share the projection.
        let catalog = router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        let entry = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["slug"] == "relay::claude-opus-4-5")
            .unwrap();
        assert_eq!(
            entry["context_window"].as_u64(),
            Some(row.projection.context_window)
        );
        assert_eq!(
            entry["max_output_tokens"].as_u64(),
            row.projection.output_limit
        );
        // 90% of the effective budget by default.
        assert_eq!(
            entry["auto_compact_token_limit"].as_u64(),
            row.projection.compression.threshold
        );
    }

    #[test]
    fn unknown_models_are_marked_as_fallback_not_as_catalog_data() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        let store = router::Store {
            providers: vec![provider("relay", "totally-unknown-model-xyz")],
            ..Default::default()
        };
        let view = preview(&paths, &store);
        let row = &view.models[0];
        assert!(row.projection.context_is_fallback);
        assert_eq!(row.projection.context_source, "fallback");
        assert_eq!(row.projection.context_window, 128_000);
        assert_eq!(row.projection.compression.threshold, Some(115_200));
    }

    #[test]
    fn compression_applied_state_separates_pending_from_applied() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        paths.ensure_directories().unwrap();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(&paths.config_path, "").unwrap();
        let mut provider = provider("relay", "gpt-5.2");
        provider.base_url = "https://relay.example/v1".into();
        provider.context_window = Some(200_000);
        let mut store = router::Store {
            providers: vec![provider],
            ..Default::default()
        };
        // A missing active configuration is inactive even without a catalog.
        let view = preview(&paths, &store);
        assert_eq!(view.models[0].compression_applied, Some(false));

        // A written file alone never proves that Codex consumes it.
        router::write_catalog(&paths, &store).unwrap();
        assert_eq!(preview(&paths, &store).models[0].compression_scope, "inactive");
        store.router.enabled = true;
        store.router.bearer_token = "synthetic-bearer".into();
        router::apply_routing_config(&paths, &store).unwrap();
        let view = preview(&paths, &store);
        assert_eq!(view.models[0].compression_applied, Some(true));
        assert_eq!(view.models[0].compression_applied_source, "catalog");

        // A policy change makes the written threshold stale until a sync.
        let mut changed = store.clone();
        changed.providers[0].compression = model_policy::CompressionPolicy::percent(50);
        let view = preview(&paths, &changed);
        assert_eq!(view.models[0].compression_applied, Some(false));
    }

    #[test]
    fn provider_compression_and_overrides_flow_through_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        let mut provider = provider("relay", "gpt-5.2");
        provider.context_window = Some(200_000);
        provider.compression = model_policy::CompressionPolicy::percent(80);
        provider.models[0].overrides.context_window = Some(200_000);
        let store = router::Store {
            providers: vec![provider],
            ..Default::default()
        };
        let view = preview(&paths, &store);
        let row = &view.models[0];
        assert_eq!(row.context.value, Some(200_000));
        assert_eq!(row.context.source, "manual");
        // The raw catalog input stays visible, but the final user context
        // constrains the effective budget used for compaction.
        assert_eq!(row.input_limit.value, Some(272_000));
        assert_eq!(row.projection.budget, 200_000);
        assert_eq!(row.projection.compression.threshold, Some(160_000));
        assert_eq!(row.projection.compression.source, "provider");
        // The generic reference supplies the output cap without a conflict.
        assert_eq!(row.projection.output_limit, Some(128_000));
    }
}
