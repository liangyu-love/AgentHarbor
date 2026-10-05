//! Read-only resolutions for an unsaved provider editor draft. This adapter
//! supplies the shared metadata resolver and client projection with local
//! sources; it never saves a provider or inspects client application state.
use super::{model_catalog, model_metadata, model_policy, models_dev, newapi, router, secret_file};
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MAX_MODELS: usize = 512;
const MAX_STORE_BYTES: u64 = 32 * 1024 * 1024;

/// Deliberately excludes provider credentials and unrelated card settings.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftProviderModels {
    pub provider_id: Option<String>,
    /// Credential availability only; the editor never sends its draft key.
    #[serde(default)]
    pub has_draft_api_key: bool,
    pub base_url: String,
    pub protocol: router::ModelProtocol,
    pub context_window: Option<u64>,
    pub catalog_providers: Vec<String>,
    pub compression: model_policy::CompressionPolicy,
    pub models: Vec<newapi::Model>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftModelResolution {
    pub model_id: String,
    pub match_kind: model_metadata::MatchKind,
    pub matched: Option<models_dev::ModelRef>,
    pub context: model_metadata::Field<u64>,
    pub input_limit: model_metadata::Field<u64>,
    pub output: model_metadata::Field<u64>,
    pub projection: model_policy::ClientProjection,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftModelResolutions {
    pub models: Vec<DraftModelResolution>,
}

fn bounded_text(text: &str, max: usize) -> bool {
    text.len() <= max && !text.chars().any(char::is_control)
}

impl DraftProviderModels {
    fn validate(&self) -> Result<(), String> {
        if self.models.len() > MAX_MODELS {
            return Err("newapi.errors.tooManyModels".into());
        }
        if !bounded_text(&self.base_url, 4096)
            || self
                .provider_id
                .as_ref()
                .is_some_and(|id| !bounded_text(id, 128))
            || self.catalog_providers.len() > 8
            || self
                .catalog_providers
                .iter()
                .any(|id| !bounded_text(id, 64))
        {
            return Err("newapi.errors.modelList".into());
        }
        // An empty/incomplete URL is normal while editing. Family matching
        // can already resolve a model without requiring a valid endpoint.
        newapi::validate_limits(&newapi::Connection {
            context_window: self.context_window,
            ..Default::default()
        })?;
        if self.compression.validate().is_err() {
            return Err("newapi.errors.context".into());
        }
        let mut ids = HashSet::new();
        for model in &self.models {
            if model.id.trim().is_empty()
                || !bounded_text(&model.id, 200)
                || !bounded_text(&model.display_name, 120)
                || !ids.insert(&model.id)
                || model.endpoint_types.len() > 16
                || model
                    .endpoint_types
                    .iter()
                    .any(|value| !bounded_text(value, 80))
                || !bounded_text(&model.metadata_source, 32)
            {
                return Err("newapi.errors.modelList".into());
            }
            // Reuse the provider limit checks without normalizing or promoting
            // draft values into saved manual overrides.
            newapi::validate_limits(&newapi::Connection {
                model: model.id.clone(),
                context_window: model.context_window,
                ..Default::default()
            })?;
            let values = &model.overrides;
            if values
                .context_window
                .is_some_and(|v| !(1_024..=100_000_000).contains(&v))
                || values
                    .input_limit
                    .is_some_and(|v| !(1_024..=100_000_000).contains(&v))
                || [values.max_output_tokens, model.max_output_tokens]
                    .iter()
                    .any(|v| v.is_some_and(|v| v == 0 || v > 100_000_000))
                || matches!((values.context_window, values.max_output_tokens), (Some(c), Some(o)) if o > c)
                || matches!((model.context_window, model.max_output_tokens), (Some(c), Some(o)) if o > c)
                || model.compression.validate().is_err()
            {
                return Err("newapi.errors.context".into());
            }
            if [
                values.reasoning.as_ref(),
                model.supported_reasoning_levels.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|levels| levels.len() > 32 || !levels.iter().all(newapi::ReasoningLevel::valid))
                || [&values.default_effort, &model.default_reasoning_level]
                    .into_iter()
                    .flatten()
                    .any(|value| !bounded_text(value, 32))
                || [&model.catalog_ref, &model.price_ref]
                    .into_iter()
                    .flatten()
                    .any(|reference| {
                        !bounded_text(&reference.provider_id, 64)
                            || !bounded_text(&reference.model_id, 256)
                    })
            {
                return Err("newapi.errors.modelList".into());
            }
            if values.price.as_ref().is_some_and(|price| {
                [
                    price.input,
                    price.output,
                    price.cache_read,
                    price.cache_write,
                ]
                .into_iter()
                .flatten()
                .any(|value| !value.is_finite() || !(0.0..=1_000_000.0).contains(&value))
            }) {
                return Err("newapi.errors.context".into());
            }
        }
        Ok(())
    }

    fn provider(&self, store: &router::Store) -> router::Provider {
        let saved = self
            .provider_id
            .as_ref()
            .and_then(|id| store.providers.iter().find(|provider| &provider.id == id));
        // Saved eligibility is part of the shared failover pool. A new
        // provider remains disabled, matching save_provider's initial state.
        let mut provider = saved.cloned().unwrap_or_default();
        provider.id = self.provider_id.clone().unwrap_or_default();
        let same_site = saved.is_some_and(|saved| {
            newapi::base_url(&saved.base_url)
                .ok()
                .zip(newapi::base_url(&self.base_url).ok())
                .is_some_and(|(saved, draft)| saved == draft)
        });
        if self.has_draft_api_key {
            // Projection only checks whether a key exists. This placeholder
            // is never used for an HTTP request or persisted.
            provider.api_key = "draft-preview-credential".into();
        } else if !same_site {
            provider.api_key.clear();
        }
        provider.base_url = self.base_url.clone();
        provider.protocol = self.protocol;
        provider.context_window = self.context_window;
        provider.catalog_providers = self.catalog_providers.clone();
        provider.compression = self.compression.clone();
        // The editor saves explicit compression and no legacy threshold.
        provider.auto_compact_limit = None;
        provider.models = self.models.clone();
        provider
    }
}

/// The stored configuration supplies global policy and failover eligibility.
/// Avoid load_store's identity/metadata migrations and disk writes; no client
/// application state or running router is needed for this projection.
fn load_store_read_only(paths: &CodexPaths) -> Result<router::Store, String> {
    let path = paths.app_data_dir.join("newapi-providers.dat");
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.len() > MAX_STORE_BYTES => {
            return Err("newapi.errors.storage".into())
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err("newapi.errors.storage".into())
        }
        _ => {}
    }
    let store: router::Store = match secret_file::read(&path)? {
        Some(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.storage".to_owned())?
        }
        None => router::Store::default(),
    };
    store
        .compression
        .validate()
        .map_err(|_| "newapi.errors.storage".to_owned())?;
    Ok(store)
}

