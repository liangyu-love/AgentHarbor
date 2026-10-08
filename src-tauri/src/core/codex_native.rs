//! Direct Responses configuration. Credentials live only in Codex's private
//! .env file; the reversible config journal contains nonsecret root settings.

use super::{
    applied_config, config_snapshot, config_update_error, model_provider_is_ours,
    reject_foreign_managed_provider_entries, remove_managed_provider_entries, ModelProtocol,
    Provider, Store,
};
use crate::{
    core::{
        codex_config::CodexConfigRepository, model_catalog, model_policy, models::CoreError,
        models_dev, newapi, storage,
    },
    platform::paths::CodexPaths,
};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Cursor, ops::Range, path::PathBuf};
use toml_edit::{DocumentMut, Item};
use uuid::Uuid;

pub(super) const PROVIDER_KEY: &str = "agentharbor_native";
const ENV_PREFIX: &str = "AGENTHARBOR_CODEX_NATIVE_";
const BEGIN: &str = "# --- AgentHarbor Codex native v1 ";
const END: &str = "# --- End AgentHarbor Codex native v1 ";
const CREDENTIAL_ERROR: &str = "newapi.errors.nativeCredentials";
const STORAGE_ERROR: &str = "newapi.errors.storage";
const MAX_ENV_BYTES: u64 = 2 * 1024 * 1024;

// Deliberately no Debug or Serialize: these values contain API credentials.
struct OwnedCredential {
    id: Uuid,
    name: String,
    key: String,
    range: Range<usize>,
}

struct EnvFile {
    original: Option<Vec<u8>>,
    text: String,
    owned: Vec<OwnedCredential>,
}

fn env_path(paths: &CodexPaths) -> PathBuf {
    paths.codex_home.join(".env")
}

fn variable_name(id: Uuid) -> String {
    format!(
        "{ENV_PREFIX}{}",
        id.simple().to_string().to_ascii_uppercase()
    )
}

fn catalog_path(paths: &CodexPaths, id: Uuid) -> PathBuf {
    paths.app_data_dir.join(format!("codex-native-{id}.json"))
}

fn installed_catalog_path(paths: &CodexPaths, document: &DocumentMut) -> Option<PathBuf> {
    let path = PathBuf::from(document.get("model_catalog_json")?.as_str()?);
    let id = path
        .file_name()?
        .to_str()?
        .strip_prefix("codex-native-")?
        .strip_suffix(".json")?;
    let id = Uuid::parse_str(id).ok()?;
    (path == catalog_path(paths, id)).then_some(path)
}

fn credential_error() -> CoreError {
    CoreError::OperationFailed(CREDENTIAL_ERROR.into())
}

fn checksum(name: &str, key: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(name.as_bytes());
    digest.update([0]);
    digest.update(key.as_bytes());
    format!("{:x}", digest.finalize())
}

fn quote_key(key: &str) -> String {
    let mut value = String::from("\"");
    for character in key.chars() {
        if matches!(character, '\\' | '"' | '$') {
            value.push('\\');
        }
        value.push(character);
    }
    value.push('"');
    value
}

fn credential_block(id: Uuid, key: &str) -> String {
    let name = variable_name(id);
    format!(
        "\n{BEGIN}{id} {} ---\n{name}={}\n{END}{id} ---\n",
        checksum(&name, key),
        quote_key(key)
    )
}

impl EnvFile {
    fn load(paths: &CodexPaths) -> Result<Self, CoreError> {
        let path = env_path(paths);
        let original = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    return Err(CoreError::OperationFailed(
                        "newapi.errors.nativeEnvFileType".into(),
                    ));
                }
                if metadata.len() > MAX_ENV_BYTES {
                    return Err(CoreError::OperationFailed(
                        "newapi.errors.nativeEnvTooLarge".into(),
                    ));
                }
                Some(std::fs::read(&path).map_err(|_| {
                    CoreError::OperationFailed("newapi.errors.nativeEnvRead".into())
                })?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(CoreError::OperationFailed(
                    "newapi.errors.nativeEnvRead".into(),
                ))
            }
        };
        Self::parse(original)
    }

    fn parse(original: Option<Vec<u8>>) -> Result<Self, CoreError> {
        let text = String::from_utf8(original.clone().unwrap_or_default())
            .map_err(|_| CoreError::OperationFailed("newapi.errors.nativeEnvSyntax".into()))?;
        // This iterator parses only. It never installs variables into this
        // process, and parser errors (which can contain secret lines) are redacted.
        let variables: Vec<(String, String)> =
            dotenvy::from_read_iter(Cursor::new(text.as_bytes()))
                .collect::<Result<_, _>>()
                .map_err(|_| CoreError::OperationFailed("newapi.errors.nativeEnvSyntax".into()))?;
        let mut lines = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            lines.push((offset, line));
            offset += line.len();
        }
        let mut owned = Vec::new();
        let mut names = HashSet::new();
        let mut index = 0;
        while index < lines.len() {
            let (start, raw) = lines[index];
            let line = raw.trim_end_matches(['\r', '\n']);
            if let Some(marker) = line.strip_prefix(BEGIN) {
                let (id, expected_checksum) = marker
                    .strip_suffix(" ---")
                    .and_then(|value| value.split_once(' '))
                    .ok_or_else(credential_error)?;
                let parsed_id = Uuid::parse_str(id).map_err(|_| credential_error())?;
                if parsed_id.to_string() != id || index + 2 >= lines.len() {
                    return Err(credential_error());
                }
                let name = variable_name(parsed_id);
                if !names.insert(name.clone()) {
                    return Err(credential_error());
                }
                let matching: Vec<_> = variables.iter().filter(|(key, _)| key == &name).collect();
                if matching.len() != 1 {
                    return Err(credential_error());
                }
                let key = &matching[0].1;
                let assignment = lines[index + 1].1.trim_end_matches(['\r', '\n']);
                let closing = lines[index + 2].1.trim_end_matches(['\r', '\n']);
                if assignment != format!("{name}={}", quote_key(key))
                    || closing != format!("{END}{id} ---")
                    || expected_checksum != checksum(&name, key)
                    || !lines[index + 2].1.ends_with('\n')
                {
                    return Err(credential_error());
                }
                // Each generated record owns its leading separator, so removal
                // restores even a user's original file without a final newline.
                let separator = if text[..start].ends_with("\r\n") {
                    2
                } else if text[..start].ends_with('\n') {
                    1
                } else {
                    0
                };
                let range = start - separator..lines[index + 2].0 + lines[index + 2].1.len();
                if owned
                    .last()
                    .is_some_and(|entry: &OwnedCredential| entry.range.end > range.start)
                {
                    return Err(credential_error());
                }
                owned.push(OwnedCredential {
                    id: parsed_id,
                    name,
                    key: key.clone(),
                    range,
                });
                index += 3;
            } else {
                if line.contains("# --- AgentHarbor Codex native")
                    || line.contains("# --- End AgentHarbor Codex native")
                {
                    return Err(credential_error());
                }
                index += 1;
            }
        }
        if variables
            .iter()
            .any(|(name, _)| name.starts_with(ENV_PREFIX) && !names.contains(name))
        {
            return Err(credential_error());
        }
        Ok(Self {
            original,
            text,
            owned,
        })
    }

    fn write(&self, paths: &CodexPaths, updated: &str) -> Result<(), CoreError> {
        if updated.len() as u64 > MAX_ENV_BYTES {
            return Err(CoreError::OperationFailed(
                "newapi.errors.nativeEnvTooLarge".into(),
            ));
        }
        // Check again immediately before replacing; preserve concurrent edits
        // visible here. This is not an interprocess compare-and-swap guarantee.
        if Self::load(paths)?.original != self.original {
            return Err(CoreError::OperationFailed(
                "newapi.errors.nativeEnvChanged".into(),
            ));
        }
        if updated.as_bytes() == self.original.as_deref().unwrap_or_default() {
            return Ok(());
        }
        storage::write_atomic(&env_path(paths), updated.as_bytes())
            .map_err(|_| CoreError::OperationFailed("newapi.errors.nativeEnvWrite".into()))
    }
}

fn selected_provider(store: &Store) -> Result<&Provider, String> {
    let active = match store.active_provider_id.as_deref() {
        Some(id) => store.providers.iter().find(|provider| provider.id == id),
        None => store
            .providers
            .iter()
            .find(|provider| provider.has_route_credentials() && !provider.model.is_empty()),
    }
    .ok_or("newapi.errors.noActiveProvider")?;
    if active.selected_protocol() != ModelProtocol::Responses {
        return Err("newapi.errors.nativeResponsesRequired".into());
    }
    if !active.is_routable() {
        return Err("newapi.errors.noActiveProvider".into());
    }
    Ok(active)
}

fn endpoint(provider: &Provider) -> Result<String, String> {
    let mut url = newapi::provider_endpoint(&provider.base_url, "v1/responses")?;
    let root = url
        .path()
        .strip_suffix("/responses")
        .ok_or("newapi.errors.url")?
        .to_owned();
    url.set_path(&root);
    Ok(url.to_string())
}

fn api_key(provider: &Provider) -> Result<String, String> {
    newapi::api_key_header(&provider.api_key)?
        .to_str()
        .map(str::to_owned)
        .map_err(|_| "newapi.errors.token".into())
}

