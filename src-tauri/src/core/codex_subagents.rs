//! Codex's two optional agent defaults, projected through the active catalog.
//! Config writes and recovery belong to the existing connection journal.
use super::*;
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item};

const MODEL: &str = "default_subagent_model";
const EFFORT: &str = "default_subagent_reasoning_effort";
const EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChoice {
    pub id: String,
    pub label: String,
    pub reasoning_efforts: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub settings: Settings,
    pub managed: bool,
    pub applied: bool,
    pub models: Vec<ModelChoice>,
    pub inherited_reasoning_efforts: Vec<String>,
}

fn catalog(paths: &CodexPaths, store: &Store) -> Result<Value, String> {
    if store.codex_access_mode == CodexAccessMode::Native {
        return match native_config::subagent_catalog(paths, store) {
            Ok(catalog) => Ok(catalog),
            // An unavailable selection must still allow restoring the saved
            // defaults from the UI. Application retains strict validation.
            Err(error)
                if matches!(
                    error.as_str(),
                    "newapi.errors.noActiveProvider" | "newapi.errors.nativeResponsesRequired"
                ) =>
            {
                Ok(serde_json::json!({"models": []}))
            }
            Err(error) => Err(error),
        };
    }
    Ok(if store.router.aggregate_model_id.is_some() {
        aggregate_catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    } else {
        catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    })
}

fn choices(paths: &CodexPaths, store: &Store) -> Result<Vec<ModelChoice>, String> {
    Ok(catalog(paths, store)?["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|model| model["visibility"] != "hide" && model["multi_agent_version"] != "disabled")
        .filter_map(|model| {
            let id = model["slug"].as_str()?;
            Some(ModelChoice {
                id: id.into(),
                label: model["display_name"].as_str().unwrap_or(id).into(),
                reasoning_efforts: model["supported_reasoning_levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|level| level["effort"].as_str())
                    .filter(|effort| EFFORTS.contains(effort))
                    .map(str::to_owned)
                    .collect(),
            })
        })
        .collect())
}

fn inherited_model<'a>(
    store: &Store,
    document: &DocumentMut,
    models: &'a [ModelChoice],
) -> Option<&'a ModelChoice> {
    let id = if store.codex_access_mode == CodexAccessMode::Native {
        store
            .providers
            .iter()
            .find(|provider| Some(&provider.id) == store.active_provider_id.as_ref())
            .or_else(|| {
                store
                    .providers
                    .iter()
                    .find(|provider| provider.is_routable())
            })
            .map(|provider| provider.model.clone())
    } else {
        codex_connection(store).ok().map(|(provider, default, _)| {
            document
                .get("model")
                .and_then(Item::as_str)
                .filter(|model| {
                    document.get("model_provider").and_then(Item::as_str) == Some(provider)
                        && models.iter().any(|choice| choice.id == *model)
                })
                .map(str::to_owned)
                .unwrap_or(default)
        })
    };
    models.iter().find(|model| Some(&model.id) == id.as_ref())
}

pub fn load(paths: &CodexPaths) -> Result<View, String> {
    let store = load_store(paths)?;
    let (_, document) = open_config(paths)?;
    let actual = read(&document).map_err(config_update_error)?;
    let models = choices(paths, &store)?;
    let inherited_reasoning_efforts = inherited_model(&store, &document, &models)
        .map(|model| model.reasoning_efforts.clone())
        .unwrap_or_default();
    Ok(View {
        settings: store
            .codex_subagents
            .clone()
            .unwrap_or_else(|| actual.clone()),
        managed: store.codex_subagents.is_some(),
        applied: store.router.enabled
            && super::route_is_ours(&document)
            && store.codex_subagents.as_ref() == Some(&actual),
        models,
        inherited_reasoning_efforts,
    })
}

pub fn save(paths: &CodexPaths, settings: Option<Settings>) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let previous = load_store_unlocked(paths)?;
    let mut store = previous.clone();
    store.codex_subagents = settings;
    validate(paths, &store)?;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(
            &store,
            super::super::router_proxy::is_running(),
            Some(paths),
        ),
        applied,
    })
}

pub(super) fn validate(paths: &CodexPaths, store: &Store) -> Result<(), String> {
    let Some(settings) = &store.codex_subagents else {
        return Ok(());
    };
    if settings
        .reasoning_effort
        .as_deref()
        .is_some_and(|effort| !EFFORTS.contains(&effort))
    {
        return Err("newapi.errors.subagentEffort".into());
    }
    if let Some(id) = &settings.model {
        let models = choices(paths, store)?;
        let model = models
            .iter()
            .find(|model| model.id == *id)
            .ok_or("newapi.errors.subagentModel")?;
        if settings
            .reasoning_effort
            .as_ref()
            .is_some_and(|effort| !model.reasoning_efforts.contains(effort))
        {
            return Err("newapi.errors.subagentEffort".into());
        }
    } else if let Some(effort) = &settings.reasoning_effort {
        let models = choices(paths, store)?;
        let (_, document) = open_config(paths)?;
        if inherited_model(store, &document, &models)
            .is_none_or(|model| !model.reasoning_efforts.contains(effort))
        {
            return Err("newapi.errors.subagentEffort".into());
        }
    }
    Ok(())
}

