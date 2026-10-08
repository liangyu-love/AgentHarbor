//! Capability preview: serializes the exact resolver output and client
//! projection that injection uses, so the panel can never disagree with the
//! generated configuration. No value is recomputed here.
use super::model_metadata::{
    Field, FieldConflict, MatchKind, MatchStep, PriceInfo, ReasoningOptionInfo,
};
use super::{model_catalog, model_defaults, model_policy, models_dev, newapi, router};
use crate::platform::paths::CodexPaths;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogHint {
    pub source: models_dev::ModelRef,
    pub canonical_raw: Option<String>,
    pub canonical_target: Option<models_dev::ModelRef>,
    /// absent | invalid | missingTarget | verified | self | declaredAlias | sourceMissing
    pub canonical_status: String,
    /// declared | absent | unknown (older embedded/runtime snapshot)
    pub reasoning_options_presence: String,
    pub reasoning_options: Vec<ReasoningOptionInfo>,
    pub interleaved: Option<models_dev::Interleaved>,
}

fn catalog_hints(catalog: &models_dev::Catalog, steps: &[MatchStep]) -> Vec<CatalogHint> {
    let Some(step) = steps.iter().rev().find(|step| !step.matched.is_empty()) else {
        return Vec::new();
    };
    // A speed reference transfers context only. The base record's identity,
    // reasoning controls and interleaving are not declarations for the variant.
    if step.step == "speedVariantReference" {
        return Vec::new();
    }
    let mut hints = Vec::new();
    for source in &step.matched {
        if hints
            .iter()
            .any(|hint: &CatalogHint| hint.source == *source)
        {
            continue;
        }
        let Some(found) = catalog.exact_in(&source.provider_id, &source.model_id) else {
            hints.push(CatalogHint {
                source: source.clone(),
                canonical_raw: None,
                canonical_target: None,
                canonical_status: "sourceMissing".into(),
                reasoning_options_presence: "unknown".into(),
                reasoning_options: Vec::new(),
                interleaved: None,
            });
            continue;
        };
        let raw = found.model.canonical_model_id.clone();
        let (target, status) = match raw.as_deref() {
            None => (None, "absent"),
            Some(raw) => match raw.split_once('/') {
                Some((provider, model))
                    if !provider.is_empty()
                        && !model.is_empty()
                        && !raw.chars().any(char::is_control) =>
                {
                    let reference = models_dev::ModelRef {
                        provider_id: provider.into(),
                        model_id: model.into(),
                    };
                    if catalog.exact_in(provider, model).is_some() {
                        (
                            Some(reference),
                            if provider == source.provider_id && model == source.model_id {
                                "self"
                            } else {
                                "verified"
                            },
                        )
                    } else if provider.eq_ignore_ascii_case(&source.provider_id) {
                        // Some publishers expose an API alias while declaring
                        // the versioned canonical name on that same record.
                        (Some(reference), "declaredAlias")
                    } else {
                        (Some(reference), "missingTarget")
                    }
                }
                _ => (None, "invalid"),
            },
        };
        hints.push(CatalogHint {
            source: source.clone(),
            canonical_raw: raw,
            canonical_target: target,
            canonical_status: status.into(),
            reasoning_options_presence: match found.model.reasoning_options_declared {
                Some(true) => "declared",
                Some(false) => "absent",
                None => "unknown",
            }
            .into(),
            reasoning_options: super::model_metadata::option_infos(found.model),
            interleaved: found.model.interleaved.clone(),
        });
    }
    hints
}

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
    pub catalog_hints: Vec<CatalogHint>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearPreview {
    pub config_revision: String,
    pub catalog_revision: String,
    pub official_revision: String,
    pub before: Preview,
    pub after: Preview,
    /// A provider-level setting can change more than the model whose card
    /// opened the preview. Report the complete scope and changed model IDs.
    pub scope_count: usize,
    pub affected_models: Vec<ClearAffectedModel>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearAffectedModel {
    pub model_id: String,
    pub display_name: String,
}

pub fn preview_clear(
    paths: &CodexPaths,
    store: &router::Store,
    provider_id: &str,
    model_id: &str,
    field: router::ClearCapabilityField,
) -> Result<ClearPreview, String> {
    let official = model_catalog::official(&paths.codex_home, &paths.app_data_dir);
    let catalog = models_dev::load(&paths.app_data_dir);
    let mut after_store = store.clone();
    router::clear_capability_on_store(&mut after_store, provider_id, model_id, field)?;
    let before_view = preview_with_sources(paths, store, &official, &catalog);
    let after_view = preview_with_sources(paths, &after_store, &official, &catalog);
    let after_by_model: HashMap<&str, &Preview> = after_view
        .models
        .iter()
        .filter(|model| model.provider_id == provider_id)
        .map(|model| (model.upstream_model.as_str(), model))
        .collect();
    let mut scope_count = 0;
    let mut affected_models = Vec::new();
    for model in before_view
        .models
        .iter()
        .filter(|model| model.provider_id == provider_id)
    {
        scope_count += 1;
        let Some(after) = after_by_model.get(model.upstream_model.as_str()) else {
            return Err("newapi.errors.notFound".into());
        };
        let before_value = serde_json::to_value(model).map_err(|_| "newapi.errors.storage")?;
        let after_value = serde_json::to_value(after).map_err(|_| "newapi.errors.storage")?;
        if before_value != after_value {
            affected_models.push(ClearAffectedModel {
                model_id: model.upstream_model.clone(),
                display_name: model.display_name.clone(),
            });
        }
    }
    let find = |view: CatalogView| {
        view.models
            .into_iter()
            .find(|model| model.provider_id == provider_id && model.upstream_model == model_id)
            .ok_or_else(|| "newapi.errors.notFound".to_owned())
    };
    let before = find(before_view)?;
    let after = find(after_view)?;
    let official_bytes =
        serde_json::to_vec(&official.models).map_err(|_| "newapi.errors.storage")?;
    Ok(ClearPreview {
        config_revision: router::config_revision(store)?,
        catalog_revision: catalog.snapshot.revision.clone(),
        official_revision: super::request_diagnostics::state_revision(&official_bytes),
        before,
        after,
        scope_count,
        affected_models,
    })
}

pub fn preview(paths: &CodexPaths, store: &router::Store) -> CatalogView {
    let snapshot = model_catalog::official(&paths.codex_home, &paths.app_data_dir);
    let models_dev = models_dev::load(&paths.app_data_dir);
    preview_with_sources(paths, store, &snapshot, &models_dev)
}

pub(crate) fn preview_with_sources(
    paths: &CodexPaths,
    store: &router::Store,
    snapshot: &model_catalog::Snapshot,
    models_dev: &models_dev::Catalog,
) -> CatalogView {
    let global_compression = store.global_compression();
    let now = chrono::Utc::now().timestamp();
    // Applied state: the written router catalog for relay entries, and the
    // native configuration when that mode owns Codex. Both are read-only.
    let applied_thresholds = router::applied_catalog_thresholds(paths);
    let native_mode = store.codex_access_mode == router::CodexAccessMode::Native;
    let router_active =
        !native_mode && store.router.enabled && router::routing_config_intact(paths);
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
            let (metadata, projection) = if native_mode {
                router::model_projection(provider, id, official, &models_dev, &global_compression)
            } else {
                router::relay_model_projection(
                    store,
                    provider,
                    id,
                    official,
                    &models_dev,
                    &global_compression,
                )
            };
            let matched_model = metadata
                .matched
                .as_ref()
                .map(|reference| format!("{}/{}", reference.provider_id, reference.model_id))
                .unwrap_or_else(|| model_defaults::model_key(id));
            let native_scope = native_state.as_ref().filter(|state| {
                state.provider_id == provider.id && state.models.iter().any(|model| model == id)
            });
            let (compression_applied, applied_source) = if native_mode {
                (
                    Some(native_scope.is_some_and(|state| {
                        state.applied == Some(true)
                            && state.applied_threshold == projection.compression.threshold
                    })),
                    "config".to_owned(),
                )
            } else if !router_active {
                (Some(false), "none".to_owned())
            } else {
                match &applied_thresholds {
                    Some(thresholds) => {
                        let slug = format!("{}::{id}", provider.id);
                        let applied =
                            thresholds.get(&slug) == Some(&projection.compression.threshold);
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
                catalog_hints: catalog_hints(&models_dev, &metadata.steps),
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
                compression_applied_threshold: if native_mode {
                    native_scope.and_then(|state| state.applied_threshold)
                } else {
                    router_active
                        .then(|| {
                            applied_thresholds.as_ref().and_then(|entries| {
                                entries
                                    .get(&format!("{}::{id}", provider.id))
                                    .copied()
                                    .flatten()
                            })
                        })
                        .flatten()
                },
                compression_shared_threshold: native_scope.and_then(|state| state.threshold),
                compression_limiting_models: native_scope
                    .map(|state| state.limiting_models.clone())
                    .unwrap_or_default(),
                compression_scope: if native_mode {
                    if native_scope.is_some() {
                        "nativeShared"
                    } else {
                        "inactive"
                    }
                } else if router_active {
                    "routerModel"
                } else {
                    "inactive"
                }
                .into(),
                compression_shared_applied: native_scope.and_then(|state| state.applied),
                compression_shared_plan: native_scope.map(|state| state.shared_plan.clone()),
                compression_config_active: if native_mode {
                    native_scope.is_some_and(|state| state.config_active)
                } else {
                    router_active
                },
                compression_written_threshold: if native_mode {
                    None
                } else {
                    applied_thresholds.as_ref().and_then(|entries| {
                        entries
                            .get(&format!("{}::{id}", provider.id))
                            .copied()
                            .flatten()
                    })
                },
            });
        }
    }
    CatalogView {
        stale: snapshot.stale,
        error: snapshot.error.clone(),
        updated_at: snapshot.updated_at,
        catalog: models_dev.status(now),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model_metadata;

    #[test]
    fn speed_reference_does_not_publish_base_identity_or_reasoning_hints() {
        let catalog = models_dev::Catalog::parse(
            serde_json::json!({
                "schemaVersion": 1,
                "providers": [{"id": "deepseek", "models": [{
                    "id": "deepseek-flash", "canonicalModelId": "deepseek/deepseek-v4.1-flash",
                    "limit": {"context": 1000000}, "reasoning": true,
                    "reasoningOptionsDeclared": true, "reasoningOptions": [{"type": "toggle"}],
                    "interleaved": {"field": "reasoning_content"}
                }]}]
            })
            .to_string()
            .as_bytes(),
            models_dev::Origin::Embedded,
        )
        .unwrap();
        let resolved = model_metadata::resolve(
            &catalog,
            &model_metadata::ResolutionRequest {
                raw_model_id: "deepseek/deepseek-v4.1-flash-fast",
                display_name: "",
                catalog_providers: &["deepseek".into()],
                price_providers: &[],
                explicit_ref: None,
                price_ref: None,
                user: Default::default(),
                provider_context: None,
                official: None,
                upstream: None,
            },
        );
        assert_eq!(resolved.context_window.value, Some(1000000));
        assert_eq!(
            resolved.context_window.source,
            model_metadata::source::CATALOG_SPEED_REFERENCE
        );
        assert!(catalog_hints(&catalog, &resolved.steps).is_empty());
    }

    #[test]
    fn canonical_links_only_validate_identity_and_keep_source_records() {
        let catalog = models_dev::Catalog::parse(serde_json::json!({
            "schemaVersion": 1, "sourceUrl": "fixture", "fetchedAt": 1, "revision": "fixture",
            "providers": [
                {"id":"relay", "models":[
                    {"id":"a", "limit":{"context": 100000}, "canonicalModelId":"official/a", "reasoningOptionsDeclared":true, "reasoningOptions":[], "interleaved":{"field":"reasoning_content"}},
                    {"id":"b", "limit":{"context": 80000}, "canonicalModelId":"official/missing"},
                    {"id":"c", "limit":{"context": 70000}, "canonicalModelId":"broken"}
                ]},
                {"id":"official", "models":[{"id":"a", "limit":{"context": 200000}}]},
                {"id":"deepseek", "models":[{"id":"deepseek-flash", "canonicalModelId":"deepseek/deepseek-v4.1-flash", "limit":{"context": 1000000}}]}
            ]
        }).to_string().as_bytes(), models_dev::Origin::Embedded).unwrap();
        let steps = vec![MatchStep {
            model_id: "a".into(),
            step: "exact".into(),
            matched: ["a", "b", "c"]
                .into_iter()
                .map(|model| models_dev::ModelRef {
                    provider_id: "relay".into(),
                    model_id: model.into(),
                })
                .collect(),
            note: String::new(),
        }];
        let hints = catalog_hints(&catalog, &steps);
        assert_eq!(
            hints
                .iter()
                .map(|hint| hint.canonical_status.as_str())
                .collect::<Vec<_>>(),
            vec!["verified", "missingTarget", "invalid"]
        );
        assert_eq!(hints[0].reasoning_options_presence, "declared");
        assert_eq!(hints[1].reasoning_options_presence, "unknown");
        assert_eq!(
            catalog.exact_in("relay", "a").unwrap().model.limit.context,
            Some(100000)
        );
        assert_eq!(
            catalog
                .exact_in("official", "a")
                .unwrap()
                .model
                .limit
                .context,
            Some(200000)
        );
        let alias_hint = catalog_hints(
            &catalog,
            &[MatchStep {
                model_id: "deepseek-v4.1-flash".into(),
                step: "canonicalIdentity".into(),
                matched: vec![models_dev::ModelRef {
                    provider_id: "deepseek".into(),
                    model_id: "deepseek-flash".into(),
                }],
                note: String::new(),
            }],
        );
        assert_eq!(alias_hint[0].canonical_status, "declaredAlias");
    }

    #[test]
    fn clear_dry_run_and_guarded_apply_share_the_resolver() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        paths.ensure_directories().unwrap();
        let mut entry = provider("relay", "fixture-model");
        entry.context_window = Some(200_000);
        entry.models[0].overrides.context_window = Some(300_000);
        let store = router::Store {
            providers: vec![entry],
            ..Default::default()
        };
        super::super::secret_file::write(
            &paths.app_data_dir.join("newapi-providers.dat"),
            &serde_json::to_vec(&store).unwrap(),
        )
        .unwrap();
        let loaded = router::load_store(&paths).unwrap();
        let dry_run = preview_clear(
            &paths,
            &loaded,
            "relay",
            "fixture-model",
            router::ClearCapabilityField::ContextWindow,
        )
        .unwrap();
        assert_eq!(dry_run.before.context.value, Some(300_000));
        assert_eq!(dry_run.before.context.source, "manual");
        assert_eq!(dry_run.after.context.value, Some(200_000));
        assert_eq!(dry_run.after.context.source, "providerSetting");
        assert_eq!(dry_run.scope_count, 1);
        assert_eq!(dry_run.affected_models[0].model_id, "fixture-model");
        router::clear_capability_if_current(
            &paths,
            "relay",
            "fixture-model",
            router::ClearCapabilityField::ContextWindow,
            &dry_run.config_revision,
            &dry_run.catalog_revision,
            &dry_run.official_revision,
        )
        .unwrap();
        let actual = preview(&paths, &router::load_store(&paths).unwrap());
        assert_eq!(actual.models[0].context.value, dry_run.after.context.value);
        assert!(router::clear_capability_if_current(
            &paths,
            "relay",
            "fixture-model",
            router::ClearCapabilityField::ContextWindow,
            &dry_run.config_revision,
            &dry_run.catalog_revision,
            &dry_run.official_revision
        )
        .is_err());
    }

    #[test]
    fn provider_level_clear_reports_every_affected_model() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        let mut entry = provider("relay", "model-one");
        entry.context_window = Some(200_000);
        entry.models.push(newapi::Model {
            id: "model-two".into(),
            display_name: "Second model".into(),
            ..Default::default()
        });
        let store = router::Store {
            providers: vec![entry],
            ..Default::default()
        };
        let dry_run = preview_clear(
            &paths,
            &store,
            "relay",
            "model-one",
            router::ClearCapabilityField::ProviderContextWindow,
        )
        .unwrap();
        assert_eq!(dry_run.before.context.value, Some(200_000));
        assert_ne!(dry_run.after.context.value, Some(200_000));
        assert_eq!(dry_run.scope_count, 2);
        assert_eq!(
            dry_run
                .affected_models
                .iter()
                .map(|model| model.model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["model-one", "model-two"]
        );
    }

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
        assert_eq!(
            preview(&paths, &store).models[0].compression_scope,
            "inactive"
        );
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