fn model_ids(provider: &Provider) -> Vec<String> {
    let mut ids: Vec<String> = provider
        .models
        .iter()
        .filter(|model| {
            model.included
                && provider.protocol_for_model(&model.id) == ModelProtocol::Responses
                && provider.supports_model(&model.id)
        })
        .map(|model| model.id.clone())
        .collect();
    if !provider.model.is_empty()
        && provider.protocol_for_model(&provider.model) == ModelProtocol::Responses
        && provider.supports_model(&provider.model)
        && !ids.contains(&provider.model)
    {
        ids.push(provider.model.clone());
    }
    ids
}

struct NativeConfigurationPlan {
    catalog: serde_json::Value,
    models: Vec<String>,
    compression: model_policy::SharedCompressionPlan,
    context: Option<u64>,
}

fn configuration_plan(
    store: &Store,
    provider: &Provider,
    models_dev: &models_dev::Catalog,
    official_models: &[serde_json::Value],
) -> NativeConfigurationPlan {
    let global_compression = store.global_compression();
    let mut seen = HashSet::new();
    let ids: Vec<_> = model_ids(provider)
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect();
    let resolved: Vec<_> = ids
        .iter()
        .map(|id| {
            let official = super::matched_official_model(official_models, &id)
                .filter(|_| super::official_endpoint_applies(provider));
            super::model_projection(provider, &id, official, models_dev, &global_compression)
        })
        .collect();
    let compression = model_policy::shared_plan(
        &ids.iter()
            .zip(&resolved)
            .map(|(id, (_, projection))| (id.as_str(), projection))
            .collect::<Vec<_>>(),
    );
    let context = provider.context_window.map(|context| {
        resolved
            .iter()
            .map(|(_, projection)| projection.context_window)
            .min()
            .map_or(context, |smallest| context.min(smallest))
    });
    let models: Vec<_> = ids
        .iter()
        .zip(resolved)
        .map(|(id, (metadata, projection))| {
            let alias = metadata.display_name.trim();
            super::relay_catalog_entry(
                id.clone(),
                super::menu_display_name(&provider.name, alias, &id),
                format!("{} → {id}", provider.name),
                projection.context_window,
                projection.output_limit,
                projection.reasoning,
                projection.default_reasoning,
            )
        })
        .collect();
    NativeConfigurationPlan {
        catalog: serde_json::json!({ "models": models }),
        models: ids,
        compression,
        context,
    }
}

fn load_configuration_plan(
    paths: &CodexPaths,
    store: &Store,
    provider: &Provider,
) -> NativeConfigurationPlan {
    let catalog = models_dev::load(&paths.app_data_dir);
    let official = model_catalog::official(&paths.codex_home, &paths.app_data_dir);
    configuration_plan(store, provider, &catalog, &official.models)
}

#[cfg(test)]
fn catalog(paths: &CodexPaths, store: &Store, provider: &Provider) -> serde_json::Value {
    load_configuration_plan(paths, store, provider).catalog
}

pub(super) fn subagent_catalog(paths: &CodexPaths, store: &Store) -> Result<serde_json::Value, String> {
    Ok(load_configuration_plan(paths, store, selected_provider(store)?).catalog)
}

/// Effective values the native config applies globally for this provider.
/// An explicit context override is capped by the smallest selected model. The compaction
/// threshold is the conservative minimum across this provider's selected
/// models so switching models inside one shared config never overstates a
/// smaller model's budget. Invalid or unknown models block a shared override.
pub(super) fn native_limits(
    paths: &CodexPaths,
    store: &Store,
    provider: &Provider,
) -> (Option<u64>, Option<u64>) {
    let plan = load_configuration_plan(paths, store, provider);
    (plan.context, plan.compression.threshold)
}

pub(crate) struct NativeCompressionState {
    pub provider_id: String,
    pub models: Vec<String>,
    pub context: Option<u64>,
    pub threshold: Option<u64>,
    pub applied_threshold: Option<u64>,
    pub limiting_models: Vec<String>,
    pub applied: Option<bool>,
    pub shared_plan: model_policy::SharedCompressionPlan,
    pub config_active: bool,
}

pub(super) fn compression_state(
    paths: &CodexPaths,
    store: &Store,
    catalog: &models_dev::Catalog,
    official_models: &[serde_json::Value],
) -> Option<NativeCompressionState> {
    let provider = selected_provider(store).ok()?;
    let plan = configuration_plan(store, provider, catalog, official_models);
    let _guard = config_snapshot::guard().ok()?;
    let applied = intact_unlocked(paths, store, &plan)
        .ok()
        .and_then(|intact| {
            (plan.compression.state != "clientDefault").then_some(
                intact && !matches!(plan.compression.state.as_str(), "invalid" | "unknown"),
            )
        });
    let document = config(paths).ok();
    let config_active = document.as_ref().is_some_and(|document| {
        let owns_scope = document.get("model_provider").and_then(Item::as_str)
            == Some(PROVIDER_KEY)
            && document
                .get("model_providers")
                .and_then(|entries| entries.get(PROVIDER_KEY))
                .and_then(|entry| entry.get("base_url"))
                .and_then(Item::as_str)
                == endpoint(provider).ok().as_deref();
        let credential = document
            .get("model_providers")
            .and_then(|entries| entries.get(PROVIDER_KEY))
            .and_then(|entry| entry.get("env_key"))
            .and_then(Item::as_str)
            .and_then(|name| {
                EnvFile::load(paths)
                    .ok()
                    .and_then(|file| file.owned.into_iter().find(|entry| entry.name == name))
            });
        owns_scope
            && credential.is_some_and(|credential| {
                api_key(provider).ok().as_deref() == Some(credential.key.as_str())
                    && document
                        .get("model_catalog_json")
                        .and_then(Item::as_str)
                        .is_some_and(|_| installed_catalog_path(paths, document).is_some())
            })
    });
    let applied_threshold = config_active
        .then_some(document)
        .flatten()
        .and_then(|document| {
            document
                .get("model_auto_compact_token_limit")
                .and_then(Item::as_integer)
                .and_then(|value| u64::try_from(value).ok())
        });
    Some(NativeCompressionState {
        provider_id: provider.id.clone(),
        models: plan.models,
        context: plan.context,
        threshold: plan.compression.threshold,
        limiting_models: plan.compression.limiting_models.clone(),
        applied_threshold,
        applied,
        shared_plan: plan.compression,
        config_active,
    })
}

fn config(paths: &CodexPaths) -> Result<DocumentMut, CoreError> {
    CodexConfigRepository::new(paths)
        .read_text()?
        .parse()
        .map_err(|_| CoreError::InvalidData("Invalid Codex config TOML".into()))
}

fn provider_credential_variables(entry: &Item) -> impl Iterator<Item = &str> {
    entry
        .get("env_key")
        .and_then(Item::as_str)
        .into_iter()
        .chain(
            entry
                .get("env_http_headers")
                .and_then(Item::as_table_like)
                .into_iter()
                .flat_map(|headers| headers.iter())
                .filter_map(|(_, value)| value.as_str()),
        )
}

fn referenced_variables(document: &DocumentMut) -> HashSet<String> {
    document
        .get("model_providers")
        .and_then(Item::as_table_like)
        .into_iter()
        .flat_map(|providers| providers.iter())
        .flat_map(|(_, provider)| provider_credential_variables(provider))
        .map(str::to_owned)
        .collect()
}

fn cleanup_unlocked(paths: &CodexPaths, only: Option<Uuid>) -> Result<(), CoreError> {
    let document = config(paths)?;
    let referenced = referenced_variables(&document);
    let foreign_references: HashSet<_> = document
        .get("model_providers")
        .and_then(Item::as_table_like)
        .into_iter()
        .flat_map(|table| table.iter())
        .filter(|(key, _)| *key != PROVIDER_KEY)
        .flat_map(|(_, entry)| provider_credential_variables(entry))
        .collect();
    let active_catalog = document
        .get("model_catalog_json")
        .and_then(Item::as_str)
        .map(PathBuf::from);
    let canonical_active_catalog = active_catalog
        .as_ref()
        .and_then(|path| std::fs::canonicalize(path).ok());
    let file = EnvFile::load(paths)?;
    let removing: Vec<_> = file
        .owned
        .iter()
        .filter(|entry| only.map_or(true, |id| entry.id == id) && !referenced.contains(&entry.name))
        .collect();
    let mut updated = file.text.clone();
    for entry in removing.iter().rev() {
        updated.replace_range(entry.range.clone(), "");
    }
    // Remove secrets first. Catalogs contain no secrets and can be rediscovered
    // by their reserved canonical UUID names if their deletion needs a retry.
    if !removing.is_empty() {
        file.write(paths, &updated)?;
    }
    let entries = match std::fs::read_dir(&paths.app_data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("codex-native-"))
            .and_then(|name| name.strip_suffix(".json"))
            .and_then(|name| Uuid::parse_str(name).ok())
        else {
            continue;
        };
        if path != catalog_path(paths, id)
            || only.is_some_and(|only| only != id)
            || foreign_references.contains(variable_name(id).as_str())
        {
            continue;
        }
        // Config paths may use equivalent separators or case on Windows.
        // Protect the current file by its path identity, not its TOML spelling.
        if active_catalog.as_ref() == Some(&path)
            || canonical_active_catalog
                .as_ref()
                .is_some_and(|active| std::fs::canonicalize(&path).ok().as_ref() == Some(active))
        {
            continue;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CoreError::OperationFailed(STORAGE_ERROR.into())),
        }
    }
    Ok(())
}

