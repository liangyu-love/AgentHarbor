use super::{model_catalog, model_defaults, router};
use crate::platform::paths::CodexPaths;
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field<T: Serialize> {
    pub value: T,
    pub source: String,
    pub updated_at: Option<i64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub provider_id: String,
    pub provider_name: String,
    pub display_name: String,
    pub upstream_model: String,
    pub matched_model: String,
    pub reasoning: Field<Vec<Value>>,
    pub default_reasoning: Option<String>,
    pub context: Field<Option<u64>>,
    pub output: Field<Option<u64>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub stale: bool,
    pub error: Option<String>,
    pub updated_at: Option<i64>,
    pub models: Vec<Preview>,
}
fn field<T: Serialize>(value: T, source: &str, updated_at: Option<i64>) -> Field<T> {
    Field {
        value,
        source: source.into(),
        updated_at,
    }
}
pub fn preview(paths: &CodexPaths, store: &router::Store) -> CatalogView {
    let snapshot = model_catalog::official(&paths.codex_home, &paths.app_data_dir);
    let catalog = model_defaults::Catalog::from_models(&snapshot.models);
    let mut models = Vec::new();
    for provider in &store.providers {
        let mut ids: Vec<&str> = provider.models.iter().map(|m| m.id.as_str()).collect();
        if !provider.model.is_empty() && !ids.contains(&provider.model.as_str()) {
            ids.push(&provider.model);
        }
        for id in ids {
            let resolved = router::resolved_model_metadata(provider, id, &catalog);
            let official = router::matched_official_model(&snapshot.models, id);
            let official_time = official
                .and_then(|m| m["slug"].as_str())
                .and_then(|slug| snapshot.model_updated_at.get(slug))
                .copied()
                .flatten();
            let channel = resolved.reasoning_metadata();
            let reasoning_source = router::model_reasoning_source(id, official, channel.as_ref());
            let (levels, default) = router::relay_reasoning_capabilities(id, reasoning_source);
            let shared = catalog.get(id);
            let known = model_defaults::named_defaults(id).is_some();
            let saved = provider.models.iter().find(|m| m.id == id);
            let manual = saved.is_some_and(|m| m.metadata_source == "manual");
            let source = if manual {
                "manual"
            } else if saved.is_some_and(|m| m.metadata_source == "catalog") {
                "catalog"
            } else if known {
                "local"
            } else {
                "upstream"
            };
            let time = if manual {
                Some(provider.updated_at)
            } else if !known {
                provider.models_fetched_at
            } else {
                None
            };
            let context_source = if manual || provider.context_window.is_some() {
                "manual"
            } else if id.to_ascii_lowercase().contains("claude") {
                "local"
            } else if shared.is_some() {
                "catalog"
            } else if saved.is_some_and(|m| m.context_window.is_some()) {
                source
            } else {
                "local"
            };
            let defaults = model_defaults::for_model(id);
            let unclamped_output = shared
                .and_then(|e| {
                    e.max_output_tokens
                        .or_else(|| e.context_window.map(|n| n.min(defaults.max_output_tokens)))
                })
                .or_else(|| {
                    if known && source != "catalog" {
                        Some(defaults.max_output_tokens)
                    } else {
                        saved.and_then(|m| m.max_output_tokens)
                    }
                });
            let output_source = if !manual
                && provider
                    .context_window
                    .is_some_and(|limit| unclamped_output.is_some_and(|output| output > limit))
            {
                "contextLimit"
            } else if manual && saved.is_some_and(|m| m.max_output_tokens.is_some()) {
                "manual"
            } else if shared.and_then(|e| e.max_output_tokens).is_some() {
                "catalog"
            } else if shared.is_some() {
                "local"
            } else if saved.is_some_and(|m| m.max_output_tokens.is_some()) {
                source
            } else {
                "local"
            };
            let declared_reasoning = reasoning_source
                .and_then(|m| m.get("supported_reasoning_levels"))
                .and_then(Value::as_array)
                .is_some_and(|levels| {
                    levels.iter().all(|level| {
                        level["effort"].as_str().is_some_and(|s| !s.is_empty())
                            && level["description"].as_str().is_some()
                    })
                });
            let reasoning_origin = if declared_reasoning {
                if official.is_some() {
                    "catalog"
                } else {
                    "upstream"
                }
            } else if !levels.is_empty() {
                "local"
            } else {
                "unknown"
            };
            let timestamp = |source| match source {
                "catalog" => official_time,
                "manual" | "contextLimit" => Some(provider.updated_at),
                "upstream" => time,
                _ => None,
            };
            models.push(Preview {
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                display_name: resolved.display_name,
                upstream_model: id.into(),
                matched_model: model_defaults::model_key(id),
                reasoning: field(levels, reasoning_origin, timestamp(reasoning_origin)),
                default_reasoning: default,
                context: field(
                    resolved.context_window,
                    context_source,
                    timestamp(context_source),
                ),
                output: field(
                    resolved.max_output_tokens,
                    output_source,
                    timestamp(output_source),
                ),
            });
        }
    }
    CatalogView {
        stale: snapshot.stale,
        error: snapshot.error,
        updated_at: snapshot.updated_at,
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channel_aliases_never_change_the_upstream_or_capability_identity() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        let provider = router::Provider {
            id: "relay".into(),
            name: "Channel".into(),
            model: "claude-opus-5-5-kiro".into(),
            models: vec![super::super::newapi::Model {
                id: "claude-opus-5-5-kiro".into(),
                display_name: "My alias".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let view = preview(
            &paths,
            &router::Store {
                providers: vec![provider],
                ..Default::default()
            },
        );
        assert_eq!(view.models[0].display_name, "My alias");
        assert_eq!(view.models[0].upstream_model, "claude-opus-5-5-kiro");
        assert_eq!(view.models[0].matched_model, "claude-opus-5-5");
        assert_eq!(view.models[0].context.value, Some(1_000_000));
        assert_eq!(view.models[0].context.source, "local");
    }

    #[test]
    fn preview_and_router_share_official_fallback_and_per_field_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().into());
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(paths.app_data_dir.join("previous-router-catalog.json"),r#"{"models":[{"slug":"gpt-6-astra","context_window":900000,"max_output_tokens":64000,"supported_reasoning_levels":[{"effort":"high","description":"High"}]}]}"#).unwrap();
        let provider = router::Provider {
            id: "a".into(),
            model: "gpt-6-astra".into(),
            api_key: "fixture".into(),
            enabled: true,
            ..Default::default()
        };
        let store = router::Store {
            providers: vec![provider],
            ..Default::default()
        };
        let preview = preview(&paths, &store);
        let catalog = router::catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        let row = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["slug"] == "a::gpt-6-astra")
            .unwrap();
        assert_eq!(
            preview.models[0].context.value,
            Some(row["context_window"].as_u64().unwrap())
        );
        assert_eq!(
            preview.models[0].reasoning.value,
            row["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .clone()
        );
        assert_eq!(preview.models[0].output.source, "catalog");
        assert!(preview.models[0].output.updated_at.is_some());
        assert!(preview.stale);
    }

    #[test]
    fn official_limits_without_reasoning_declaration_keep_reasoning_source_local() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        std::fs::write(
            paths.codex_home.join("models_cache.json"),
            r#"{"models":[{"slug":"gpt-6-astra","context_window":900000,"max_output_tokens":64000}]} "#,
        )
        .unwrap();
        let provider = router::Provider {
            id: "catalog".into(),
            model: "gpt-6-astra".into(),
            models: vec![super::super::newapi::Model {
                id: "gpt-6-astra".into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let view = preview(
            &paths,
            &router::Store {
                providers: vec![provider],
                ..Default::default()
            },
        );

        assert_eq!(view.models[0].context.value, Some(900_000));
        assert_eq!(view.models[0].context.source, "catalog");
        assert_eq!(view.models[0].output.value, Some(64_000));
        assert_eq!(view.models[0].output.source, "catalog");
        assert_eq!(view.models[0].reasoning.source, "local");
        assert!(!view.models[0].reasoning.value.is_empty());
    }

    #[test]
    fn provider_context_limit_is_reported_as_output_constraint() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        let provider = router::Provider {
            id: "limited".into(),
            model: "gpt-6-astra".into(),
            context_window: Some(4096),
            updated_at: 12345,
            models: vec![super::super::newapi::Model {
                id: "gpt-6-astra".into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let view = preview(
            &paths,
            &router::Store {
                providers: vec![provider],
                ..Default::default()
            },
        );

        assert_eq!(view.models[0].context.value, Some(4096));
        assert_eq!(view.models[0].context.source, "manual");
        assert_eq!(view.models[0].output.value, Some(4096));
        assert_eq!(view.models[0].output.source, "contextLimit");
        assert_eq!(view.models[0].output.updated_at, Some(12345));
    }

    #[test]
    fn saved_catalog_metadata_keeps_catalog_provenance_when_refresh_lacks_model() {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_path_buf());
        let provider = router::Provider {
            id: "saved".into(),
            model: "vendor-model".into(),
            models: vec![super::super::newapi::Model {
                id: "vendor-model".into(),
                display_name: "Saved vendor".into(),
                context_window: Some(100_000),
                max_output_tokens: Some(20_000),
                metadata_source: "catalog".into(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let view = preview(
            &paths,
            &router::Store {
                providers: vec![provider],
                ..Default::default()
            },
        );

        assert_eq!(view.models[0].display_name, "Saved vendor");
        assert_eq!(view.models[0].context.value, Some(100_000));
        assert_eq!(view.models[0].context.source, "catalog");
        assert_eq!(view.models[0].output.value, Some(20_000));
        assert_eq!(view.models[0].output.source, "catalog");
    }
}