pub(super) fn matches(document: &DocumentMut, store: &Store) -> bool {
    store
        .codex_subagents
        .as_ref()
        .is_none_or(|expected| read(document).is_ok_and(|value| value == *expected))
}

fn syntax_error() -> CoreError {
    CoreError::OperationFailed("newapi.errors.subagentConfig".into())
}

pub(super) fn read(document: &DocumentMut) -> Result<Settings, CoreError> {
    let Some(agents) = document.get("agents") else {
        return Ok(Settings::default());
    };
    let table = agents.as_table_like().ok_or_else(syntax_error)?;
    let field = |key| {
        table
            .get(key)
            .map(|value| value.as_str().map(str::to_owned).ok_or_else(syntax_error))
            .transpose()
    };
    Ok(Settings {
        model: field(MODEL)?,
        reasoning_effort: field(EFFORT)?,
    })
}

fn write(document: &mut DocumentMut, settings: &Settings) -> Result<(), CoreError> {
    read(document)?;
    if document.get("agents").is_none() {
        if *settings == Settings::default() {
            return Ok(());
        }
        document["agents"] = Item::Table(toml_edit::Table::new());
    }
    let table = document["agents"]
        .as_table_like_mut()
        .ok_or_else(syntax_error)?;
    for (key, value) in [
        (MODEL, &settings.model),
        (EFFORT, &settings.reasoning_effort),
    ] {
        if let Some(value) = value {
            let mut item = toml_edit::Value::from(value.clone());
            if let Some(current) = table.get(key).and_then(Item::as_value) {
                *item.decor_mut() = current.decor().clone();
            }
            table.insert(key, Item::Value(item));
        } else {
            table.remove(key);
        }
    }
    if table.is_empty() {
        document.remove("agents");
    }
    Ok(())
}

/// Nonsecret journal projection of exactly the two owned settings.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ownership {
    original: String,
    applied: Settings,
}

impl Ownership {
    pub(super) fn matches(&self, settings: &Settings) -> bool {
        &self.applied == settings
    }

    pub(super) fn apply(
        current: Option<&Self>,
        document: &mut DocumentMut,
        settings: &Settings,
    ) -> Result<Self, CoreError> {
        read(document)?;
        let original = if let Some(current) = current {
            current.original.clone()
        } else {
            let mut projection = DocumentMut::new();
            for key in [MODEL, EFFORT] {
                if let Some(item) = document.get("agents").and_then(|agents| agents.get(key)) {
                    projection[key] = item.clone();
                }
            }
            projection.to_string()
        };
        write(document, settings)?;
        Ok(Self {
            original,
            applied: settings.clone(),
        })
    }

    pub(super) fn validate(&self) -> Result<(), CoreError> {
        let original = self
            .original
            .parse::<DocumentMut>()
            .map_err(|_| syntax_error())?;
        if original
            .iter()
            .any(|(key, item)| ![MODEL, EFFORT].contains(&key) || item.as_str().is_none())
        {
            return Err(syntax_error());
        }
        Ok(())
    }

    pub(super) fn restore(&self, document: &mut DocumentMut) -> Result<(), CoreError> {
        self.validate()?;
        let current = read(document)?;
        let original = self
            .original
            .parse::<DocumentMut>()
            .map_err(|_| syntax_error())?;
        let mut restored = current.clone();
        if current.model == self.applied.model {
            restored.model = original
                .get(MODEL)
                .and_then(Item::as_str)
                .map(str::to_owned);
        }
        if current.reasoning_effort == self.applied.reasoning_effort {
            restored.reasoning_effort = original
                .get(EFFORT)
                .and_then(Item::as_str)
                .map(str::to_owned);
        }
        write(document, &restored)?;
        // Restore original scalar comments without touching unrelated agents keys.
        if let Some(agents) = document.get_mut("agents").and_then(Item::as_table_like_mut) {
            for (key, owned) in [
                (MODEL, current.model == self.applied.model),
                (
                    EFFORT,
                    current.reasoning_effort == self.applied.reasoning_effort,
                ),
            ] {
                if owned {
                    if let Some(item) = original.get(key) {
                        agents.insert(key, item.clone());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "codex_subagents_tests.rs"]
mod tests;