pub(super) fn cleanup_credentials(paths: &CodexPaths) -> Result<(), String> {
    let _guard = config_snapshot::guard().map_err(config_update_error)?;
    cleanup_unlocked(paths, None).map_err(config_update_error)
}

pub(super) fn intact(paths: &CodexPaths, store: &Store) -> Result<bool, String> {
    let _guard = config_snapshot::guard().map_err(config_update_error)?;
    let plan = load_configuration_plan(paths, store, selected_provider(store)?);
    intact_unlocked(paths, store, &plan)
}

fn supplemental_headers(
    document: &DocumentMut,
    expected_base: &str,
) -> Result<Vec<(&'static str, Item)>, CoreError> {
    if document.get("model_provider").and_then(Item::as_str) != Some(PROVIDER_KEY) {
        return Ok(Vec::new());
    }
    let existing = document
        .get("model_providers")
        .and_then(|p| p.get(PROVIDER_KEY));
    if ["experimental_bearer_token", "auth", "aws"]
        .iter()
        .any(|key| existing.and_then(|entry| entry.get(key)).is_some())
    {
        return Err(CoreError::OperationFailed(
            "newapi.errors.nativeAuthOverrides".into(),
        ));
    }
    let mut headers = Vec::new();
    for key in ["http_headers", "env_http_headers"] {
        if let Some(value) = existing.and_then(|entry| entry.get(key)) {
            if !value
                .as_table_like()
                .is_some_and(|table| table.iter().all(|(_, value)| value.as_str().is_some()))
            {
                return Err(CoreError::OperationFailed(
                    "newapi.errors.nativeHeaders".into(),
                ));
            }
            if existing
                .and_then(|entry| entry.get("base_url"))
                .and_then(Item::as_str)
                != Some(expected_base)
            {
                return Err(CoreError::OperationFailed(
                    "newapi.errors.nativeHeaderScope".into(),
                ));
            }
            headers.push((key, value.clone()));
        }
    }
    Ok(headers)
}

fn intact_unlocked(
    paths: &CodexPaths,
    store: &Store,
    plan: &NativeConfigurationPlan,
) -> Result<bool, String> {
    let provider = selected_provider(store)?;
    let expected_base = endpoint(provider)?;
    let expected_key = api_key(provider)?;
    let original = CodexConfigRepository::new(paths)
        .read_text()
        .map_err(config_update_error)?;
    let document = original
        .parse::<DocumentMut>()
        .map_err(|_| "newapi.errors.configParse".to_owned())?;
    let Some(snapshot) = config_snapshot::load(paths, &original)
        .map_err(config_update_error)?
    else {
        return Ok(false);
    };
    let file = EnvFile::load(paths).map_err(config_update_error)?;
    let Some(entry) = document
        .get("model_providers")
        .and_then(|providers| providers.get(PROVIDER_KEY))
    else {
        return Ok(false);
    };
    let Some(credential) = entry
        .get("env_key")
        .and_then(Item::as_str)
        .and_then(|name| file.owned.iter().find(|item| item.name == name))
    else {
        return Ok(false);
    };
    if std::env::var_os(&credential.name)
        .is_some_and(|value| value.to_str() != Some(credential.key.as_str()))
    {
        return Err(CREDENTIAL_ERROR.into());
    }
    let Some(expected_catalog) = installed_catalog_path(paths, &document) else {
        return Ok(false);
    };
    let (expected_context, expected_threshold) = (plan.context, plan.compression.threshold);
    let catalog_matches = std::fs::read(&expected_catalog)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value == plan.catalog);
    Ok(credential.key == expected_key
        && document.get("model_provider").and_then(Item::as_str) == Some(PROVIDER_KEY)
        && document.get("model").and_then(Item::as_str) == Some(provider.model.as_str())
        && document.get("model_catalog_json").and_then(Item::as_str)
            == Some(expected_catalog.to_string_lossy().as_ref())
        && [
            ("model_context_window", expected_context),
            ("model_auto_compact_token_limit", expected_threshold),
        ]
        .iter()
        .all(|(key, expected)| match (document.get(key), *expected) {
            (None, None) => true,
            (Some(item), Some(expected)) => item.as_integer() == Some(expected as i64),
            _ => false,
        })
        && document.get("preferred_auth_method").and_then(Item::as_str) == Some("apikey")
        && entry.get("base_url").and_then(Item::as_str) == Some(expected_base.as_str())
        && entry.get("wire_api").and_then(Item::as_str) == Some("responses")
        && entry.get("requires_openai_auth").and_then(Item::as_bool) == Some(false)
        && ["experimental_bearer_token", "auth", "aws"]
            .iter()
            .all(|key| entry.get(key).is_none())
        && supplemental_headers(&document, &expected_base).is_ok()
        && catalog_matches
        && snapshot.subagents_match(store.codex_subagents.as_ref())
        && super::subagents::matches(&document, store))
}

pub(super) fn apply(paths: &CodexPaths, store: &Store) -> Result<newapi::AppliedConfig, String> {
    super::subagents::validate(paths, store)?;
    let _guard = config_snapshot::guard().map_err(config_update_error)?;
    let provider = selected_provider(store)?;
    let expected_base = endpoint(provider)?;
    let key = api_key(provider)?;
    let plan = load_configuration_plan(paths, store, provider);
    let (limit_context, limit_threshold) = (plan.context, plan.compression.threshold);
    newapi::validate_limits(&newapi::Connection {
        model: provider.model.clone(),
        context_window: limit_context,
        auto_compact_limit: limit_threshold,
        ..Default::default()
    })?;
    if intact_unlocked(paths, store, &plan)? {
        if cleanup_unlocked(paths, None).is_err() {
            log::warn!("Native credential cleanup will retry on next use");
        }
        return Ok(newapi::AppliedConfig {
            path: paths.config_path.display().to_string(),
            backup_path: None,
        });
    }
    // Preserve supplemental headers only within their original endpoint scope.
    // Never silently drop them, or copy header credentials to another host.
    supplemental_headers(&config(paths).map_err(config_update_error)?, &expected_base)
        .map_err(config_update_error)?;
    let file = EnvFile::load(paths).map_err(config_update_error)?;
    let id = Uuid::new_v4();
    let document = config(paths).map_err(config_update_error)?;
    let reusable = (document.get("model_provider").and_then(Item::as_str) == Some(PROVIDER_KEY))
        .then(|| {
            document
                .get("model_providers")
                .and_then(|entries| entries.get(PROVIDER_KEY))
        })
        .flatten()
        .filter(|entry| {
            entry.get("base_url").and_then(Item::as_str) == Some(expected_base.as_str())
        })
        .and_then(|entry| entry.get("env_key").and_then(Item::as_str))
        .and_then(|name| file.owned.iter().find(|credential| credential.name == name))
        .filter(|credential| {
            credential.key == key
                && std::env::var_os(&credential.name)
                    .is_none_or(|value| value.to_str() == Some(key.as_str()))
        });
    let name = reusable
        .map(|credential| credential.name.clone())
        .unwrap_or_else(|| variable_name(id));
    let catalog_path = catalog_path(paths, id);
    if (reusable.is_none() && std::env::var_os(&name).is_some()) || catalog_path.exists() {
        return Err(CREDENTIAL_ERROR.into());
    }
    if reusable.is_none() {
        let updated = format!("{}{}", file.text, credential_block(id, &key));
        EnvFile::parse(Some(updated.as_bytes().to_vec())).map_err(config_update_error)?;
        file.write(paths, &updated).map_err(config_update_error)?;
    }
    let mut pending = None;
    let result = (|| -> Result<_, CoreError> {
        storage::write_atomic(&catalog_path, &serde_json::to_vec_pretty(&plan.catalog)?)?;
        CodexConfigRepository::new(paths).update(|original| {
            let mut document = original
                .parse::<DocumentMut>()
                .map_err(|_| CoreError::InvalidData("Invalid Codex config TOML".into()))?;
            if document
                .get("model_providers")
                .is_some_and(|item| !item.is_table_like())
            {
                return Err(CoreError::InvalidData(
                    "model_providers must be a TOML table".into(),
                ));
            }
            // Read again inside the repository's guarded update so a user edit
            // between preflight and commit is never replaced by stale headers.
            let headers = supplemental_headers(&document, &expected_base)?;
            let mut snapshot = match config_snapshot::load(paths, original)? {
                Some(snapshot) if snapshot.selected_provider_is_ours(&document) => snapshot,
                Some(snapshot) => {
                    snapshot.restore_subagents(&mut document)?;
                    config_snapshot::Snapshot::capture(paths, &document)?
                }
                None if model_provider_is_ours(&document) => {
                    config_snapshot::recover_legacy(paths, &document, store)?
                }
                None => {
                    reject_foreign_managed_provider_entries(&document)?;
                    config_snapshot::Snapshot::capture(paths, &document)?
                }
            };
            remove_managed_provider_entries(&mut document);
            for (field, value) in [
                ("model", provider.model.clone()),
                ("model_provider", PROVIDER_KEY.into()),
                (
                    "model_catalog_json",
                    catalog_path.to_string_lossy().into_owned(),
                ),
            ] {
                config_snapshot::set_value(&mut document, field, Some(value.into()));
            }
            config_snapshot::set_value(
                &mut document,
                "model_context_window",
                limit_context.map(|value| (value as i64).into()),
            );
            config_snapshot::set_value(
                &mut document,
                "model_auto_compact_token_limit",
                limit_threshold.map(|value| (value as i64).into()),
            );
            if document.get("model_providers").is_none() {
                let mut table = toml_edit::Table::new();
                table.set_implicit(true);
                document["model_providers"] = Item::Table(table);
            }
            let inline_providers = document["model_providers"]
                .as_value()
                .is_some_and(toml_edit::Value::is_inline_table);
            let entry = &mut document["model_providers"][PROVIDER_KEY];
            *entry = if inline_providers {
                Item::Value(toml_edit::Value::InlineTable(toml_edit::InlineTable::new()))
            } else {
                Item::Table(toml_edit::Table::new())
            };
            entry["name"] = toml_edit::value(&provider.name);
            entry["base_url"] = toml_edit::value(&expected_base);
            entry["wire_api"] = toml_edit::value("responses");
            entry["env_key"] = toml_edit::value(&name);
            // Never permit a third-party endpoint to select official tokens.
            entry["requires_openai_auth"] = toml_edit::value(false);
            for (key, value) in &headers {
                entry[key] = value.clone();
            }
            snapshot.apply_auth(&mut document, true)?;
            snapshot.apply_subagents(&mut document, store.codex_subagents.as_ref())?;
            snapshot.prepare(paths, original, &document, config_snapshot::Action::Apply)?;
            pending = Some(snapshot);
            Ok(document.to_string())
        })
    })();
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if cleanup_unlocked(paths, Some(id)).is_err() {
                log::warn!("An uncommitted native credential will be cleaned up on next use");
            }
            return Err(config_update_error(error));
        }
    };
    if let Some(snapshot) = pending {
        snapshot.finish(paths);
    }
    // Configuration has committed. Cleanup cannot turn success into an error
    // and leave the persisted access mode disagreeing with the live config.
    if cleanup_unlocked(paths, None).is_err() {
        log::warn!("Native credential cleanup will retry on next use");
    }
    Ok(applied_config(result))
}