pub fn preview(
    paths: &CodexPaths,
    draft: DraftProviderModels,
) -> Result<DraftModelResolutions, String> {
    draft.validate()?;
    let store = load_store_read_only(paths)?;
    let official = model_catalog::official_read_only(&paths.codex_home, &paths.app_data_dir);
    let catalog = models_dev::load(&paths.app_data_dir);
    Ok(resolve_with_sources(
        &draft,
        &store,
        &official.models,
        &catalog,
    ))
}

fn resolve_with_sources(
    draft: &DraftProviderModels,
    store: &router::Store,
    official: &[serde_json::Value],
    catalog: &models_dev::Catalog,
) -> DraftModelResolutions {
    let provider = draft.provider(store);
    let global = store.global_compression();
    let models = draft
        .models
        .iter()
        .map(|model| {
            let official = router::matched_official_model(official, &model.id)
                .filter(|_| router::official_endpoint_applies(&provider));
            let (metadata, projection) =
                if store.codex_access_mode == router::CodexAccessMode::Native {
                    router::model_projection(&provider, &model.id, official, catalog, &global)
                } else {
                    router::relay_model_projection(
                        store, &provider, &model.id, official, catalog, &global,
                    )
                };
            DraftModelResolution {
                model_id: model.id.clone(),
                match_kind: metadata.match_kind,
                matched: metadata.matched,
                context: metadata.context_window,
                input_limit: metadata.input_limit,
                output: metadata.output_limit,
                projection,
            }
        })
        .collect();
    DraftModelResolutions { models }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn draft() -> DraftProviderModels {
        DraftProviderModels {
            provider_id: None,
            has_draft_api_key: false,
            base_url: String::new(),
            protocol: router::ModelProtocol::Responses,
            context_window: None,
            catalog_providers: vec![],
            compression: model_policy::CompressionPolicy::Inherit,
            models: vec![newapi::Model {
                id: "gpt-6-luna".into(),
                included: false,
                ..Default::default()
            }],
        }
    }

    fn catalog() -> models_dev::Catalog {
        models_dev::Catalog::from_snapshot(
            models_dev::Snapshot {
                schema_version: models_dev::SCHEMA_VERSION,
                revision: "fixture".into(),
                providers: vec![models_dev::Provider {
                    id: "openai".into(),
                    name: "OpenAI".into(),
                    models: vec![models_dev::Model {
                        id: "gpt-6-luna".into(),
                        name: "GPT 6 Luna".into(),
                        limit: models_dev::Limit {
                            context: Some(400_000),
                            input: Some(272_000),
                            output: Some(128_000),
                        },
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            },
            models_dev::Origin::Embedded,
        )
        .unwrap()
    }

    fn store(global: &model_policy::CompressionPolicy) -> router::Store {
        router::Store {
            compression: global.clone(),
            ..Default::default()
        }
    }

    fn failover_store() -> router::Store {
        let mut current = draft().provider(&router::Store::default());
        current.id = "current".into();
        current.base_url = "https://relay.example/".into();
        current.enabled = true;
        current.api_key = "stored-test-key".into();
        current.models[0].included = true;
        let mut peer = current.clone();
        peer.id = "peer".into();
        peer.models[0].overrides.input_limit = Some(128_000);
        router::Store {
            providers: vec![current, peer],
            router: router::RouterSettings {
                same_model_failover: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn saved_draft_matches_failover_projection_and_native_stays_model_scoped() {
        let mut draft = draft();
        draft.provider_id = Some("current".into());
        draft.base_url = "https://relay.example/v1".into();
        let mut store = failover_store();
        let catalog = catalog();
        let resolved = resolve_with_sources(&draft, &store, &[], &catalog);
        assert_eq!(
            resolved.models[0].projection.compression.threshold,
            Some(115_200)
        );
        assert_eq!(resolved.models[0].projection.budget_source, "failover");
        let (_, saved_projection) = router::relay_model_projection(
            &store,
            &store.providers[0],
            "gpt-6-luna",
            None,
            &catalog,
            &store.global_compression(),
        );
        assert_eq!(
            serde_json::to_value(&resolved.models[0].projection).unwrap(),
            serde_json::to_value(saved_projection).unwrap()
        );
        store.codex_access_mode = router::CodexAccessMode::Native;
        let native = resolve_with_sources(&draft, &store, &[], &catalog);
        assert_eq!(
            native.models[0].projection.compression.threshold,
            Some(244_800)
        );
        store.codex_access_mode = router::CodexAccessMode::Router;
        store.providers[1].models[0].protocol = Some(router::ModelProtocol::ChatCompletions);
        let incompatible = resolve_with_sources(&draft, &store, &[], &catalog);
        assert_eq!(
            incompatible.models[0].projection.compression.threshold,
            Some(244_800)
        );
        assert_eq!(store.providers[0].api_key, "stored-test-key");
    }

    #[test]
    fn draft_key_availability_and_saved_enable_state_control_failover_eligibility() {
        let mut draft = draft();
        draft.provider_id = Some("current".into());
        draft.base_url = "https://another.example/v1".into();
        let mut store = failover_store();
        store.providers[0].auto_compact_limit = Some(50_000);
        let unavailable = resolve_with_sources(&draft, &store, &[], &catalog());
        assert_eq!(
            unavailable.models[0].projection.compression.threshold,
            Some(244_800)
        );
        assert!(draft.provider(&store).api_key.is_empty());
        assert_eq!(draft.provider(&store).auto_compact_limit, None);
        draft.has_draft_api_key = true;
        let available = resolve_with_sources(&draft, &store, &[], &catalog());
        assert_eq!(
            available.models[0].projection.compression.threshold,
            Some(115_200)
        );
        store.providers[0].enabled = false;
        let disabled = resolve_with_sources(&draft, &store, &[], &catalog());
        assert_eq!(
            disabled.models[0].projection.compression.threshold,
            Some(244_800)
        );
        draft.provider_id = None;
        let new_provider = resolve_with_sources(&draft, &store, &[], &catalog());
        assert_eq!(new_provider.models[0].context.value, Some(400_000));
        assert_eq!(
            new_provider.models[0].projection.compression.threshold,
            Some(244_800)
        );
        assert!(!draft.provider(&store).enabled);
    }

    #[test]
    fn omitted_draft_key_and_old_store_defaults_keep_the_shared_global_default() {
        let draft: DraftProviderModels = serde_json::from_value(json!({
            "baseUrl":"", "protocol":"responses", "contextWindow":null,
            "catalogProviders":[], "compression":{"mode":"inherit"}, "models":[]
        }))
        .unwrap();
        assert!(!draft.has_draft_api_key);
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("home"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let path = paths.app_data_dir.join("newapi-providers.dat");
        secret_file::write(&path, br#"{"version":1,"providers":[]}"#).unwrap();
        let original = std::fs::read(&path).unwrap();
        let store = load_store_read_only(&paths).unwrap();
        assert_eq!(
            store.global_compression(),
            model_policy::CompressionPolicy::percent(90)
        );
        assert_eq!(store.codex_access_mode, router::CodexAccessMode::Router);
        assert!(!store.router.same_model_failover);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn pending_model_uses_shared_resolution_and_real_prompt_budget() {
        let draft = draft();
        draft.validate().unwrap();
        let catalog = catalog();
        let global = model_policy::CompressionPolicy::percent(90);
        let resolved = resolve_with_sources(&draft, &store(&global), &[], &catalog);
        let entry = &resolved.models[0];
        assert_eq!(entry.context.value, Some(400_000));
        assert_eq!(entry.input_limit.value, Some(272_000));
        assert_eq!(entry.matched.as_ref().unwrap().provider_id, "openai");
        assert_eq!(entry.projection.compression.threshold, Some(244_800));
        assert_eq!(entry.projection.compression.source, "global");
        let (metadata, projection) = router::model_projection(
            &draft.provider(&store(&global)),
            "gpt-6-luna",
            None,
            &catalog,
            &global,
        );
        assert_eq!(
            serde_json::to_value(&entry.context).unwrap(),
            serde_json::to_value(metadata.context_window).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&entry.projection).unwrap(),
            serde_json::to_value(projection).unwrap()
        );
        assert!(!draft.models[0].included);
        assert!(draft.models[0].overrides.is_empty());
    }

    #[test]
    fn speed_variant_preview_uses_the_shared_context_reference_and_keeps_its_raw_id() {
        let mut draft = draft();
        draft.models[0].id = "openai/gpt-6-luna-fast".into();
        let global = model_policy::CompressionPolicy::percent(90);
        let resolved = resolve_with_sources(&draft, &store(&global), &[], &catalog());
        let entry = &resolved.models[0];
        assert_eq!(entry.model_id, "openai/gpt-6-luna-fast");
        assert_eq!(entry.match_kind, model_metadata::MatchKind::Normalized);
        assert_eq!(entry.context.value, Some(400_000));
        assert_eq!(
            entry.context.source,
            model_metadata::source::CATALOG_SPEED_REFERENCE
        );
        assert!(entry.input_limit.value.is_none());
        assert!(entry.output.value.is_none());
        assert_eq!(entry.projection.compression.threshold, Some(360_000));
        assert_eq!(
            entry.projection.output_limit,
            Some(model_metadata::FALLBACK_MAX_OUTPUT_TOKENS)
        );
        assert_eq!(
            entry.projection.output_source,
            model_metadata::source::FALLBACK
        );
        assert!(entry.projection.reasoning.is_empty());
        let (metadata, projection) = router::model_projection(
            &draft.provider(&store(&global)),
            &entry.model_id,
            None,
            &catalog(),
            &global,
        );
        assert_eq!(
            serde_json::to_value(&entry.context).unwrap(),
            serde_json::to_value(metadata.context_window).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&entry.projection).unwrap(),
            serde_json::to_value(projection).unwrap()
        );
        assert_eq!(draft.models[0].id, "openai/gpt-6-luna-fast");
        assert!(draft.models[0].overrides.is_empty());
    }

    #[test]
    fn unknown_or_invalid_reference_never_fabricates_matching_information() {
        let mut draft = draft();
        draft.models[0].id = "opaque-private-model".into();
        let global = model_policy::CompressionPolicy::percent(90);
        let resolved = resolve_with_sources(&draft, &store(&global), &[], &catalog());
        assert_eq!(
            resolved.models[0].match_kind,
            model_metadata::MatchKind::Unmatched
        );
        assert!(resolved.models[0].matched.is_none());
        assert!(resolved.models[0].context.value.is_none());
        assert!(resolved.models[0].projection.context_is_fallback);
        draft.models[0].id = "gpt-6-luna".into();
        draft.models[0].catalog_ref = Some(models_dev::ModelRef {
            provider_id: "missing".into(),
            model_id: "missing".into(),
        });
        let resolved = resolve_with_sources(&draft, &store(&global), &[], &catalog());
        assert_eq!(
            resolved.models[0].match_kind,
            model_metadata::MatchKind::Invalid
        );
        assert!(resolved.models[0].matched.is_none());
        assert!(resolved.models[0].context.value.is_none());
    }

    #[test]
    fn drafts_resolve_without_creating_store_or_catalog_backup() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("home"));
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        let source = paths.codex_home.join("models_cache.json");
        let bytes =
            serde_json::to_vec(&json!({"models":[{"slug":"gpt-6-luna","context_window":999000}]}))
                .unwrap();
        std::fs::write(&source, &bytes).unwrap();
        let mut draft = draft();
        draft.base_url = "https://api.openai.com/v1".into();
        let resolved = preview(&paths, draft).unwrap();
        assert_eq!(resolved.models[0].context.value, Some(999_000));
        assert_eq!(
            resolved.models[0].context.source,
            model_metadata::source::OFFICIAL
        );
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert!(!paths.app_data_dir.exists());
    }

    #[test]
    fn stored_global_policy_is_read_without_migration_and_corruption_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().join("home"));
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let path = paths.app_data_dir.join("newapi-providers.dat");
        secret_file::write(
            &path,
            br#"{"version":1,"compression":{"mode":"percent","percent":75}}"#,
        )
        .unwrap();
        let original = std::fs::read(&path).unwrap();
        assert_eq!(
            load_store_read_only(&paths).unwrap().global_compression(),
            model_policy::CompressionPolicy::percent(75)
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(!paths
            .app_data_dir
            .join("newapi-providers.pre-v8.dat")
            .exists());
        secret_file::write(&path, b"{corrupt").unwrap();
        assert_eq!(
            load_store_read_only(&paths).err().unwrap(),
            "newapi.errors.storage"
        );
    }

    #[test]
    fn official_account_values_are_only_used_for_the_official_endpoint() {
        let mut draft = draft();
        let official = vec![json!({"slug":"gpt-6-luna","context_window":999000})];
        let global = model_policy::CompressionPolicy::percent(90);
        draft.base_url = "https://relay.example/v1".into();
        let relay = resolve_with_sources(&draft, &store(&global), &official, &catalog());
        assert_eq!(relay.models[0].context.value, Some(400_000));
        draft.base_url = "https://api.openai.com/v1".into();
        let official = resolve_with_sources(&draft, &store(&global), &official, &catalog());
        assert_eq!(official.models[0].context.value, Some(999_000));
    }
}