/// The reserved env variable, checksummed owned block and catalog path together
/// prove a native installation when its nonsecret journal was lost.
pub(super) fn proven_installation(paths: &CodexPaths, document: &DocumentMut) -> bool {
    if document.get("model_provider").and_then(Item::as_str) != Some(PROVIDER_KEY) {
        return false;
    }
    let Some(entry) = document
        .get("model_providers")
        .and_then(|p| p.get(PROVIDER_KEY))
    else {
        return false;
    };
    let Ok(file) = EnvFile::load(paths) else {
        return false;
    };
    let Some(_credential) = entry
        .get("env_key")
        .and_then(Item::as_str)
        .and_then(|name| file.owned.iter().find(|entry| entry.name == name))
    else {
        return false;
    };
    entry.get("wire_api").and_then(Item::as_str) == Some("responses")
        && entry.get("requires_openai_auth").and_then(Item::as_bool) == Some(false)
        && entry
            .get("base_url")
            .and_then(Item::as_str)
            .is_some_and(|url| newapi::base_url(url).is_ok())
        && installed_catalog_path(paths, document).is_some_and(|path| path.is_file())
}

pub(super) fn matches_installation(
    paths: &CodexPaths,
    document: &DocumentMut,
    provider: &Provider,
) -> bool {
    let Some(entry) = document
        .get("model_providers")
        .and_then(|p| p.get(PROVIDER_KEY))
    else {
        return false;
    };
    let Ok(file) = EnvFile::load(paths) else {
        return false;
    };
    entry.get("base_url").and_then(Item::as_str) == endpoint(provider).ok().as_deref()
        && entry
            .get("env_key")
            .and_then(Item::as_str)
            .and_then(|name| file.owned.iter().find(|entry| entry.name == name))
            .is_some_and(|credential| {
                api_key(provider).ok().as_deref() == Some(credential.key.as_str())
            })
}

pub(super) fn installed_catalog_model(paths: &CodexPaths, document: &DocumentMut) -> bool {
    if document.get("model_provider").and_then(Item::as_str) != Some(PROVIDER_KEY) {
        return false;
    }
    let Some(path) = installed_catalog_path(paths, document) else {
        return false;
    };
    let Some(model) = document.get("model").and_then(Item::as_str) else {
        return false;
    };
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|catalog| {
            catalog
                .get("models")
                .and_then(serde_json::Value::as_array)
                .map(|models| {
                    models.iter().any(|entry| {
                        entry.get("slug").and_then(serde_json::Value::as_str) == Some(model)
                    })
                })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::router::{remove_routing_config, CodexAccessMode, RouterSettings};

    const ORIGINAL: &str = "# user config\nmodel = 'user-model' # chosen model\nmodel_provider = 'custom'\nmodel_catalog_json = 'user-catalog.json'\nmodel_context_window = 90000\nmodel_auto_compact_token_limit = 80000\npreferred_auth_method = 'chatgpt' # login preference\ntheme = 'dark'\n[model_providers.custom]\nname = 'User provider'\nbase_url = 'https://user.example.invalid'\n[mcp_servers.tools]\ncommand = 'node' # keep this comment\n";
    const USER_ENV: &str = "# user-owned credentials\r\nUSER_KEY='untouched'\r\nMULTILINE=\"line one\nline two\"\nTAIL=value";
    const AUTH: &[u8] = b"{\"fixture\":\"synthetic-official-login-remains-byte-for-byte\"}\n";

    fn model(id: &str, protocol: ModelProtocol, context: u64) -> newapi::Model {
        newapi::Model {
            id: id.into(),
            protocol: Some(protocol),
            // A collected upstream declaration, which outranks the catalog.
            context_window: Some(context),
            metadata_source: "upstream".into(),
            ..Default::default()
        }
    }

    fn fixture() -> (tempfile::TempDir, CodexPaths, Store) {
        let directory = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(directory.path().join("codex"));
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(&paths.config_path, ORIGINAL).unwrap();
        std::fs::write(&paths.auth_path, AUTH).unwrap();
        let mut excluded = model("excluded-model", ModelProtocol::Responses, 80000);
        excluded.included = false;
        let provider = Provider {
            id: "fixture-native".into(),
            name: "Fixture native".into(),
            base_url: "https://native.example.invalid/api/v1/".into(),
            api_key: r#"synthetic-$HOME-"quote'-\key"#.into(),
            model: "response-model".into(),
            enabled: true,
            models: vec![
                model("response-model", ModelProtocol::Responses, 240000),
                model("second-response", ModelProtocol::Responses, 160000),
                model("chat-model", ModelProtocol::ChatCompletions, 80000),
                model("message-model", ModelProtocol::AnthropicMessages, 80000),
                excluded,
            ],
            ..Default::default()
        };
        let store = Store {
            providers: vec![provider],
            active_provider_id: Some("fixture-native".into()),
            codex_access_mode: CodexAccessMode::Native,
            router: RouterSettings {
                enabled: true,
                bearer_token: "synthetic-local-token".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        (directory, paths, store)
    }

    fn text(paths: &CodexPaths) -> String {
        std::fs::read_to_string(&paths.config_path).unwrap()
    }

    fn document(paths: &CodexPaths) -> DocumentMut {
        text(paths).parse().unwrap()
    }

    fn installed_id(paths: &CodexPaths) -> Uuid {
        let document = document(paths);
        let name = document["model_providers"][PROVIDER_KEY]["env_key"]
            .as_str()
            .unwrap();
        EnvFile::load(paths)
            .unwrap()
            .owned
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .id
    }

    fn generated_catalogs(paths: &CodexPaths) -> Vec<PathBuf> {
        let mut paths: Vec<_> = std::fs::read_dir(&paths.app_data_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("codex-native-") && name.ends_with(".json")
                    })
            })
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn native_uses_real_endpoint_model_and_only_selected_provider_responses_catalog() {
        let (_directory, paths, mut store) = fixture();
        let mut other = store.providers[0].clone();
        other.id = "another-provider".into();
        other.model = "foreign-response".into();
        other.models = vec![model("foreign-response", ModelProtocol::Responses, 320000)];
        store.providers.push(other);
        apply(&paths, &store).unwrap();
        let document = document(&paths);
        let provider = &document["model_providers"][PROVIDER_KEY];
        assert_eq!(
            provider["base_url"].as_str(),
            Some("https://native.example.invalid/api/v1")
        );
        assert_eq!(provider["wire_api"].as_str(), Some("responses"));
        assert_eq!(provider["requires_openai_auth"].as_bool(), Some(false));
        assert_eq!(document["model"].as_str(), Some("response-model"));
        assert_eq!(document["model_provider"].as_str(), Some(PROVIDER_KEY));
        assert_eq!(document["preferred_auth_method"].as_str(), Some("apikey"));
        assert!(document.get("model_context_window").is_none());
        // The 90% default applies per model; the shared config keeps the
        // conservative minimum across this provider's models (160000 * 90%).
        assert_eq!(
            document["model_auto_compact_token_limit"].as_integer(),
            Some(144_000)
        );
        for key in [
            "experimental_bearer_token",
            "http_headers",
            "env_http_headers",
            "auth",
        ] {
            assert!(provider.get(key).is_none());
        }
        let catalog: serde_json::Value = serde_json::from_slice(
            &std::fs::read(catalog_path(&paths, installed_id(&paths))).unwrap(),
        )
        .unwrap();
        let models = catalog["models"].as_array().unwrap();
        let ids: Vec<_> = models
            .iter()
            .map(|model| model["slug"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["response-model", "second-response"]);
        assert_eq!(models[0]["context_window"], 240000);
        assert_eq!(models[1]["context_window"], 160000);
        assert!(intact(&paths, &store).unwrap());

        // Only an explicit provider-wide override belongs in root config.
        store.providers[0].context_window = Some(200000);
        store.providers[0].auto_compact_limit = Some(150000);
        apply(&paths, &store).unwrap();
        let document = self::document(&paths);
        assert_eq!(document["model_context_window"].as_integer(), Some(200000));
        assert_eq!(
            document["model_auto_compact_token_limit"].as_integer(),
            Some(150000)
        );
        assert!(intact(&paths, &store).unwrap());
    }

    #[test]
    fn kiro_capability_matching_preserves_routing_id_and_menu_alias() {
        let (_directory, paths, mut store) = fixture();
        let provider = &mut store.providers[0];
        provider.model = "claude-opus-5-5-kiro".into();
        provider.models = vec![newapi::Model {
            id: provider.model.clone(),
            display_name: "Opus 自定义别名".into(),
            protocol: Some(ModelProtocol::Responses),
            context_window: Some(32000),
            max_output_tokens: Some(1024),
            metadata_source: "upstream".into(),
            supported_reasoning_levels: Some(vec![]),
            ..Default::default()
        }];
        // The account snapshot exists, but this relay does not target the
        // official endpoint, so its values must not apply.
        let official = serde_json::json!({"models":[{
            "slug":"claude-opus-5-5", "context_window":1000000, "max_output_tokens":128000,
            "supported_reasoning_levels":[{"effort":"high","description":"Model high"},{"effort":"max","description":"Model max"}],
            "default_reasoning_level":"high"
        }]});
        super::super::super::model_catalog::seed_test_account_catalog(&paths.codex_home, &paths.app_data_dir, official.to_string().as_bytes());
        std::fs::write(
            paths.codex_home.join("models_cache.json"),
            official.to_string(),
        )
        .unwrap();
        let native = catalog(&paths, &store, &store.providers[0]);
        let router = super::super::catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        assert_eq!(native["models"][0]["slug"], "claude-opus-5-5-kiro");
        assert_eq!(
            router["models"][1]["slug"],
            "fixture-native::claude-opus-5-5-kiro"
        );
        for entry in [&native["models"][0], &router["models"][1]] {
            assert_eq!(entry["display_name"], "Fixture native/Opus 自定义别名");
            // The relay's own declaration stays authoritative.
            assert_eq!(entry["context_window"], 32_000);
            assert_eq!(entry["max_output_tokens"], 1_024);
            assert_eq!(entry["supported_reasoning_levels"], serde_json::json!([]));
            assert!(entry["default_reasoning_level"].is_null());
        }

        // The same model on the official endpoint keeps the account values.
        store.providers[0].base_url = "https://chatgpt.com/backend-api/codex".into();
        let native = catalog(&paths, &store, &store.providers[0]);
        // The upstream declaration still outranks the account snapshot; only
        // undeclared fields come from it.
        assert_eq!(native["models"][0]["context_window"], 32_000);
        assert_eq!(store.providers[0].models[0].id, "claude-opus-5-5-kiro");
    }

    #[test]
    fn model_identity_unifies_context_output_and_reasoning_across_channels() {
        let (_directory, paths, mut store) = fixture();
        let template = store.providers[0].clone();
        // No channel records a declaration: identity matching supplies the
        // shared value instead of each channel inventing its own.
        let ids = [
            "unify-model-x",
            "OpenAI/UNIFY-MODEL-X@fast",
            "channel__unify-model-x:free",
        ];
        store.providers = ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                let mut provider = template.clone();
                provider.id = format!("channel-{index}");
                provider.model = (*id).into();
                provider.models = vec![newapi::Model {
                    id: (*id).into(),
                    metadata_source: "legacy".into(),
                    ..Default::default()
                }];
                provider
            })
            .collect();
        let router = super::super::catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        for (index, provider) in store.providers.iter().enumerate() {
            let native = catalog(&paths, &store, provider);
            assert_eq!(
                native["models"][0]["slug"], provider.model,
                "Routing ID is preserved"
            );
            for entry in [&native["models"][0], &router["models"][index]] {
                for field in [
                    "context_window",
                    "max_output_tokens",
                    "supported_reasoning_levels",
                    "default_reasoning_level",
                ] {
                    assert_eq!(
                        entry[field], native["models"][0][field],
                        "channel {index}: {field}"
                    );
                }
                assert_eq!(entry["context_window"], 128_000);
            }
        }
        // A user override applies to one model only.
        let metadata = &mut store.providers[0].models[0];
        metadata.overrides.context_window = Some(200_000);
        metadata.overrides.max_output_tokens = Some(8_192);
        let native = catalog(&paths, &store, &store.providers[0]);
        assert_eq!(native["models"][0]["context_window"], 200_000);
        assert_eq!(native["models"][0]["max_output_tokens"], 8_192);
        for provider in &store.providers[1..] {
            let other = catalog(&paths, &store, provider);
            assert_eq!(other["models"][0]["context_window"], 128_000);
        }
    }

    #[test]
    fn upstream_declarations_win_over_the_account_snapshot_for_relays() {
        let (_directory, paths, mut store) = fixture();
        store.providers[0].model = "gpt-6-astra".into();
        store.providers[0].models = vec![model("gpt-6-astra", ModelProtocol::Responses, 240_000)];
        let official = serde_json::json!({"models":[{
            "slug":"gpt-6-astra", "supported_reasoning_levels":[{"effort":"high","description":"Official high"}],
            "default_reasoning_level":"high"
        }]});
        super::super::super::model_catalog::seed_test_account_catalog(&paths.codex_home, &paths.app_data_dir, official.to_string().as_bytes());
        std::fs::write(
            paths.codex_home.join("models_cache.json"),
            official.to_string(),
        )
        .unwrap();
        for (levels, default, expected_levels, expected_default) in [
            (
                Some(vec![newapi::ReasoningLevel {
                    effort: "low".into(),
                    description: "Provider low".into(),
                }]),
                Some("high"),
                vec!["low"],
                Some("low"),
            ),
            (Some(vec![]), Some("high"), vec![], None),
            // Without an upstream declaration, publisher catalog levels apply.
            // The account snapshot's single `high` level still cannot leak in.
            (
                None,
                Some("low"),
                vec!["low", "medium", "high", "xhigh", "max"],
                Some("medium"),
            ),
        ] {
            store.providers[0].models[0].supported_reasoning_levels = levels;
            store.providers[0].models[0].default_reasoning_level = default.map(str::to_owned);
            let native = catalog(&paths, &store, &store.providers[0]);
            let router =
                super::super::catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
            for entry in [&native["models"][0], &router["models"][1]] {
                let efforts: Vec<&str> = entry["supported_reasoning_levels"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|level| level["effort"].as_str().unwrap())
                    .collect();
                assert_eq!(efforts, expected_levels);
                assert_eq!(
                    entry["default_reasoning_level"],
                    serde_json::json!(expected_default)
                );
            }
            assert_eq!(
                router["models"][0], official["models"][0],
                "Official model stays untouched"
            );
        }
    }

    #[test]
    fn native_catalog_uses_collected_declarations_and_never_leaks_official_ids_or_flags() {
        let (_directory, paths, mut store) = fixture();
        store.providers[0].model = "gpt-6-astra".into();
        store.providers[0].models = vec![
            model("gpt-6-astra", ModelProtocol::Responses, 240_000),
            model("gpt-6-astra-preview", ModelProtocol::Responses, 160_000),
        ];
        let levels = serde_json::json!([
            {"effort": "low", "description": "Cached low"},
            {"effort": "ultra", "description": "Cached ultra"}
        ]);
        let cache = serde_json::to_vec(&serde_json::json!({"models": [
            {
                "slug": "gpt-6-astra",
                "default_reasoning_level": "low",
                "supported_reasoning_levels": levels,
                "comp_hash": "official-only-id",
                "use_responses_lite": true
            },
            {"slug": "official-only-model"}
        ]}))
        .unwrap();
        std::fs::write(paths.codex_home.join("models_cache.json"), &cache).unwrap();
        let value = catalog(&paths, &store, &store.providers[0]);
        let models = value["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "gpt-6-astra");
        // The catalog supplies the human name; identities and limits still come
        // from the relay's own collected declaration.
        assert!(models[0]["display_name"]
            .as_str()
            .unwrap()
            .starts_with("Fixture native/"));
        // Limits still follow the upstream declaration. Undeclared reasoning
        // levels use OpenAI's model catalog, never the account cache's `ultra`.
        assert_eq!(models[0]["context_window"], 240_000);
        assert_eq!(
            models[0]["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["effort"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(models[0]["default_reasoning_level"], "medium");
        assert!(models[0].get("comp_hash").is_none());
        assert!(models[0].get("use_responses_lite").is_none());
        assert_eq!(models[1]["slug"], "gpt-6-astra-preview");
        assert_eq!(models[1]["context_window"], 160_000);
        assert_eq!(
            models[1]["supported_reasoning_levels"],
            serde_json::json!([])
        );
        assert!(models[1]["default_reasoning_level"].is_null());
        assert_eq!(
            std::fs::read(paths.codex_home.join("models_cache.json")).unwrap(),
            cache
        );
        assert_eq!(text(&paths), ORIGINAL);
        assert!(!env_path(&paths).exists());
        assert!(!config_snapshot::path(&paths).exists());
    }

    #[test]
    fn native_dotenv_roundtrips_special_keys_without_touching_auth_or_user_content() {
        let (_directory, paths, store) = fixture();
        std::fs::write(env_path(&paths), USER_ENV).unwrap();
        apply(&paths, &store).unwrap();
        let installed = EnvFile::load(&paths).unwrap();
        assert!(installed.text.starts_with(USER_ENV));
        assert_eq!(installed.owned.len(), 1);
        assert_eq!(installed.owned[0].key, store.providers[0].api_key);
        let variables: Vec<_> = dotenvy::from_read_iter(Cursor::new(installed.text.as_bytes()))
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(variables.contains(&("USER_KEY".into(), "untouched".into())));
        assert!(variables.contains(&("MULTILINE".into(), "line one\nline two".into())));
        assert!(!text(&paths).contains(&store.providers[0].api_key));
        assert!(!std::fs::read_to_string(config_snapshot::path(&paths))
            .unwrap()
            .contains(&store.providers[0].api_key));
        assert_eq!(std::fs::read(&paths.auth_path).unwrap(), AUTH);
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(std::fs::read_to_string(env_path(&paths)).unwrap(), USER_ENV);
        assert_eq!(std::fs::read(&paths.auth_path).unwrap(), AUTH);
        assert!(generated_catalogs(&paths).is_empty());
        assert_eq!(
            text(&paths).parse::<toml::Value>().unwrap(),
            ORIGINAL.parse::<toml::Value>().unwrap()
        );
    }

    #[test]
    fn native_rejects_non_responses_or_invalid_active_selection_without_writes() {
        for selected in ["chat-model", "message-model"] {
            let (_directory, paths, mut store) = fixture();
            store.providers[0].model = selected.into();
            assert_eq!(
                apply(&paths, &store).err().unwrap(),
                "newapi.errors.nativeResponsesRequired"
            );
            assert_eq!(text(&paths), ORIGINAL);
            assert!(!env_path(&paths).exists());
            assert!(!config_snapshot::path(&paths).exists());
        }
        for active in [
            Some("missing-provider".to_owned()),
            Some("fixture-native".to_owned()),
        ] {
            let (_directory, paths, mut store) = fixture();
            let mut fallback = store.providers[0].clone();
            fallback.id = "valid-fallback".into();
            store.providers.push(fallback);
            store.providers[0].enabled = false;
            store.active_provider_id = active;
            assert_eq!(
                apply(&paths, &store).err().unwrap(),
                "newapi.errors.noActiveProvider"
            );
            assert_eq!(text(&paths), ORIGINAL);
            assert!(!env_path(&paths).exists());
        }
    }

    #[test]
    fn native_legacy_missing_active_id_uses_first_eligible_model_without_protocol_fallback() {
        let (_directory, paths, mut store) = fixture();
        store.active_provider_id = None;
        apply(&paths, &store).unwrap();
        assert!(intact(&paths, &store).unwrap());
        let (_directory, paths, mut store) = fixture();
        store.active_provider_id = None;
        let mut fallback = store.providers[0].clone();
        fallback.id = "responses-fallback".into();
        store.providers.push(fallback);
        store.providers[0].model = "chat-model".into();
        assert_eq!(
            apply(&paths, &store).err().unwrap(),
            "newapi.errors.nativeResponsesRequired"
        );
        assert!(!env_path(&paths).exists());
    }

    #[test]
    fn native_reapply_is_idempotent_and_does_not_rotate_credentials_or_backups() {
        let (_directory, paths, store) = fixture();
        apply(&paths, &store).unwrap();
        let installed = text(&paths);
        let env = std::fs::read(env_path(&paths)).unwrap();
        let catalogs = generated_catalogs(&paths);
        let backups = std::fs::read_dir(paths.app_data_dir.join("config-backups"))
            .unwrap()
            .count();
        let second = apply(&paths, &store).unwrap();
        assert!(second.backup_path.is_none());
        assert_eq!(text(&paths), installed);
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
        assert_eq!(generated_catalogs(&paths), catalogs);
        assert_eq!(
            std::fs::read_dir(paths.app_data_dir.join("config-backups"))
                .unwrap()
                .count(),
            backups
        );
    }

    #[test]
    fn native_metadata_updates_keep_loaded_credentials_and_replace_only_the_catalog() {
        let (_directory, paths, mut store) = fixture();
        apply(&paths, &store).unwrap();
        let original_id = installed_id(&paths);
        let original_catalog = installed_catalog_path(&paths, &document(&paths)).unwrap();
        let env = std::fs::read(env_path(&paths)).unwrap();
        store.providers[0].name = "Updated model metadata".into();
        store.providers[0].models[0].context_window = Some(250000);
        apply(&paths, &store).unwrap();
        assert_eq!(installed_id(&paths), original_id);
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
        assert!(!original_catalog.exists());
        assert_eq!(generated_catalogs(&paths).len(), 1);
        assert!(intact(&paths, &store).unwrap());
        std::fs::remove_file(config_snapshot::path(&paths)).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert!(EnvFile::load(&paths).unwrap().owned.is_empty());
        assert!(generated_catalogs(&paths).is_empty());
        assert_eq!(
            text(&paths).parse::<toml::Value>().unwrap(),
            ORIGINAL.parse::<toml::Value>().unwrap()
        );
    }

    #[test]
    fn native_provider_or_key_switch_allocates_a_new_generation() {
        let (_directory, paths, mut store) = fixture();
        apply(&paths, &store).unwrap();
        let original_id = installed_id(&paths);
        store.providers[0].api_key = "synthetic-rotated-key".into();
        apply(&paths, &store).unwrap();
        let rotated_id = installed_id(&paths);
        assert_ne!(rotated_id, original_id);
        assert!(!catalog_path(&paths, original_id).exists());
        assert_eq!(EnvFile::load(&paths).unwrap().owned.len(), 1);
        assert_eq!(
            EnvFile::load(&paths).unwrap().owned[0].key,
            "synthetic-rotated-key"
        );
        store.providers[0].base_url = "https://second.example.invalid".into();
        apply(&paths, &store).unwrap();
        let switched_id = installed_id(&paths);
        assert_ne!(switched_id, rotated_id);
        assert!(!catalog_path(&paths, rotated_id).exists());
        assert_eq!(
            document(&paths)["model_providers"][PROVIDER_KEY]["base_url"].as_str(),
            Some("https://second.example.invalid/v1")
        );
    }

    #[test]
    fn native_failed_config_backup_keeps_old_generation_and_cleans_only_staged_key() {
        let (_directory, paths, mut store) = fixture();
        std::fs::write(env_path(&paths), USER_ENV).unwrap();
        apply(&paths, &store).unwrap();
        let installed = text(&paths);
        let env = std::fs::read(env_path(&paths)).unwrap();
        let catalogs = generated_catalogs(&paths);
        let backup_dir = paths.app_data_dir.join("config-backups");
        let retained_backups = paths.app_data_dir.join("retained-test-backups");
        std::fs::rename(&backup_dir, &retained_backups).unwrap();
        std::fs::write(&backup_dir, "prevent backup-directory creation").unwrap();
        store.providers[0].api_key = "synthetic-next-key".into();
        assert!(apply(&paths, &store).is_err());
        assert_eq!(text(&paths), installed);
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
        assert_eq!(generated_catalogs(&paths), catalogs);
        assert_eq!(std::fs::read(&paths.auth_path).unwrap(), AUTH);
        std::fs::remove_file(&backup_dir).unwrap();
        std::fs::rename(&retained_backups, &backup_dir).unwrap();
        apply(&paths, &store).unwrap();
        assert_eq!(EnvFile::load(&paths).unwrap().owned.len(), 1);
        assert!(intact(&paths, &store).unwrap());
    }

    #[test]
    fn native_malformed_or_ambiguous_owned_env_fails_without_secret_errors_or_writes() {
        let id = Uuid::new_v4();
        let block = credential_block(id, "synthetic-env-secret");
        let variants = [
            block.replace("synthetic-env-secret", "synthetic-changed-secret"),
            format!("{block}{block}"),
            format!("{}=\"synthetic-env-secret\"\n", variable_name(id)),
            block.replace(&format!("{END}{id} ---"), "# damaged closing marker"),
        ];
        for original in variants {
            let (_directory, paths, store) = fixture();
            std::fs::write(env_path(&paths), &original).unwrap();
            let error = apply(&paths, &store).err().unwrap();
            assert_eq!(error, CREDENTIAL_ERROR);
            assert!(!error.contains("synthetic"));
            assert_eq!(std::fs::read_to_string(env_path(&paths)).unwrap(), original);
            assert_eq!(text(&paths), ORIGINAL);
            assert!(!config_snapshot::path(&paths).exists());
        }
    }

    #[test]
    fn native_env_errors_explain_safe_rejection_without_exposing_or_overwriting_secrets() {
        let (_directory, paths, store) = fixture();
        for (bytes, expected) in [
            (
                b"USER_KEY=\"unterminated-synthetic-secret\n".to_vec(),
                "newapi.errors.nativeEnvSyntax",
            ),
            (vec![0xff], "newapi.errors.nativeEnvSyntax"),
            (
                vec![b' '; MAX_ENV_BYTES as usize + 1],
                "newapi.errors.nativeEnvTooLarge",
            ),
        ] {
            std::fs::write(env_path(&paths), &bytes).unwrap();
            let error = apply(&paths, &store).unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.contains("synthetic"));
            assert_eq!(std::fs::read(env_path(&paths)).unwrap(), bytes);
            assert_eq!(text(&paths), ORIGINAL);
        }
        std::fs::remove_file(env_path(&paths)).unwrap();
        std::fs::create_dir(env_path(&paths)).unwrap();
        assert_eq!(
            apply(&paths, &store).unwrap_err(),
            "newapi.errors.nativeEnvFileType"
        );
        assert!(env_path(&paths).is_dir());
        std::fs::remove_dir(env_path(&paths)).unwrap();
        std::fs::write(env_path(&paths), USER_ENV).unwrap();
        let file = EnvFile::load(&paths).unwrap();
        let changed = format!("{USER_ENV}\nCONCURRENT_USER_KEY=synthetic-secret\n");
        std::fs::write(env_path(&paths), &changed).unwrap();
        assert_eq!(
            config_update_error(file.write(&paths, USER_ENV).unwrap_err()),
            "newapi.errors.nativeEnvChanged"
        );
        assert_eq!(std::fs::read_to_string(env_path(&paths)).unwrap(), changed);
        assert_eq!(text(&paths), ORIGINAL);
    }

    #[test]
    fn native_reapply_rejects_manual_auth_overrides_without_erasing_them() {
        for key in ["experimental_bearer_token", "auth", "aws"] {
            let (_directory, paths, mut store) = fixture();
            apply(&paths, &store).unwrap();
            let mut edited = document(&paths);
            edited["model_providers"][PROVIDER_KEY][key] = toml_edit::value("synthetic-user-auth");
            std::fs::write(&paths.config_path, edited.to_string()).unwrap();
            let before = text(&paths);
            let env = std::fs::read(env_path(&paths)).unwrap();
            let catalogs = generated_catalogs(&paths);
            store.providers[0].name = "Updated metadata".into();
            assert_eq!(
                apply(&paths, &store).unwrap_err(),
                "newapi.errors.nativeAuthOverrides"
            );
            assert_eq!(text(&paths), before);
            assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
            assert_eq!(generated_catalogs(&paths), catalogs);
            assert_eq!(std::fs::read(&paths.auth_path).unwrap(), AUTH);
        }
    }

    #[test]
    fn native_cleanup_keeps_referenced_keys_and_preserves_foreign_files() {
        let (_directory, paths, store) = fixture();
        std::fs::write(env_path(&paths), USER_ENV).unwrap();
        apply(&paths, &store).unwrap();
        let active = installed_id(&paths);
        let stale = Uuid::new_v4();
        let retained = Uuid::new_v4();
        let updated = format!(
            "{}{}{}",
            std::fs::read_to_string(env_path(&paths)).unwrap(),
            credential_block(stale, "synthetic-staged-secret"),
            credential_block(retained, "synthetic-referenced-secret")
        );
        std::fs::write(env_path(&paths), updated).unwrap();
        std::fs::write(catalog_path(&paths, stale), "{}").unwrap();
        std::fs::write(catalog_path(&paths, retained), "{}").unwrap();
        let foreign_path = paths.app_data_dir.join("codex-native-user.json");
        std::fs::write(&foreign_path, "user-owned catalog").unwrap();
        let mut config = document(&paths);
        config["model_providers"]["foreign"] = Item::Table(toml_edit::Table::new());
        config["model_providers"]["foreign"]["env_key"] = toml_edit::value(variable_name(retained));
        std::fs::write(&paths.config_path, config.to_string()).unwrap();
        cleanup_credentials(&paths).unwrap();
        let file = EnvFile::load(&paths).unwrap();
        let ids: HashSet<_> = file.owned.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, HashSet::from([active, retained]));
        assert!(file.text.starts_with(USER_ENV));
        assert!(!catalog_path(&paths, stale).exists());
        assert!(catalog_path(&paths, retained).exists());
        assert_eq!(
            std::fs::read_to_string(foreign_path).unwrap(),
            "user-owned catalog"
        );
    }

    #[test]
    fn native_cleanup_respects_inline_provider_credential_references() {
        let (_directory, paths, store) = fixture();
        apply(&paths, &store).unwrap();
        let id = installed_id(&paths);
        let env = std::fs::read(env_path(&paths)).unwrap();
        // A user's valid inline representation still owns its env reference.
        let changed = format!(
            "model_providers = {{ foreign = {{ env_key = '{}' }} }}\n",
            variable_name(id)
        );
        std::fs::write(&paths.config_path, &changed).unwrap();
        cleanup_credentials(&paths).unwrap();
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
        assert!(catalog_path(&paths, id).exists());
        assert_eq!(text(&paths), changed);
    }

    #[test]
    fn native_key_rotation_keeps_credentials_referenced_by_supplemental_headers() {
        let (_directory, paths, mut store) = fixture();
        apply(&paths, &store).unwrap();
        let original_id = installed_id(&paths);
        let original_name = variable_name(original_id);
        let mut changed = document(&paths);
        changed["model_providers"][PROVIDER_KEY]["env_http_headers"] =
            toml_edit::value(toml_edit::InlineTable::new());
        changed["model_providers"][PROVIDER_KEY]["env_http_headers"]["X-API-Key"] =
            toml_edit::value(&original_name);
        changed["model_providers"]["foreign"] = Item::Table(toml_edit::Table::new());
        changed["model_providers"]["foreign"]["env_http_headers"] =
            Item::Table(toml_edit::Table::new());
        changed["model_providers"]["foreign"]["env_http_headers"]["X-API-Key"] =
            toml_edit::value(&original_name);
        std::fs::write(&paths.config_path, changed.to_string()).unwrap();
        store.providers[0].api_key = "synthetic-rotated-secret".into();
        apply(&paths, &store).unwrap();
        assert_ne!(installed_id(&paths), original_id);
        assert!(EnvFile::load(&paths)
            .unwrap()
            .owned
            .iter()
            .any(|entry| entry.id == original_id));
        assert_eq!(
            document(&paths)["model_providers"][PROVIDER_KEY]["env_http_headers"]["X-API-Key"]
                .as_str(),
            Some(original_name.as_str())
        );
        remove_routing_config(&paths, &store).unwrap();
        let owned = EnvFile::load(&paths).unwrap().owned;
        assert_eq!(owned.len(), 1);
        assert_eq!(owned[0].id, original_id);
        let mut changed = document(&paths);
        changed["model_providers"]["foreign"]
            .as_table_mut()
            .unwrap()
            .remove("env_http_headers");
        std::fs::write(&paths.config_path, changed.to_string()).unwrap();
        cleanup_credentials(&paths).unwrap();
        assert!(EnvFile::load(&paths).unwrap().owned.is_empty());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn native_cleanup_keeps_current_catalog_with_equivalent_windows_spelling() {
        let (_directory, paths, store) = fixture();
        apply(&paths, &store).unwrap();
        let mut changed = document(&paths);
        let original = changed["model_catalog_json"].as_str().unwrap().to_owned();
        let alternate = original.replace('\\', "/");
        assert_ne!(original, alternate);
        assert_eq!(PathBuf::from(&original), PathBuf::from(&alternate));
        changed["model_catalog_json"] = toml_edit::value(&alternate);
        std::fs::write(&paths.config_path, changed.to_string()).unwrap();
        apply(&paths, &store).unwrap();
        assert!(PathBuf::from(&alternate).is_file());
        assert!(intact(&paths, &store).unwrap());
        changed["model_catalog_json"] = toml_edit::value(original.to_ascii_uppercase());
        std::fs::write(&paths.config_path, changed.to_string()).unwrap();
        cleanup_credentials(&paths).unwrap();
        assert!(PathBuf::from(&original).is_file());
    }

    #[test]
    fn native_intact_does_not_treat_invalid_optional_field_types_as_absent() {
        for key in ["model_context_window", "model_auto_compact_token_limit"] {
            let (_directory, paths, store) = fixture();
            apply(&paths, &store).unwrap();
            let mut changed = document(&paths);
            changed[key] = toml_edit::value("invalid-integer");
            let changed = changed.to_string();
            std::fs::write(&paths.config_path, &changed).unwrap();
            assert!(!intact(&paths, &store).unwrap());
            assert_eq!(text(&paths), changed);
        }
    }

    #[test]
    fn native_cleanup_removes_stale_secrets_even_when_catalog_path_is_not_a_file() {
        let (_directory, paths, mut store) = fixture();
        apply(&paths, &store).unwrap();
        let previous_catalog = catalog_path(&paths, installed_id(&paths));
        std::fs::remove_file(&previous_catalog).unwrap();
        std::fs::create_dir(&previous_catalog).unwrap();
        store.providers[0].api_key = "synthetic-next-key".into();
        apply(&paths, &store).unwrap();
        assert!(intact(&paths, &store).unwrap());
        assert_eq!(EnvFile::load(&paths).unwrap().owned.len(), 1);
        assert!(
            previous_catalog.is_dir(),
            "Never delete a directory in place of a catalog"
        );
        std::fs::remove_dir(&previous_catalog).unwrap();
        cleanup_credentials(&paths).unwrap();
        assert_eq!(EnvFile::load(&paths).unwrap().owned.len(), 1);
        assert_eq!(
            EnvFile::load(&paths).unwrap().owned[0].key,
            "synthetic-next-key"
        );
    }

    #[test]
    fn native_cleanup_discovers_orphan_catalogs_without_an_env_block() {
        let (_directory, paths, store) = fixture();
        apply(&paths, &store).unwrap();
        let active = catalog_path(&paths, installed_id(&paths));
        let orphan = catalog_path(&paths, Uuid::new_v4());
        std::fs::write(&orphan, "{}").unwrap();
        cleanup_credentials(&paths).unwrap();
        assert!(!orphan.exists());
        assert!(active.exists());
        assert!(intact(&paths, &store).unwrap());
    }

    #[test]
    fn native_shared_context_never_overstates_a_model_override_and_default_is_not_applied() {
        let (_directory, paths, mut store) = fixture();
        store.providers[0].context_window = Some(200000);
        store.providers[0].models[1].overrides.context_window = Some(100000);
        apply(&paths, &store).unwrap();
        assert_eq!(
            document(&paths)["model_context_window"].as_integer(),
            Some(100000)
        );
        assert_eq!(
            native_limits(&paths, &store, &store.providers[0]).0,
            Some(100000)
        );
        store.compression = model_policy::CompressionPolicy::ClientDefault;
        apply(&paths, &store).unwrap();
        let catalog = models_dev::load(&paths.app_data_dir);
        let state = compression_state(&paths, &store, &catalog, &[]).unwrap();
        assert!(state.config_active);
        assert_eq!(state.applied, None);
        assert_eq!(state.applied_threshold, None);
    }

    #[test]
    fn native_missing_snapshot_without_backup_cannot_replace_existing_baseline() {
        let (_directory, paths, store) = fixture();
        apply(&paths, &store).unwrap();
        std::fs::remove_file(config_snapshot::path(&paths)).unwrap();
        for entry in std::fs::read_dir(paths.app_data_dir.join("config-backups")).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        let installed = text(&paths);
        let env = std::fs::read(env_path(&paths)).unwrap();
        let catalogs = generated_catalogs(&paths);
        assert!(!intact(&paths, &store).unwrap());
        assert_eq!(
            apply(&paths, &store).err().unwrap(),
            "newapi.errors.routerSnapshot"
        );
        assert_eq!(text(&paths), installed);
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
        assert_eq!(generated_catalogs(&paths), catalogs);
    }

    #[test]
    fn native_missing_journal_recovers_for_reapply_remove_and_router_switch() {
        for action in ["reapply", "remove", "router"] {
            let (_directory, paths, store) = fixture();
            apply(&paths, &store).unwrap();
            std::fs::remove_file(config_snapshot::path(&paths)).unwrap();
            assert!(!intact(&paths, &store).unwrap());
            match action {
                "reapply" => {
                    apply(&paths, &store).unwrap();
                    assert!(intact(&paths, &store).unwrap());
                }
                "router" => {
                    super::super::apply_routing_config(&paths, &store).unwrap();
                }
                _ => {}
            }
            remove_routing_config(&paths, &store).unwrap();
            let restored: toml::Value = text(&paths).parse().unwrap();
            assert_eq!(restored, ORIGINAL.parse::<toml::Value>().unwrap());
            assert!(EnvFile::load(&paths).unwrap().owned.is_empty());
            assert!(generated_catalogs(&paths).is_empty());
            assert_eq!(std::fs::read(&paths.auth_path).unwrap(), AUTH);
        }
    }

    #[test]
    fn native_missing_journal_rejects_ambiguous_baseline_and_preserves_user_drift() {
        let (_directory, paths, store) = fixture();
        apply(&paths, &store).unwrap();
        std::fs::remove_file(config_snapshot::path(&paths)).unwrap();
        let backup = paths
            .app_data_dir
            .join("config-backups")
            .join(format!("{}.toml", Uuid::new_v4()));
        std::fs::write(&backup, ORIGINAL.replace("user-model", "another-model")).unwrap();
        let installed = text(&paths);
        let env = std::fs::read(env_path(&paths)).unwrap();
        assert_eq!(
            remove_routing_config(&paths, &store).unwrap_err(),
            "newapi.errors.routerSnapshot"
        );
        assert_eq!(text(&paths), installed);
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
        std::fs::remove_file(backup).unwrap();
        let mut changed = document(&paths);
        changed["model"] = toml_edit::value("user-edited-model");
        changed["model_context_window"] = toml_edit::value(123456);
        std::fs::write(&paths.config_path, changed.to_string()).unwrap();
        remove_routing_config(&paths, &store).unwrap();
        assert_eq!(
            document(&paths)["model"].as_str(),
            Some("user-edited-model")
        );
        assert_eq!(
            document(&paths)["model_context_window"].as_integer(),
            Some(123456)
        );
    }

    #[test]
    fn native_headers_survive_reapply_and_are_never_copied_to_another_endpoint() {
        let (_directory, paths, mut store) = fixture();
        apply(&paths, &store).unwrap();
        let mut changed = document(&paths);
        changed["model_providers"][PROVIDER_KEY]["http_headers"] =
            toml_edit::value(toml_edit::InlineTable::new());
        changed["model_providers"][PROVIDER_KEY]["http_headers"]["X-Organization"] =
            toml_edit::value("fixture-org");
        changed["model_providers"][PROVIDER_KEY]["env_http_headers"] =
            toml_edit::value(toml_edit::InlineTable::new());
        changed["model_providers"][PROVIDER_KEY]["env_http_headers"]["X-API-Key"] =
            toml_edit::value("USER_KEY");
        std::fs::write(&paths.config_path, changed.to_string()).unwrap();
        assert!(intact(&paths, &store).unwrap());
        let id = installed_id(&paths);
        assert!(apply(&paths, &store).unwrap().backup_path.is_none());
        assert_eq!(installed_id(&paths), id);
        store.providers[0].name = "Renamed".into();
        store.providers[0].models[0].context_window = Some(250000);
        apply(&paths, &store).unwrap();
        let applied = document(&paths);
        for key in ["http_headers", "env_http_headers"] {
            assert_eq!(
                applied["model_providers"][PROVIDER_KEY][key]
                    .to_string()
                    .trim(),
                changed["model_providers"][PROVIDER_KEY][key]
                    .to_string()
                    .trim()
            );
        }
        let env = std::fs::read(env_path(&paths)).unwrap();
        store.providers[0].base_url = "https://another.example.invalid".into();
        assert_eq!(
            apply(&paths, &store).unwrap_err(),
            "newapi.errors.nativeHeaderScope"
        );
        assert_eq!(text(&paths), applied.to_string());
        assert_eq!(std::fs::read(env_path(&paths)).unwrap(), env);
    }

    #[test]
    fn native_endpoint_matches_routed_responses_for_non_v1_and_custom_roots() {
        for base in [
            "https://api.novita.ai/v3/openai",
            "https://api.z.ai/api/coding/paas/v4/",
            "https://open.bigmodel.cn/api/paas/v4",
            "https://api.deepseek.com/anthropic",
            "https://custom.example.invalid/prefix/v1/",
            "https://custom.example.invalid/prefix/",
        ] {
            let provider = Provider {
                base_url: base.into(),
                ..Default::default()
            };
            let routed = newapi::provider_endpoint(base, "v1/responses")
                .unwrap()
                .to_string();
            assert_eq!(
                format!(
                    "{}/responses",
                    endpoint(&provider).unwrap().trim_end_matches('/')
                ),
                routed
            );
        }
    }

    #[test]
    fn native_model_picker_selection_restores_original_but_external_model_is_preserved() {
        for (selection, expected) in [
            ("second-response", "user-model"),
            ("user-custom-model", "user-custom-model"),
        ] {
            let (_directory, paths, store) = fixture();
            apply(&paths, &store).unwrap();
            let mut config = document(&paths);
            config["model"] = toml_edit::value(selection);
            config["theme"] = toml_edit::value("light");
            std::fs::write(&paths.config_path, config.to_string()).unwrap();
            remove_routing_config(&paths, &store).unwrap();
            let restored = document(&paths);
            assert_eq!(restored["model"].as_str(), Some(expected));
            assert_eq!(restored["theme"].as_str(), Some("light"));
            assert_eq!(restored["model_provider"].as_str(), Some("custom"));
            assert_eq!(restored["preferred_auth_method"].as_str(), Some("chatgpt"));
            assert!(EnvFile::load(&paths).unwrap().owned.is_empty());
            assert!(!config_snapshot::path(&paths).exists());
        }
    }
    #[test]
    fn native_model_ids_exclude_a_chat_default() {
        let provider = Provider {
            model: "chat-default".into(),
            protocol: ModelProtocol::ChatCompletions,
            models: vec![model("responses-model", ModelProtocol::Responses, 1000)],
            ..Default::default()
        };
        assert_eq!(model_ids(&provider), vec!["responses-model"]);
    }
}
