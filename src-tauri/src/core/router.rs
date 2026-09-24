use super::{
    codex_config::{CodexConfigRepository, ConfigWriteResult},
    models::{ApiProxyConfigPayload, CoreError},
    newapi, router_balance, secret_file, storage,
};
use crate::platform::paths::CodexPaths;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

pub const DEFAULT_PORT: u16 = 43127;
const PREVIOUS_DEFAULT_PORT: u16 = 25817;
pub const PROVIDER_PREFIX: &str = "agentharbor_p";
pub const CATALOG_PROVIDER_KEY: &str = "agentharbor_relay";
const DIRECT_PROVIDER_KEY: &str = "agentharbor_newapi";
const PREVIOUS_PROVIDER_PREFIX: &str = "codexharbor_p";
const PREVIOUS_CATALOG_PROVIDER_KEY: &str = "codexharbor_relay";
const PREVIOUS_DIRECT_PROVIDER_KEY: &str = "codexharbor_newapi";
const PREVIOUS_RELAY_MODEL_PREFIX: &str = "aimami_relay_";
const MAX_PROVIDERS: usize = 64;
const MAX_MODELS_PER_PROVIDER: usize = 512;
const STORE_VERSION: u32 = 4;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ModelProtocol {
    #[default]
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

pub struct ModelEndpoint<'a> {
    pub base_url: &'a str,
    pub protocol: ModelProtocol,
}

pub struct ModelCredential<'a> {
    pub api_key: &'a str,
}

pub struct BalanceCredential<'a> {
    pub user_id: Option<u64>,
    pub access_token: &'a str,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub user_id: Option<u64>,
    pub access_token: String,
    pub api_key: String,
    #[serde(default)]
    pub protocol: ModelProtocol,
    pub model: String,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
    pub enabled: bool,
    /// 该供应商的网络出口：proxy = 跟随全局代理设置，direct = 直连不走代理。
    #[serde(default)]
    pub network_mode: String,
    pub models: Vec<newapi::Model>,
    pub models_fetched_at: Option<i64>,
    pub balance: Option<newapi::Balance>,
    pub balance_source: Option<String>,
    pub test_status: String,
    pub test_at: Option<i64>,
    /// Read only during migration from builds that exposed the removed Chat wire protocol.
    #[serde(rename = "wireApi", default = "default_wire_api", skip_serializing)]
    pub(crate) legacy_wire_api: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_wire_api() -> String {
    "responses".to_owned()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RouterSettings {
    pub enabled: bool,
    pub accountless: bool,
    pub port: u16,
    pub bearer_token: String,
}

impl Default for RouterSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            accountless: false,
            port: DEFAULT_PORT,
            bearer_token: String::new(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    pub version: u32,
    pub providers: Vec<Provider>,
    pub router: RouterSettings,
    pub active_provider_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub user_id: Option<u64>,
    pub has_access_token: bool,
    pub has_api_key: bool,
    pub protocol: ModelProtocol,
    pub model: String,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
    pub enabled: bool,
    pub network_mode: String,
    pub models: Vec<newapi::Model>,
    pub models_fetched_at: Option<i64>,
    pub balance: Option<newapi::Balance>,
    pub balance_source: Option<String>,
    pub test_status: String,
    pub test_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Provider {
    pub fn view(&self) -> ProviderView {
        ProviderView {
            id: self.id.clone(),
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            user_id: self.user_id,
            has_access_token: !self.access_token.is_empty(),
            has_api_key: !self.api_key.is_empty(),
            protocol: self.protocol,
            model: self.model.clone(),
            context_window: self.context_window,
            auto_compact_limit: self.auto_compact_limit,
            enabled: self.enabled,
            network_mode: self.network_mode().to_owned(),
            models: self.models.clone(),
            models_fetched_at: self.models_fetched_at,
            balance: self.balance.clone(),
            balance_source: self.balance_source.clone(),
            test_status: self.test_status.clone(),
            test_at: self.test_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    pub(crate) fn is_routable(&self) -> bool {
        self.enabled
            && !self.api_key.is_empty()
            && !self.model.is_empty()
            && self.model_supports_protocol()
    }

    pub(crate) fn is_routable_for(&self, protocol: ModelProtocol) -> bool {
        self.protocol == protocol && self.is_routable()
    }

    pub fn model_endpoint(&self) -> ModelEndpoint<'_> {
        ModelEndpoint {
            base_url: &self.base_url,
            protocol: self.protocol,
        }
    }

    pub fn model_credential(&self) -> ModelCredential<'_> {
        ModelCredential {
            api_key: &self.api_key,
        }
    }

    pub fn balance_credential(&self) -> BalanceCredential<'_> {
        BalanceCredential {
            user_id: self.user_id,
            access_token: &self.access_token,
        }
    }

    /// proxy = 跟随全局代理（默认），direct = 直连。
    pub fn network_mode(&self) -> &str {
        if self.network_mode == "direct" {
            "direct"
        } else {
            "proxy"
        }
    }

    fn model_supports_protocol(&self) -> bool {
        self.supports_model(&self.model)
    }

    pub(crate) fn supports_model(&self, model_id: &str) -> bool {
        match self.models.iter().find(|model| model.id == model_id) {
            None => true,
            Some(model) => model_supports_protocol(model, self.protocol),
        }
    }
}

fn model_supports_protocol(model: &newapi::Model, protocol: ModelProtocol) -> bool {
    let endpoint_types: Vec<String> = model
        .endpoint_types
        .iter()
        .map(|endpoint| endpoint.to_ascii_lowercase())
        .collect();
    if endpoint_types.is_empty() {
        return true;
    }
    match protocol {
        ModelProtocol::Responses => {
            endpoint_types
                .iter()
                .any(|endpoint| endpoint.contains("response"))
                || !endpoint_types.iter().any(|endpoint| {
                    endpoint.contains("chat")
                        || endpoint.contains("completion")
                        || endpoint.contains("anthropic")
                        || endpoint.contains("message")
                })
        }
        ModelProtocol::ChatCompletions => {
            endpoint_types
                .iter()
                .any(|endpoint| endpoint.contains("chat") || endpoint.contains("completion"))
                || !endpoint_types.iter().any(|endpoint| {
                    endpoint.contains("response")
                        || endpoint.contains("anthropic")
                        || endpoint.contains("message")
                })
        }
        ModelProtocol::AnthropicMessages => {
            endpoint_types
                .iter()
                .any(|endpoint| endpoint.contains("anthropic") || endpoint.contains("message"))
                || !endpoint_types.iter().any(|endpoint| {
                    endpoint.contains("response")
                        || endpoint.contains("chat")
                        || endpoint.contains("completion")
                })
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterView {
    pub enabled: bool,
    pub accountless: bool,
    pub port: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewApiState {
    pub providers: Vec<ProviderView>,
    pub router: RouterView,
    pub proxy_running: bool,
    pub active_provider_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateChange {
    pub state: NewApiState,
    pub applied: Option<newapi::AppliedConfig>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub state: NewApiState,
    pub imported: usize,
    pub skipped: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub user_id: Option<u64>,
    pub access_token: String,
    pub api_key: String,
    #[serde(default)]
    pub protocol: ModelProtocol,
    pub model: String,
    #[serde(default)]
    pub models: Vec<newapi::Model>,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
    /// 可选：proxy / direct；编辑时留空表示保持原设置。
    #[serde(default)]
    pub network_mode: String,
}

fn now() -> i64 {
    Utc::now().timestamp()
}

fn store_path(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join("newapi-providers.dat")
}

static STORE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn store_write_guard() -> Result<MutexGuard<'static, ()>, String> {
    STORE_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "newapi.errors.storage".to_owned())
}

fn load_store_unlocked(paths: &CodexPaths) -> Result<Store, String> {
    if let Some(bytes) = secret_file::read(&store_path(paths))? {
        let mut store: Store =
            serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.storage")?;
        let mut changed = false;
        let mut removed_incompatible_route = false;
        if store.version < 2 {
            // Older builds treated a successful model-list request as a
            // connectivity test. Require one real chat request before showing
            // a provider as reachable.
            for provider in &mut store.providers {
                provider.test_status = "untested".into();
                provider.test_at = None;
            }
            changed = true;
        }
        let mut migrated_previous_identity = store.router.port == PREVIOUS_DEFAULT_PORT
            || store.router.bearer_token.starts_with("aimami_")
            || store.router.bearer_token.starts_with("codexharbor_");
        if store.router.bearer_token.is_empty() || migrated_previous_identity {
            store.router.bearer_token = new_router_bearer();
            changed = true;
        }
        if store.router.port == PREVIOUS_DEFAULT_PORT {
            store.router.port = DEFAULT_PORT;
            changed = true;
        }
        let previous_active = store.active_provider_id.clone();
        let mut migrated_active = None;
        for provider in &mut store.providers {
            if provider.id.starts_with("aimami_") {
                migrated_previous_identity = true;
                let previous_id =
                    std::mem::replace(&mut provider.id, uuid::Uuid::new_v4().to_string());
                if previous_active.as_deref() == Some(previous_id.as_str()) {
                    migrated_active = Some(provider.id.clone());
                }
                changed = true;
            }
        }
        if store.version < 4 {
            for provider in &mut store.providers {
                for model in &mut provider.models {
                    *model = std::mem::take(model).with_local_defaults();
                }
                if !provider.model.trim().is_empty()
                    && !provider
                        .models
                        .iter()
                        .any(|model| model.id == provider.model)
                {
                    provider.models.push(
                        newapi::Model {
                            id: provider.model.clone(),
                            display_name: provider.model.clone(),
                            included: true,
                            metadata_source: "local".into(),
                            ..Default::default()
                        }
                        .with_local_defaults(),
                    );
                }
            }
            changed = true;
        }
        if migrated_active.is_some() {
            store.active_provider_id = migrated_active;
        }
        if migrated_previous_identity && store.router.enabled {
            // Import credentials and choices, but require an explicit enable in
            // AgentHarbor so the old application's active route is not replaced.
            store.router.enabled = false;
            removed_incompatible_route = true;
            changed = true;
        }
        for provider in &mut store.providers {
            match provider.legacy_wire_api.as_str() {
                "responses" => {}
                "chat" => {
                    provider.protocol = ModelProtocol::ChatCompletions;
                    provider.legacy_wire_api = default_wire_api();
                    changed = true;
                }
                _ => {
                    provider.enabled = false;
                    if store.active_provider_id.as_deref() == Some(provider.id.as_str()) {
                        store.active_provider_id = None;
                    }
                    provider.legacy_wire_api = default_wire_api();
                    changed = true;
                }
            }
        }
        if store.version < STORE_VERSION {
            store.version = STORE_VERSION;
            changed = true;
        }
        if store.router.enabled && effective_active_for(&store, ModelProtocol::Responses).is_none()
        {
            store.router.enabled = false;
            removed_incompatible_route = true;
            changed = true;
        }
        if removed_incompatible_route {
            remove_routing_config(paths, &store)?;
        }
        if changed {
            save_store_unlocked(paths, &store)?;
        }
        return Ok(store);
    }
    let mut store = match migrate_original_relay(paths)? {
        Some(store) => store,
        None => migrate_legacy_connection(paths)?.unwrap_or_default(),
    };
    store.version = STORE_VERSION;
    store.router.bearer_token = new_router_bearer();
    // Persist the promoted store so later reads do not depend on the foreign files.
    save_store_unlocked(paths, &store)?;
    Ok(store)
}

pub fn load_store(paths: &CodexPaths) -> Result<Store, String> {
    let _guard = store_write_guard()?;
    load_store_unlocked(paths)
}

fn new_router_bearer() -> String {
    format!("agentharbor_{}", uuid::Uuid::new_v4().simple())
}

// The previous AiMaMi release keeps its relay data in `codexmate/relay/`:
// state.json holds providers/settings, keys/<provider id> holds plaintext API keys.
// Promoting it lets users of the installed app see their real relays immediately.
fn migrate_original_relay(paths: &CodexPaths) -> Result<Option<Store>, String> {
    let relay_dir = paths.codex_home.join("codexmate").join("relay");
    let Ok(text) = std::fs::read_to_string(relay_dir.join("state.json")) else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return Ok(None);
    };
    let items = value
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let active: Vec<String> = value
        .pointer("/activeByIde/codex")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut providers = Vec::new();
    for item in &items {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 100)
        else {
            continue;
        };
        let Ok(base_url) =
            newapi::base_url(item.get("baseUrl").and_then(Value::as_str).unwrap_or(""))
        else {
            continue;
        };
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("relay")
            .trim()
            .to_owned();
        let api_key = if item.get("apiKeyStored").and_then(Value::as_bool) == Some(true) {
            std::fs::read_to_string(relay_dir.join("keys").join(id))
                .map(|k| k.trim().to_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut models = Vec::new();
        for entry in item
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(model_id) = entry
                .get("model")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty() && s.len() <= 200)
            else {
                continue;
            };
            if models.iter().any(|m: &newapi::Model| m.id == model_id) {
                continue;
            }
            models.push(newapi::Model {
                id: model_id.to_owned(),
                context_window: entry
                    .get("contextWindow")
                    .and_then(Value::as_u64)
                    .filter(|v| *v > 0),
                max_output_tokens: None,
                endpoint_types: Vec::new(),
                ..Default::default()
            });
        }
        let updated = item.get("updatedAt").and_then(Value::as_i64).unwrap_or(0) / 1000;
        providers.push(Provider {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            base_url: base_url.to_string(),
            api_key,
            model: item
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned(),
            models_fetched_at: (!models.is_empty() && updated > 0).then_some(updated),
            enabled: active.iter().any(|a| a == id),
            test_status: "untested".to_owned(),
            created_at: item.get("createdAt").and_then(Value::as_i64).unwrap_or(0) / 1000,
            updated_at: updated,
            models,
            ..Default::default()
        });
    }
    if providers.is_empty() {
        return Ok(None);
    }
    let active_provider_id = providers
        .iter()
        .find(|provider| provider.enabled)
        .map(|provider| provider.id.clone());
    Ok(Some(Store {
        version: STORE_VERSION,
        providers,
        active_provider_id,
        router: RouterSettings {
            enabled: false,
            accountless: value
                .get("codexRouterNoAccountMode")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            port: DEFAULT_PORT,
            bearer_token: String::new(),
        },
    }))
}

/// Read a store from an explicit path (used by the local proxy per request).
pub fn read_store(path: &Path) -> Result<Store, String> {
    match secret_file::read(path)? {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.storage".into()),
        None => Ok(Store::default()),
    }
}

// Promote the previous single-connection setup into the first provider card.
fn migrate_legacy_connection(paths: &CodexPaths) -> Result<Option<Store>, String> {
    let bytes = match secret_file::read(&paths.app_data_dir.join("newapi-connection.dat"))? {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    let legacy: newapi::Connection = match serde_json::from_slice(&bytes) {
        Ok(legacy) => legacy,
        Err(_) => return Ok(None),
    };
    if legacy.base_url.is_empty() {
        return Ok(None);
    }
    let name = newapi::base_url(&legacy.base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "relay".to_owned());
    let provider = Provider {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        base_url: legacy.base_url,
        user_id: legacy.user_id,
        access_token: legacy.access_token,
        api_key: legacy.api_key,
        model: legacy.model,
        context_window: legacy.context_window,
        auto_compact_limit: legacy.auto_compact_limit,
        enabled: false,
        test_status: "untested".into(),
        created_at: now(),
        updated_at: now(),
        ..Default::default()
    };
    Ok(Some(Store {
        version: STORE_VERSION,
        providers: vec![provider],
        ..Default::default()
    }))
}

fn save_store_unlocked(paths: &CodexPaths, store: &Store) -> Result<(), String> {
    let bytes = serde_json::to_vec(store).map_err(|_| "newapi.errors.storage")?;
    secret_file::write(&store_path(paths), &bytes)
}

pub(crate) fn state_from_store(store: &Store, proxy_running: bool) -> NewApiState {
    NewApiState {
        providers: store.providers.iter().map(Provider::view).collect(),
        router: RouterView {
            enabled: store.router.enabled,
            accountless: store.router.accountless,
            port: store.router.port,
        },
        proxy_running,
        active_provider_id: effective_active_for(store, ModelProtocol::Responses)
            .map(|provider| provider.id.clone()),
    }
}

/// The provider traffic is routed to: the pinned one, else the first enabled.
pub fn effective_active(store: &Store) -> Option<&Provider> {
    store
        .active_provider_id
        .as_ref()
        .and_then(|id| {
            store
                .providers
                .iter()
                .find(|p| &p.id == id && p.is_routable())
        })
        .or_else(|| store.providers.iter().find(|p| p.is_routable()))
}

pub fn routable(store: &Store) -> Vec<&Provider> {
    store.providers.iter().filter(|p| p.is_routable()).collect()
}

fn validate_input(input: &mut ProviderInput) -> Result<(), String> {
    input.name = input.name.trim().to_owned();
    if input.name.is_empty() || input.name.chars().count() > 80 {
        return Err("newapi.errors.name".into());
    }
    input.base_url = newapi::base_url(&input.base_url)?.to_string();
    if input.user_id == Some(0)
        || input
            .user_id
            .is_some_and(|value| value > 9_007_199_254_740_991)
    {
        return Err("newapi.errors.userId".into());
    }
    input.access_token = input.access_token.trim().to_owned();
    input.api_key = input.api_key.trim().to_owned();
    input.model = input.model.trim().to_owned();
    normalize_models(&mut input.models, &input.model)?;
    for token in [&input.access_token, &input.api_key] {
        if !token.is_empty() {
            newapi::auth_header(token)?;
        }
    }
    newapi::validate_limits(&newapi::Connection {
        base_url: input.base_url.clone(),
        model: input.model.clone(),
        context_window: input.context_window,
        auto_compact_limit: input.auto_compact_limit,
        ..Default::default()
    })?;
    Ok(())
}

fn normalize_models(models: &mut Vec<newapi::Model>, default_model: &str) -> Result<(), String> {
    if models.len() > MAX_MODELS_PER_PROVIDER {
        return Err("newapi.errors.tooManyModels".into());
    }
    let mut ids = HashSet::new();
    for model in models.iter_mut() {
        model.id = model.id.trim().to_owned();
        model.display_name = model.display_name.trim().to_owned();
        if model.id.is_empty()
            || model.id.len() > 200
            || model.id.contains("::")
            || model.id.contains(['\r', '\n'])
            || !ids.insert(model.id.clone())
        {
            return Err("newapi.errors.modelList".into());
        }
        if model.display_name.len() > 120 || model.display_name.contains(['\r', '\n']) {
            return Err("newapi.errors.modelList".into());
        }
        model
            .endpoint_types
            .retain(|value| value.len() <= 80 && !value.contains(['\r', '\n']));
        model.endpoint_types.truncate(16);
        *model = std::mem::take(model).with_local_defaults();
        if model
            .context_window
            .is_some_and(|value| !(1_024..=100_000_000).contains(&value))
            || model
                .max_output_tokens
                .is_some_and(|value| value == 0 || value > 100_000_000)
            || matches!((model.context_window, model.max_output_tokens), (Some(context), Some(output)) if output > context)
        {
            return Err("newapi.errors.context".into());
        }
        if !matches!(
            model.metadata_source.as_str(),
            "upstream" | "local" | "manual"
        ) {
            model.metadata_source = "manual".into();
        }
    }
    if !default_model.is_empty() {
        if let Some(model) = models.iter_mut().find(|model| model.id == default_model) {
            model.included = true;
        } else {
            if models.len() >= MAX_MODELS_PER_PROVIDER {
                return Err("newapi.errors.tooManyModels".into());
            }
            models.push(
                newapi::Model {
                    id: default_model.to_owned(),
                    display_name: default_model.to_owned(),
                    included: true,
                    metadata_source: "local".into(),
                    ..Default::default()
                }
                .with_local_defaults(),
            );
        }
    }
    Ok(())
}

fn apply_routing_if_enabled(
    paths: &CodexPaths,
    store: &Store,
) -> Result<Option<newapi::AppliedConfig>, String> {
    if store.router.enabled {
        Ok(Some(apply_routing_config(paths, store)?))
    } else {
        Ok(None)
    }
}

pub fn save_provider(paths: &CodexPaths, mut input: ProviderInput) -> Result<StateChange, String> {
    validate_input(&mut input)?;
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let timestamp = now();
    let applied;
    match input
        .id
        .as_ref()
        .and_then(|id| store.providers.iter_mut().find(|p| &p.id == id))
    {
        Some(provider) => {
            // Never silently send credentials from one site/account to another.
            let same_site = provider.base_url == input.base_url;
            if same_site {
                if input.access_token.is_empty() && provider.user_id == input.user_id {
                    input.access_token = provider.access_token.clone();
                }
                if input.api_key.is_empty() {
                    input.api_key = provider.api_key.clone();
                }
            }
            let next_network_mode = if matches!(input.network_mode.as_str(), "proxy" | "direct") {
                input.network_mode.clone()
            } else {
                provider.network_mode().to_owned()
            };
            let models_changed =
                provider.base_url != input.base_url || provider.api_key != input.api_key;
            let balance_changed = models_changed
                || provider.user_id != input.user_id
                || provider.access_token != input.access_token;
            let connection_changed = models_changed
                || provider.protocol != input.protocol
                || provider.model != input.model
                || provider.models != input.models
                || provider.network_mode() != next_network_mode;
            provider.name = input.name.clone();
            provider.base_url = input.base_url.clone();
            provider.user_id = input.user_id;
            provider.access_token = input.access_token.clone();
            provider.api_key = input.api_key.clone();
            provider.protocol = input.protocol;
            provider.model = input.model.clone();
            provider.models = input.models.clone();
            provider.context_window = input.context_window;
            provider.auto_compact_limit = input.auto_compact_limit;
            provider.network_mode = next_network_mode;
            provider.updated_at = timestamp;
            provider.models_fetched_at = (!provider.models.is_empty()).then_some(timestamp);
            if balance_changed {
                provider.balance = None;
                provider.balance_source = None;
            }
            if connection_changed {
                provider.test_status = "untested".into();
                provider.test_at = None;
            }
            applied = apply_routing_if_enabled(paths, &store)?;
        }
        None => {
            if input.id.is_some() {
                return Err("newapi.errors.notFound".into());
            }
            if store.providers.len() >= MAX_PROVIDERS {
                return Err("newapi.errors.tooMany".into());
            }
            let provider = Provider {
                id: uuid::Uuid::new_v4().to_string(),
                name: input.name.clone(),
                base_url: input.base_url.clone(),
                user_id: input.user_id,
                access_token: input.access_token.clone(),
                api_key: input.api_key.clone(),
                protocol: input.protocol,
                model: input.model.clone(),
                models: input.models.clone(),
                models_fetched_at: (!input.models.is_empty()).then_some(timestamp),
                context_window: input.context_window,
                auto_compact_limit: input.auto_compact_limit,
                enabled: false,
                network_mode: if input.network_mode == "direct" {
                    "direct".into()
                } else {
                    "proxy".into()
                },
                test_status: "untested".into(),
                created_at: timestamp,
                updated_at: timestamp,
                ..Default::default()
            };
            store.providers.push(provider);
            // A new disabled provider cannot change routing, but re-apply is harmless and keeps
            // the config consistent with the store after the mutation path below.
            applied = apply_routing_if_enabled(paths, &store)?;
        }
    }
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store(&store, super::router_proxy::is_running()),
        applied,
    })
}

pub fn delete_provider(paths: &CodexPaths, id: &str) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    if !store.providers.iter().any(|p| p.id == id) {
        return Err("newapi.errors.notFound".into());
    }
    store.providers.retain(|p| p.id != id);
    if store.active_provider_id.as_deref() == Some(id) {
        store.active_provider_id = None;
    }
    let applied = apply_routing_if_enabled(paths, &store)?;
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store(&store, super::router_proxy::is_running()),
        applied,
    })
}

pub fn set_enabled(paths: &CodexPaths, id: &str, enabled: bool) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let provider = store
        .providers
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("newapi.errors.notFound")?;
    if enabled && (provider.api_key.is_empty() || provider.model.is_empty()) {
        return Err("newapi.errors.enableRequirements".into());
    }
    if enabled && !provider.model_supports_protocol() {
        return Err("newapi.errors.protocolRequired".into());
    }
    provider.enabled = enabled;
    provider.updated_at = now();
    let applied = apply_routing_if_enabled(paths, &store)?;
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store(&store, super::router_proxy::is_running()),
        applied,
    })
}

pub fn fetch_models(
    paths: &CodexPaths,
    provider_id: Option<&str>,
    base_url: &str,
    api_key: &str,
    protocol: ModelProtocol,
    proxy: &ApiProxyConfigPayload,
) -> Result<Vec<newapi::Model>, String> {
    let mut key = api_key.trim().to_owned();
    let mut network_direct = false;
    if let Some(id) = provider_id {
        let store = load_store(paths)?;
        if let Some(provider) = store.providers.iter().find(|p| p.id == id) {
            let same_site = newapi::base_url(&provider.base_url)
                .ok()
                .map(|u| u.to_string())
                == newapi::base_url(base_url).ok().map(|u| u.to_string());
            if same_site {
                if key.is_empty() {
                    key = provider.api_key.clone();
                }
                network_direct = provider.network_mode() == "direct";
            }
        }
    }
    if key.is_empty() {
        return Err("newapi.errors.apiKeyRequired".into());
    }
    let connection = newapi::Connection {
        base_url: newapi::base_url(base_url)?.to_string(),
        api_key: key,
        ..Default::default()
    };
    let provider_proxy =
        newapi::effective_proxy(if network_direct { "direct" } else { "proxy" }, proxy);
    match protocol {
        ModelProtocol::Responses | ModelProtocol::ChatCompletions => {
            newapi::models(&connection, &provider_proxy)
        }
        ModelProtocol::AnthropicMessages => newapi::anthropic_models(&connection, &provider_proxy),
    }
}

pub fn test_provider(
    paths: &CodexPaths,
    id: &str,
    model: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<ProviderView, String> {
    let snapshot = {
        let _guard = store_write_guard()?;
        let store = load_store_unlocked(paths)?;
        store
            .providers
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or("newapi.errors.notFound")?
    };
    let connection = newapi::Connection {
        base_url: snapshot.base_url.clone(),
        api_key: snapshot.api_key.clone(),
        ..Default::default()
    };
    if connection.api_key.is_empty() {
        return Err("newapi.errors.apiKeyRequired".into());
    }
    // 遵循该供应商自己的网络模式（系统代理 / 直连）。
    let provider_proxy = newapi::effective_proxy(snapshot.network_mode(), proxy);
    // 模型列表由选择弹窗预先拉取，这里只执行真正决定状态的模型请求。
    let test_model = model.trim();
    let result = match snapshot.protocol {
        ModelProtocol::Responses => {
            newapi::test_responses(&connection, test_model, &provider_proxy)
        }
        ModelProtocol::ChatCompletions => {
            newapi::test_chat(&connection, test_model, &provider_proxy)
        }
        ModelProtocol::AnthropicMessages => {
            newapi::test_anthropic_messages(&connection, test_model, &provider_proxy)
        }
    };
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let index = store
        .providers
        .iter()
        .position(|p| p.id == id)
        .ok_or("newapi.errors.notFound")?;
    if store.providers[index].base_url != snapshot.base_url
        || store.providers[index].api_key != snapshot.api_key
        || store.providers[index].protocol != snapshot.protocol
        || store.providers[index].network_mode() != snapshot.network_mode()
    {
        return Err("newapi.errors.changed".into());
    }
    let provider = &mut store.providers[index];
    let timestamp = now();
    match result {
        Ok(()) => {
            provider.test_status = "ok".into();
            provider.test_at = Some(timestamp);
        }
        Err(error) => {
            provider.test_status = "failed".into();
            provider.test_at = Some(timestamp);
            save_store_unlocked(paths, &store)?;
            return Err(error);
        }
    }
    save_store_unlocked(paths, &store)?;
    Ok(store.providers[index].view())
}

/// 设置供应商网络出口：proxy = 跟随全局代理，direct = 直连。
pub fn set_network_mode(paths: &CodexPaths, id: &str, mode: &str) -> Result<StateChange, String> {
    if !matches!(mode, "proxy" | "direct") {
        return Err("newapi.errors.networkMode".into());
    }
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let provider = store
        .providers
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("newapi.errors.notFound")?;
    if provider.network_mode() == mode {
        return Ok(StateChange {
            state: state_from_store(&store, super::router_proxy::is_running()),
            applied: None,
        });
    }
    provider.network_mode = mode.to_owned();
    provider.test_status = "untested".into();
    provider.test_at = None;
    provider.updated_at = now();
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store(&store, super::router_proxy::is_running()),
        applied: None,
    })
}

// 余额适配由 router_balance 隔离：官方域名走官方协议，自定义域名先通过
// 无凭据公开接口识别 NewAPI / Sub2API，再发送对应的账户访问令牌。
pub fn query_provider_balance(
    paths: &CodexPaths,
    id: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<ProviderView, String> {
    let snapshot = {
        let _guard = store_write_guard()?;
        let store = load_store_unlocked(paths)?;
        store
            .providers
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or("newapi.errors.notFound")?
    };
    let connection = newapi::Connection {
        base_url: snapshot.base_url.clone(),
        user_id: snapshot.user_id,
        access_token: snapshot.access_token.clone(),
        api_key: snapshot.api_key.clone(),
        ..Default::default()
    };
    let provider_proxy = newapi::effective_proxy(snapshot.network_mode(), proxy);
    let (balance, source) = router_balance::query(&connection, &provider_proxy)?;
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let index = store
        .providers
        .iter()
        .position(|p| p.id == id)
        .ok_or("newapi.errors.notFound")?;
    let provider = &mut store.providers[index];
    if provider.base_url != snapshot.base_url
        || provider.user_id != snapshot.user_id
        || provider.access_token != snapshot.access_token
        || provider.api_key != snapshot.api_key
    {
        return Err("newapi.errors.changed".into());
    }
    provider.balance = Some(balance);
    provider.balance_source = Some(source.into());
    save_store_unlocked(paths, &store)?;
    Ok(store.providers[index].view())
}

pub fn set_smart_routing(paths: &CodexPaths, enabled: bool) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let applied;
    if enabled {
        if effective_active_for(&store, ModelProtocol::Responses).is_none() {
            return Err("newapi.errors.noActiveProvider".into());
        }
        applied = Some(apply_routing_config(paths, &store)?);
        store.router.enabled = true;
    } else {
        applied = remove_routing_config(paths, &store)?;
        store.router.enabled = false;
    }
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store(&store, super::router_proxy::is_running()),
        applied,
    })
}

pub fn set_accountless(paths: &CodexPaths, accountless: bool) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    store.router.accountless = accountless;
    let applied = apply_routing_if_enabled(paths, &store)?;
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store(&store, super::router_proxy::is_running()),
        applied,
    })
}

fn open_config(paths: &CodexPaths) -> Result<(Option<String>, toml_edit::DocumentMut), String> {
    let original = CodexConfigRepository::new(paths)
        .read_optional_text()
        .map_err(|_| "newapi.errors.configRead".to_owned())?;
    let document = original
        .as_deref()
        .unwrap_or("")
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "newapi.errors.configParse")?;
    Ok((original, document))
}

pub fn effective_active_for(store: &Store, protocol: ModelProtocol) -> Option<&Provider> {
    store
        .active_provider_id
        .as_ref()
        .and_then(|id| {
            store
                .providers
                .iter()
                .find(|provider| &provider.id == id && provider.is_routable_for(protocol))
        })
        .or_else(|| {
            store
                .providers
                .iter()
                .find(|provider| provider.is_routable_for(protocol))
        })
}

fn config_update_error(error: CoreError) -> String {
    match error {
        CoreError::TomlParse(_) | CoreError::InvalidData(_) => "newapi.errors.configParse".into(),
        CoreError::OperationFailed(message) if message.starts_with("newapi.errors.") => message,
        CoreError::OperationFailed(_) => "newapi.errors.changed".into(),
        _ => "newapi.errors.storage".into(),
    }
}

fn applied_config(result: ConfigWriteResult) -> newapi::AppliedConfig {
    newapi::AppliedConfig {
        path: result.path.display().to_string(),
        backup_path: result.backup_path.map(|path| path.display().to_string()),
    }
}

fn remove_managed_provider_entries(document: &mut toml_edit::DocumentMut) {
    if let Some(providers) = document
        .get_mut("model_providers")
        .and_then(toml_edit::Item::as_table_mut)
    {
        let ours: Vec<String> = providers
            .iter()
            .map(|(key, _)| key.to_owned())
            .filter(|key| managed_provider_key(key))
            .collect();
        for key in ours {
            providers.remove(&key);
        }
    }
}

fn model_provider_is_ours(document: &toml_edit::DocumentMut) -> bool {
    document
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(managed_provider_key)
}

fn managed_provider_key(key: &str) -> bool {
    key == DIRECT_PROVIDER_KEY
        || key == CATALOG_PROVIDER_KEY
        || key.starts_with(PROVIDER_PREFIX)
        || key == PREVIOUS_DIRECT_PROVIDER_KEY
        || key == PREVIOUS_CATALOG_PROVIDER_KEY
        || key.starts_with(PREVIOUS_PROVIDER_PREFIX)
}

pub(crate) fn catalog_path(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join("newapi-router-catalog.json")
}

/// Codex reads relay + official models from this catalog file (`model_catalog_json`),
/// matching the original app's protocol. Official entries are borrowed read-only from
/// the original app's catalog when it is present.
pub(crate) fn write_catalog(paths: &CodexPaths, store: &Store) -> Result<PathBuf, String> {
    let document = catalog_value(&paths.app_data_dir, &paths.codex_home, store);
    let bytes =
        serde_json::to_vec_pretty(&document).map_err(|_| "newapi.errors.storage".to_owned())?;
    storage::write_atomic(&catalog_path(paths), &bytes)
        .map_err(|_| "newapi.errors.storage".to_owned())?;
    Ok(catalog_path(paths))
}

/// Pure catalog builder (no disk writes) so the local proxy can serve it per request.
pub(crate) fn catalog_value(
    app_data_dir: &std::path::Path,
    codex_home: &std::path::Path,
    store: &Store,
) -> Value {
    let mut models: Vec<Value> = Vec::new();
    let mut slugs = HashSet::new();
    // Codex refreshes this cache from the official service. It is the most accurate
    // local source for models available to the currently signed-in account.
    for source in [
        codex_home.join("models_cache.json"),
        app_data_dir.join("previous-router-catalog.json"),
    ] {
        let Ok(original) = std::fs::read(source) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&original) else {
            continue;
        };
        let Some(entries) = value.get("models").and_then(Value::as_array) else {
            continue;
        };
        for entry in entries {
            let Some(slug) = entry.get("slug").and_then(Value::as_str) else {
                continue;
            };
            let is_relay = slug.starts_with(PREVIOUS_RELAY_MODEL_PREFIX) || slug.contains("::");
            if !is_relay && slugs.insert(slug.to_owned()) {
                models.push(entry.clone());
            }
        }
    }
    for provider in store
        .providers
        .iter()
        .filter(|provider| provider.is_routable_for(ModelProtocol::Responses))
    {
        let mut provider_models: Vec<(String, Option<u64>)> = provider
            .models
            .iter()
            .filter(|model| {
                model.included && model_supports_protocol(model, ModelProtocol::Responses)
            })
            .map(|model| {
                (
                    model.id.clone(),
                    model.context_window.or(provider.context_window),
                )
            })
            .collect();
        if !provider.model.trim().is_empty()
            && !provider_models.iter().any(|(id, _)| id == &provider.model)
        {
            provider_models.push((provider.model.clone(), provider.context_window));
        }
        for (model_id, context_window) in provider_models {
            let slug = format!("{}::{model_id}", provider.id);
            if !slugs.insert(slug.clone()) {
                continue;
            }
            let metadata = provider.models.iter().find(|model| model.id == model_id);
            let display_name = metadata
                .map(|model| model.display_name.trim())
                .filter(|name| !name.is_empty())
                .unwrap_or(&model_id);
            models.push(serde_json::json!({
                "slug": slug,
                "display_name": format!("{} / {display_name}", provider.name),
                "description": format!("{} → {model_id}", provider.name),
                "context_window": context_window,
                "max_output_tokens": metadata.and_then(|model| model.max_output_tokens),
                "visibility": "list",
                "supported_in_api": true,
                "priority": 100,
            }));
        }
    }
    serde_json::json!({ "models": models })
}

pub fn apply_routing_config(
    paths: &CodexPaths,
    store: &Store,
) -> Result<newapi::AppliedConfig, String> {
    use toml_edit::value;
    if store.router.bearer_token.is_empty() {
        return Err("newapi.errors.storage".into());
    }
    let active = effective_active_for(store, ModelProtocol::Responses)
        .ok_or("newapi.errors.noActiveProvider")?;
    let catalog = write_catalog(paths, store)?;
    let result = CodexConfigRepository::new(paths)
        .update(|original| {
            let mut document = original
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| CoreError::InvalidData("Invalid Codex config TOML".into()))?;
            if document
                .get("model_providers")
                .is_some_and(|item| !item.is_table())
            {
                return Err(CoreError::InvalidData(
                    "model_providers must be a TOML table".into(),
                ));
            }
            remove_managed_provider_entries(&mut document);
            document["model"] = value(format!("{}::{}", active.id, active.model));
            document["model_provider"] = value(CATALOG_PROVIDER_KEY);
            document["model_catalog_json"] = value(catalog.to_string_lossy().to_string());
            for (key, setting) in [
                ("model_context_window", active.context_window),
                ("model_auto_compact_token_limit", active.auto_compact_limit),
            ] {
                if let Some(setting) = setting {
                    document[key] = value(setting as i64);
                } else {
                    document.remove(key);
                }
            }
            if document.get("model_providers").is_none() {
                let mut providers = toml_edit::Table::new();
                providers.set_implicit(true);
                document["model_providers"] = toml_edit::Item::Table(providers);
            }
            let entry = &mut document["model_providers"][CATALOG_PROVIDER_KEY];
            *entry = toml_edit::Item::Table(toml_edit::Table::new());
            entry["name"] = value("AgentHarbor 智能路由");
            entry["base_url"] = value(format!("http://127.0.0.1:{}/codex/v1", store.router.port));
            entry["wire_api"] = value("responses");
            entry["experimental_bearer_token"] = value(&store.router.bearer_token);
            entry["requires_openai_auth"] = value(false);
            if store.router.accountless {
                document["preferred_auth_method"] = value("apikey");
            } else {
                document.remove("preferred_auth_method");
            }
            Ok(document.to_string())
        })
        .map_err(config_update_error)?;
    Ok(applied_config(result))
}

/// Repair an enabled router configuration only when its critical fields are
/// missing or stale. This avoids creating a backup every time the page loads.
pub fn ensure_routing_config(
    paths: &CodexPaths,
    store: &Store,
) -> Result<Option<newapi::AppliedConfig>, String> {
    if !store.router.enabled {
        return Ok(None);
    }
    let active = effective_active_for(store, ModelProtocol::Responses)
        .ok_or("newapi.errors.noActiveProvider")?;
    let (_, document) = open_config(paths)?;
    let expected_base = format!("http://127.0.0.1:{}/codex/v1", store.router.port);
    let expected_model = format!("{}::{}", active.id, active.model);
    let expected_catalog = catalog_path(paths).to_string_lossy().to_string();
    let catalog_current = std::fs::read(catalog_path(paths))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|catalog| {
            catalog == catalog_value(&paths.app_data_dir, &paths.codex_home, store)
        });
    let provider = document
        .get("model_providers")
        .and_then(toml_edit::Item::as_table)
        .and_then(|providers| providers.get(CATALOG_PROVIDER_KEY))
        .and_then(toml_edit::Item::as_table);
    let current = provider.is_some_and(|provider| {
        provider.get("base_url").and_then(toml_edit::Item::as_str) == Some(expected_base.as_str())
            && provider.get("wire_api").and_then(toml_edit::Item::as_str) == Some("responses")
            && provider
                .get("experimental_bearer_token")
                .and_then(toml_edit::Item::as_str)
                == Some(store.router.bearer_token.as_str())
    }) && document
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        == Some(CATALOG_PROVIDER_KEY)
        && document.get("model").and_then(toml_edit::Item::as_str) == Some(expected_model.as_str())
        && document
            .get("model_catalog_json")
            .and_then(toml_edit::Item::as_str)
            == Some(expected_catalog.as_str())
        && catalog_current;
    if current {
        Ok(None)
    } else {
        apply_routing_config(paths, store).map(Some)
    }
}

pub fn remove_routing_config(
    paths: &CodexPaths,
    store: &Store,
) -> Result<Option<newapi::AppliedConfig>, String> {
    if !paths.config_path.exists() {
        let _ = std::fs::remove_file(catalog_path(paths));
        return Ok(None);
    }
    let result = CodexConfigRepository::new(paths)
        .update(|original| {
            let mut document = original
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| CoreError::InvalidData("Invalid Codex config TOML".into()))?;
            if document
                .get("model_providers")
                .is_some_and(|item| !item.is_table())
            {
                return Err(CoreError::InvalidData(
                    "model_providers must be a TOML table".into(),
                ));
            }
            remove_managed_provider_entries(&mut document);
            if model_provider_is_ours(&document) {
                document.remove("model_provider");
                document.remove("model");
                document.remove("model_catalog_json");
                document.remove("model_context_window");
                document.remove("model_auto_compact_token_limit");
            }
            if store.router.accountless {
                document.remove("preferred_auth_method");
            }
            Ok(document.to_string())
        })
        .map_err(config_update_error)?;
    let _ = std::fs::remove_file(catalog_path(paths));
    Ok(Some(applied_config(result)))
}

#[derive(Deserialize)]
struct ExportedProvider {
    name: String,
    #[serde(alias = "baseUrl")]
    base_url: String,
    #[serde(default)]
    user_id: Option<u64>,
    #[serde(default)]
    protocol: ModelProtocol,
    #[serde(default)]
    model: String,
    #[serde(default)]
    models: Vec<newapi::Model>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    auto_compact_limit: Option<u64>,
}

pub fn import_providers(paths: &CodexPaths, path: &Path) -> Result<ImportResult, String> {
    let bytes = std::fs::read(path).map_err(|_| "newapi.errors.importFile")?;
    if bytes.len() > 1024 * 1024 {
        return Err("newapi.errors.importFile".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.importFile")?;
    let items = value
        .get("providers")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| value.as_array().cloned())
        .ok_or("newapi.errors.importFile")?;
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let timestamp = now();
    for item in items {
        let Ok(entry) = serde_json::from_value::<ExportedProvider>(item) else {
            skipped += 1;
            continue;
        };
        let Ok(base_url) = newapi::base_url(&entry.base_url) else {
            skipped += 1;
            continue;
        };
        let base_url = base_url.to_string();
        let name = entry.name.trim().to_owned();
        let mut models = entry.models;
        if normalize_models(&mut models, entry.model.trim()).is_err() {
            skipped += 1;
            continue;
        }
        if name.is_empty()
            || store
                .providers
                .iter()
                .any(|p| p.base_url == base_url && p.name == name)
            || store.providers.len() >= MAX_PROVIDERS
        {
            skipped += 1;
            continue;
        }
        store.providers.push(Provider {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            base_url,
            user_id: entry.user_id,
            protocol: entry.protocol,
            model: entry.model,
            models,
            context_window: entry.context_window,
            auto_compact_limit: entry.auto_compact_limit,
            enabled: false,
            test_status: "untested".into(),
            created_at: timestamp,
            updated_at: timestamp,
            ..Default::default()
        });
        imported += 1;
    }
    save_store_unlocked(paths, &store)?;
    Ok(ImportResult {
        state: state_from_store(&store, super::router_proxy::is_running()),
        imported,
        skipped,
    })
}

pub fn export_providers(paths: &CodexPaths, path: &Path) -> Result<(), String> {
    let store = load_store(paths)?;
    let providers: Vec<Value> = store
        .providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "baseUrl": p.base_url,
                "userId": p.user_id,
                "protocol": p.protocol,
                "model": p.model,
                "models": p.models,
                "contextWindow": p.context_window,
                "autoCompactLimit": p.auto_compact_limit,
            })
        })
        .collect();
    let document = serde_json::json!({
        "version": 3,
        "exportedAt": now(),
        "providers": providers,
    });
    let mut text =
        serde_json::to_vec_pretty(&document).map_err(|_| "newapi.errors.storage".to_owned())?;
    text.push(b'\n');
    storage::write_atomic(path, &text).map_err(|_| "newapi.errors.storage".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        mcp,
        models::{ApiProxyConfigPayload, McpServerSummary, McpTransport},
    };
    use std::collections::HashMap;
    use std::io::{Read, Write};

    fn paths() -> (tempfile::TempDir, CodexPaths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = CodexPaths::from_home(dir.path().to_owned());
        (dir, paths)
    }

    fn input(name: &str, base_url: &str) -> ProviderInput {
        ProviderInput {
            id: None,
            name: name.into(),
            base_url: base_url.into(),
            user_id: Some(7),
            access_token: "token-a".into(),
            api_key: "sk-a".into(),
            protocol: ModelProtocol::Responses,
            model: "model-a".into(),
            models: Vec::new(),
            context_window: None,
            auto_compact_limit: None,
            network_mode: "proxy".into(),
        }
    }

    fn mark_tested(paths: &CodexPaths, id: &str) {
        let mut store = load_store(paths).unwrap();
        let provider = store
            .providers
            .iter_mut()
            .find(|provider| provider.id == id)
            .unwrap();
        provider.test_status = "ok".into();
        provider.test_at = Some(123);
        save_store_unlocked(paths, &store).unwrap();
    }

    #[test]
    fn old_model_fetch_status_is_reset_until_a_real_chat_test_runs() {
        let (_dir, paths) = paths();
        let provider = Provider {
            id: "legacy-provider".into(),
            name: "Legacy".into(),
            base_url: "https://legacy.example.com/".into(),
            test_status: "ok".into(),
            test_at: Some(123),
            models: vec![newapi::Model {
                id: "model-a".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let store = Store {
            version: 1,
            providers: vec![provider],
            ..Default::default()
        };
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&store).unwrap()).unwrap();

        let migrated = load_store(&paths).unwrap();
        assert_eq!(migrated.version, STORE_VERSION);
        assert_eq!(migrated.providers[0].test_status, "untested");
        assert_eq!(migrated.providers[0].test_at, None);
        assert_eq!(migrated.providers[0].models.len(), 1);

        // Persisting the version prevents later loads from clearing a new real
        // test result.
        mark_tested(&paths, "legacy-provider");
        assert_eq!(load_store(&paths).unwrap().providers[0].test_status, "ok");
    }

    #[test]
    fn connection_edits_and_network_mode_reset_connectivity_status() {
        let (_dir, paths) = paths();
        let state = save_provider(&paths, input("Relay A", "https://a.example.com")).unwrap();
        let id = state.state.providers[0].id.clone();

        mark_tested(&paths, &id);
        let mut rename = input("Renamed", "https://a.example.com");
        rename.id = Some(id.clone());
        rename.access_token.clear();
        rename.api_key.clear();
        let renamed = save_provider(&paths, rename).unwrap();
        assert_eq!(renamed.state.providers[0].test_status, "ok");

        mark_tested(&paths, &id);
        let mut model_edit = input("Renamed", "https://a.example.com");
        model_edit.id = Some(id.clone());
        model_edit.model = "model-b".into();
        let changed = save_provider(&paths, model_edit).unwrap();
        assert_eq!(changed.state.providers[0].test_status, "untested");
        assert_eq!(changed.state.providers[0].test_at, None);

        mark_tested(&paths, &id);
        let mut key_edit = input("Renamed", "https://a.example.com");
        key_edit.id = Some(id.clone());
        key_edit.model = "model-b".into();
        key_edit.api_key = "sk-b".into();
        let changed = save_provider(&paths, key_edit).unwrap();
        assert_eq!(changed.state.providers[0].test_status, "untested");

        mark_tested(&paths, &id);
        let mut url_edit = input("Renamed", "https://b.example.com");
        url_edit.id = Some(id.clone());
        url_edit.model = "model-b".into();
        url_edit.api_key = "sk-b".into();
        let changed = save_provider(&paths, url_edit).unwrap();
        assert_eq!(changed.state.providers[0].test_status, "untested");

        mark_tested(&paths, &id);
        let network = set_network_mode(&paths, &id, "direct").unwrap();
        assert_eq!(network.state.providers[0].network_mode, "direct");
        assert_eq!(network.state.providers[0].test_status, "untested");
        assert_eq!(network.state.providers[0].test_at, None);

        mark_tested(&paths, &id);
        assert_eq!(
            set_network_mode(&paths, &id, "invalid").unwrap_err(),
            "newapi.errors.networkMode"
        );
        assert_eq!(load_store(&paths).unwrap().providers[0].test_status, "ok");
    }

    #[test]
    fn failed_chat_request_is_persisted_as_failed_instead_of_reachable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let body = r#"{"error":{"message":"model unavailable"}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let (_dir, paths) = paths();
        let state = save_provider(&paths, input("Relay", &format!("http://{address}/"))).unwrap();
        let id = state.state.providers[0].id.clone();
        assert_eq!(
            test_provider(&paths, &id, "model-a", &ApiProxyConfigPayload::default()).unwrap_err(),
            "newapi.errors.response"
        );
        server.join().unwrap();

        let provider = &load_store(&paths).unwrap().providers[0];
        assert_eq!(provider.test_status, "failed");
        assert!(provider.test_at.is_some());
    }

    #[test]
    fn providers_roundtrip_and_migrate_from_legacy_connection() {
        let (_dir, paths) = paths();
        let state = save_provider(&paths, input("Relay A", "https://a.example.com")).unwrap();
        assert_eq!(state.state.providers.len(), 1);
        assert!(!state.state.providers[0].enabled);
        // The view carries flags only; the serialized form must not contain secrets.
        assert!(state.state.providers[0].has_api_key);

        // Editing with blank secrets keeps them bound to the same site.
        let id = state.state.providers[0].id.clone();
        let mut edit = input("Relay A", "https://a.example.com");
        edit.id = Some(id.clone());
        edit.access_token.clear();
        edit.api_key.clear();
        let state = save_provider(&paths, edit).unwrap();
        assert!(state.state.providers[0].has_api_key);
        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers[0].api_key, "sk-a");

        // A legacy single connection is promoted once into a disabled provider.
        let legacy = newapi::Connection {
            base_url: "https://old.example.com/".into(),
            user_id: Some(3),
            access_token: "legacy-token".into(),
            api_key: "sk-legacy".into(),
            model: "legacy-model".into(),
            ..Default::default()
        };
        secret_file::write(
            &paths.app_data_dir.join("newapi-connection.dat"),
            &serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(store_path(&paths)).unwrap();
        // With no provider store on disk, migration rebuilds it from the legacy entry alone.
        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers.len(), 1);
        assert_eq!(store.providers[0].base_url, "https://old.example.com/");
        assert_eq!(store.providers[0].api_key, "sk-legacy");
        assert!(!store.providers[0].enabled);
    }

    #[test]
    fn selected_model_library_survives_save_and_reload() {
        let (_dir, paths) = paths();
        let mut draft = input("Relay A", "https://a.example.com");
        draft.model = "model-fast".into();
        draft.models = vec![
            newapi::Model {
                id: "model-fast".into(),
                display_name: "Fast".into(),
                context_window: Some(200_000),
                max_output_tokens: Some(16_384),
                endpoint_types: vec!["openai-response".into()],
                included: true,
                metadata_source: "manual".into(),
            },
            newapi::Model {
                id: "model-deep".into(),
                display_name: "Deep reasoning".into(),
                context_window: Some(400_000),
                max_output_tokens: Some(128_000),
                endpoint_types: vec!["openai-response".into()],
                included: true,
                metadata_source: "upstream".into(),
            },
        ];

        let state = save_provider(&paths, draft).unwrap();
        assert_eq!(state.state.providers[0].models.len(), 2);

        let provider = &load_store(&paths).unwrap().providers[0];
        assert_eq!(provider.model, "model-fast");
        assert_eq!(provider.models.len(), 2);
        assert_eq!(provider.models[1].display_name, "Deep reasoning");
        assert_eq!(provider.models[1].context_window, Some(400_000));
        assert!(provider.models.iter().all(|model| model.included));
    }

    #[test]
    fn version_three_models_migrate_to_visible_local_metadata() {
        let (_dir, paths) = paths();
        let legacy = serde_json::json!({
            "version": 3,
            "providers": [{
                "id": "legacy-provider",
                "name": "Legacy",
                "baseUrl": "https://legacy.example.com/",
                "apiKey": "sk-legacy",
                "protocol": "responses",
                "model": "gpt-5-test",
                "models": [{
                    "id": "gpt-5-test",
                    "contextWindow": null,
                    "maxOutputTokens": null,
                    "endpointTypes": []
                }]
            }],
            "router": {}
        });
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&legacy).unwrap()).unwrap();

        let migrated = load_store(&paths).unwrap();
        assert_eq!(migrated.version, STORE_VERSION);
        let model = &migrated.providers[0].models[0];
        assert!(model.included);
        assert_eq!(model.display_name, "gpt-5-test");
        assert_eq!(model.context_window, Some(400_000));
        assert_eq!(model.max_output_tokens, Some(128_000));
        assert_eq!(model.metadata_source, "local");
    }

    #[test]
    fn previous_router_identity_moves_to_the_isolated_port_and_namespace() {
        let (_dir, paths) = paths();
        let store = Store {
            version: 1,
            providers: vec![Provider {
                id: "aimami_relay_fixture".into(),
                name: "Previous relay".into(),
                base_url: "https://relay.example.com/".into(),
                api_key: "sk-fixture".into(),
                model: "model-a".into(),
                enabled: true,
                ..Default::default()
            }],
            router: RouterSettings {
                port: PREVIOUS_DEFAULT_PORT,
                bearer_token: "aimami_previous_token".into(),
                enabled: true,
                ..Default::default()
            },
            active_provider_id: Some("aimami_relay_fixture".into()),
        };
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&store).unwrap()).unwrap();

        let migrated = load_store(&paths).unwrap();
        assert_eq!(migrated.router.port, DEFAULT_PORT);
        assert!(!migrated.router.enabled);
        assert!(migrated.router.bearer_token.starts_with("agentharbor_"));
        assert!(!migrated.providers[0].id.starts_with("aimami_"));
        assert_eq!(
            migrated.active_provider_id.as_deref(),
            Some(migrated.providers[0].id.as_str())
        );
    }

    #[test]
    fn previous_harbor_router_token_is_rotated_and_disabled() {
        let (_dir, paths) = paths();
        let store = Store {
            version: 1,
            providers: vec![Provider {
                id: "existing-provider-id".into(),
                name: "Previous harbor relay".into(),
                base_url: "https://relay.example.com/".into(),
                api_key: "sk-fixture".into(),
                model: "model-a".into(),
                enabled: true,
                ..Default::default()
            }],
            router: RouterSettings {
                port: DEFAULT_PORT,
                bearer_token: "codexharbor_previous_token".into(),
                enabled: true,
                ..Default::default()
            },
            active_provider_id: Some("existing-provider-id".into()),
        };
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&store).unwrap()).unwrap();

        let migrated = load_store(&paths).unwrap();
        assert!(!migrated.router.enabled);
        assert!(migrated.router.bearer_token.starts_with("agentharbor_"));
        assert_eq!(migrated.providers[0].id, "existing-provider-id");
    }

    #[test]
    fn concurrent_provider_writes_do_not_overwrite_each_other() {
        let (_dir, paths) = paths();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        std::thread::scope(|scope| {
            for (name, url) in [
                ("Relay A", "https://a.example.com"),
                ("Relay B", "https://b.example.com"),
            ] {
                let paths = paths.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    save_provider(&paths, input(name, url)).unwrap();
                });
            }
            barrier.wait();
        });
        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers.len(), 2);
        assert!(store
            .providers
            .iter()
            .any(|provider| provider.name == "Relay A"));
        assert!(store
            .providers
            .iter()
            .any(|provider| provider.name == "Relay B"));
    }

    #[test]
    fn router_and_mcp_config_writes_do_not_overwrite_each_other() {
        let (_dir, paths) = paths();
        std::fs::write(&paths.config_path, "theme = 'dark'\n").unwrap();
        let store = Store {
            providers: vec![Provider {
                id: "relay-a".into(),
                name: "Relay A".into(),
                base_url: "https://relay.example.com/".into(),
                api_key: "sk-test".into(),
                model: "model-a".into(),
                enabled: true,
                ..Default::default()
            }],
            router: RouterSettings {
                bearer_token: "agentharbor_test_token".into(),
                ..Default::default()
            },
            active_provider_id: Some("relay-a".into()),
            ..Default::default()
        };
        let server = McpServerSummary {
            name: "context7".into(),
            transport: McpTransport::Stdio,
            enabled: true,
            source_path: paths.config_path.display().to_string(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "@upstash/context7-mcp@latest".into()],
            url: None,
            headers: HashMap::new(),
            environment: HashMap::new(),
        };
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));

        std::thread::scope(|scope| {
            let router_paths = paths.clone();
            let router_barrier = barrier.clone();
            scope.spawn(move || {
                router_barrier.wait();
                apply_routing_config(&router_paths, &store).unwrap();
            });
            let mcp_paths = paths.clone();
            let mcp_barrier = barrier.clone();
            scope.spawn(move || {
                mcp_barrier.wait();
                mcp::upsert_mcp_server(&mcp_paths, &server).unwrap();
            });
            barrier.wait();
        });

        let config: toml::Value = std::fs::read_to_string(&paths.config_path)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(config["theme"].as_str(), Some("dark"));
        assert_eq!(
            config["model_providers"][CATALOG_PROVIDER_KEY]["wire_api"].as_str(),
            Some("responses")
        );
        assert_eq!(
            config["mcp_servers"]["context7"]["command"].as_str(),
            Some("npx")
        );
    }

    #[test]
    fn enable_requires_credentials_and_routing_needs_an_active_provider() {
        let (_dir, paths) = paths();
        let mut bare = input("Relay A", "https://a.example.com");
        bare.api_key.clear();
        bare.model.clear();
        let state = save_provider(&paths, bare).unwrap();
        let id = state.state.providers[0].id.clone();
        assert_eq!(
            set_enabled(&paths, &id, true).unwrap_err(),
            "newapi.errors.enableRequirements"
        );
        assert_eq!(
            set_smart_routing(&paths, true).unwrap_err(),
            "newapi.errors.noActiveProvider"
        );
        let mut complete = input("Relay A", "https://a.example.com");
        complete.id = Some(id.clone());
        save_provider(&paths, complete).unwrap();
        let state = set_enabled(&paths, &id, true).unwrap();
        assert!(state.state.providers[0].enabled);
        let change = set_smart_routing(&paths, true).unwrap();
        assert!(change.applied.is_some());
        assert!(change.state.router.enabled);
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        let store = load_store(&paths).unwrap();
        assert!(store.router.bearer_token.starts_with("agentharbor_"));
        assert!(store.router.bearer_token.len() > 24);
        assert!(text.contains(&store.router.bearer_token));
        assert!(text.contains("http://127.0.0.1:43127/codex/v1"));
        assert!(text.contains("::model-a"));
        assert!(text.contains("model_catalog_json"));
        let change = set_smart_routing(&paths, false).unwrap();
        assert!(!change.state.router.enabled);
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(!text.contains(CATALOG_PROVIDER_KEY));
        assert!(!text.contains("model_provider ="));
        assert!(!text.contains("model_catalog_json"));
        assert!(!catalog_path(&paths).exists());
    }

    #[test]
    fn protocol_incompatible_model_metadata_blocks_enable() {
        let (_dir, paths) = paths();
        let state = save_provider(&paths, input("Relay A", "https://a.example.com")).unwrap();
        let id = state.state.providers[0].id.clone();
        let mut store = load_store(&paths).unwrap();
        store.providers[0].models = vec![newapi::Model {
            id: "model-a".into(),
            context_window: None,
            max_output_tokens: None,
            endpoint_types: vec!["openai-chat".into()],
            ..Default::default()
        }];
        save_store_unlocked(&paths, &store).unwrap();
        assert_eq!(
            set_enabled(&paths, &id, true).unwrap_err(),
            "newapi.errors.protocolRequired"
        );
    }

    #[test]
    fn legacy_chat_routes_migrate_to_explicit_protocol() {
        let (_dir, paths) = paths();
        let provider = Provider {
            id: "legacy-chat".into(),
            name: "Legacy Chat".into(),
            base_url: "https://chat.example.com/".into(),
            api_key: "sk-chat".into(),
            model: "chat-model".into(),
            enabled: true,
            ..Default::default()
        };
        let store = Store {
            version: 1,
            providers: vec![provider],
            router: RouterSettings {
                enabled: true,
                bearer_token: "old-router-token".into(),
                ..Default::default()
            },
            active_provider_id: Some("legacy-chat".into()),
        };
        let mut value = serde_json::to_value(store).unwrap();
        value["providers"][0]["wireApi"] = serde_json::json!("chat");
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::create_dir_all(paths.config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &paths.config_path,
            format!(
                "model = 'legacy-chat::chat-model'\nmodel_provider = '{CATALOG_PROVIDER_KEY}'\n[model_providers.{CATALOG_PROVIDER_KEY}]\nwire_api = 'chat'\n"
            ),
        )
        .unwrap();

        let migrated = load_store(&paths).unwrap();
        assert!(migrated.providers[0].enabled);
        assert_eq!(
            migrated.providers[0].protocol,
            ModelProtocol::ChatCompletions
        );
        assert!(!migrated.router.enabled);
        assert_eq!(migrated.active_provider_id.as_deref(), Some("legacy-chat"));
        let config = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(!config.contains(CATALOG_PROVIDER_KEY));
        assert!(!config.contains("model_provider"));
    }

    #[test]
    fn failed_route_cleanup_does_not_commit_store_migration() {
        let (_dir, paths) = paths();
        let provider = Provider {
            id: "legacy-chat".into(),
            name: "Legacy Chat".into(),
            base_url: "https://chat.example.com/".into(),
            api_key: "sk-chat".into(),
            model: "chat-model".into(),
            enabled: true,
            ..Default::default()
        };
        let store = Store {
            version: 1,
            providers: vec![provider],
            router: RouterSettings {
                enabled: true,
                bearer_token: "old-router-token".into(),
                ..Default::default()
            },
            active_provider_id: Some("legacy-chat".into()),
        };
        let mut value = serde_json::to_value(store).unwrap();
        value["providers"][0]["wireApi"] = serde_json::json!("chat");
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::create_dir_all(paths.config_path.parent().unwrap()).unwrap();
        std::fs::write(&paths.config_path, "[invalid").unwrap();

        let error = match load_store(&paths) {
            Ok(_) => panic!("migration unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error, "newapi.errors.configParse");

        let stored: Value =
            serde_json::from_slice(&secret_file::read(&store_path(&paths)).unwrap().unwrap())
                .unwrap();
        assert_eq!(stored["version"].as_u64(), Some(1));
        assert_eq!(stored["router"]["enabled"].as_bool(), Some(true));
        assert_eq!(stored["providers"][0]["wireApi"].as_str(), Some("chat"));
    }

    #[test]
    fn routing_config_preserves_foreign_entries_and_supports_accountless() {
        let (_dir, paths) = paths();
        std::fs::write(
            &paths.config_path,
            "# keep\nmodel = 'gpt-x'\nmodel_provider = 'aimami_newapi'\n[model_providers.other]\nname = 'Other'\n[model_providers.aimami_newapi]\nname = 'legacy'\npreferred_auth_method_placeholder = true\n",
        )
        .unwrap();
        // The placeholder key above lives at top level; rewrite with a real one.
        std::fs::write(
            &paths.config_path,
            "# keep\nmodel = 'gpt-x'\nmodel_provider = 'aimami_newapi'\npreferred_auth_method = 'chatgpt'\n[model_providers.other]\nname = 'Other'\n[model_providers.aimami_newapi]\nname = 'legacy'\n[model_providers.codexharbor_relay]\nname = 'Previous Harbor'\n",
        )
        .unwrap();
        let state = save_provider(&paths, input("Relay A", "https://a.example.com")).unwrap();
        let id = state.state.providers[0].id.clone();
        set_enabled(&paths, &id, true).unwrap();
        let mut store = load_store(&paths).unwrap();
        store.router.accountless = true;
        // Catalog entries come from the cached model list (populated by 测试连通 in the UI).
        store.providers[0].models.push(newapi::Model {
            id: "model-a".into(),
            context_window: Some(128000),
            max_output_tokens: None,
            endpoint_types: vec![],
            ..Default::default()
        });
        save_store_unlocked(&paths, &store).unwrap();
        apply_routing_config(&paths, &store).unwrap();
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(text.contains("# keep"));
        let parsed: toml::Value = text.parse().unwrap();
        assert_eq!(
            parsed["model_providers"]["other"]["name"].as_str(),
            Some("Other")
        );
        assert_eq!(
            parsed["model_providers"]["aimami_newapi"]["name"].as_str(),
            Some("legacy")
        );
        assert!(text.contains("preferred_auth_method = \"apikey\""));
        assert!(text.contains(CATALOG_PROVIDER_KEY));
        assert!(!text.contains(PREVIOUS_CATALOG_PROVIDER_KEY));
        let catalog = std::fs::read_to_string(catalog_path(&paths)).unwrap();
        assert!(catalog.contains(&format!("{id}::model-a")));
        remove_routing_config(&paths, &store).unwrap();
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(!text.contains("agentharbor_p_"));
        assert!(!catalog_path(&paths).exists());
        let parsed: toml::Value = text.parse().unwrap();
        // Our routing keys are gone; foreign tables and settings survive.
        assert_eq!(
            parsed["model_providers"]["other"]["name"].as_str(),
            Some("Other")
        );
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("model_provider").is_none());
        assert!(parsed.get("preferred_auth_method").is_none());
    }

    #[test]
    fn codex_catalog_lists_every_selected_responses_model_with_local_metadata() {
        let (_dir, paths) = paths();
        let store = Store {
            version: STORE_VERSION,
            providers: vec![Provider {
                id: "relay-one".into(),
                name: "Woyao".into(),
                base_url: "https://relay.example.com/".into(),
                api_key: "sk-relay".into(),
                protocol: ModelProtocol::Responses,
                model: "model-default".into(),
                enabled: true,
                models: vec![
                    newapi::Model {
                        id: "model-default".into(),
                        display_name: "Default model".into(),
                        context_window: Some(128_000),
                        max_output_tokens: Some(8_192),
                        endpoint_types: vec!["openai-response".into()],
                        included: true,
                        metadata_source: "local".into(),
                    },
                    newapi::Model {
                        id: "model-extra".into(),
                        display_name: "Extra model".into(),
                        context_window: Some(400_000),
                        max_output_tokens: Some(32_768),
                        endpoint_types: vec!["openai-response".into()],
                        included: true,
                        metadata_source: "manual".into(),
                    },
                    newapi::Model {
                        id: "model-hidden".into(),
                        display_name: "Hidden model".into(),
                        included: false,
                        ..Default::default()
                    },
                    newapi::Model {
                        id: "chat-only".into(),
                        display_name: "Chat only".into(),
                        endpoint_types: vec!["openai-chat".into()],
                        included: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let catalog = catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "relay-one::model-default");
        assert_eq!(models[0]["display_name"], "Woyao / Default model");
        assert_eq!(models[0]["context_window"], 128_000);
        assert_eq!(models[1]["slug"], "relay-one::model-extra");
        assert_eq!(models[1]["context_window"], 400_000);
        assert_eq!(models[1]["max_output_tokens"], 32_768);
    }

    #[test]
    fn import_skips_invalid_and_duplicate_entries_and_export_hides_secrets() {
        let (_dir, paths) = paths();
        save_provider(&paths, input("Relay A", "https://a.example.com")).unwrap();
        let import_path = paths.app_data_dir.join("import.json");
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(
            &import_path,
            serde_json::json!({"providers": [
                {"name": "Relay A", "baseUrl": "https://a.example.com"},
                {"name": "Relay B", "baseUrl": "https://b.example.com", "model": "m"},
                {"name": "", "baseUrl": "https://c.example.com"},
                {"name": "Bad", "baseUrl": "not a url"}
            ]})
            .to_string(),
        )
        .unwrap();
        let result = import_providers(&paths, &import_path).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 3);
        let export_path = paths.app_data_dir.join("export.json");
        export_providers(&paths, &export_path).unwrap();
        let text = String::from_utf8(std::fs::read(&export_path).unwrap()).unwrap();
        assert!(!text.contains("sk-a"));
        assert!(!text.contains("token-a"));
        assert!(text.contains("https://b.example.com"));
        let exported: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(exported["version"], 3);
        let relay_b = exported["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["name"] == "Relay B")
            .unwrap();
        assert_eq!(relay_b["models"][0]["id"], "m");
        assert_eq!(relay_b["models"][0]["included"], true);
        assert!(relay_b.get("apiKey").is_none());
        assert!(relay_b.get("accessToken").is_none());
    }

    #[test]
    fn migrates_providers_from_the_original_app_relay_store() {
        let (_dir, paths) = paths();
        let relay = paths.codex_home.join("codexmate").join("relay");
        std::fs::create_dir_all(relay.join("keys")).unwrap();
        std::fs::write(
            relay.join("state.json"),
            serde_json::json!({
                "schemaVersion": 13,
                "providers": [
                    {"id": "aimami_relay_602abf8b50", "name": "DeepSeek 官方", "baseUrl": "https://api.deepseek.com",
                     "apiKeyStored": true, "model": "deepseek-flash", "models": [{"model": "deepseek-flash", "contextWindow": 128000}],
                     "updatedAt": 1789477026171i64, "createdAt": 1785544851181i64},
                    {"id": "aimami_relay_off", "name": "停用站", "baseUrl": "https://off.example.com",
                     "apiKeyStored": false, "model": "", "models": []}
                ],
                "activeByIde": {"codex": ["aimami_relay_602abf8b50"]},
                "proxy": {"running": true, "port": 25817},
                "codexRouterEnabled": true,
                "codexRouterNoAccountMode": false
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            relay.join("keys").join("aimami_relay_602abf8b50"),
            "sk-original-key",
        )
        .unwrap();

        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers.len(), 2);
        let deepseek = &store.providers[0];
        assert!(!deepseek.id.starts_with("aimami_"));
        assert_eq!(deepseek.api_key, "sk-original-key");
        assert_eq!(deepseek.base_url, "https://api.deepseek.com/");
        assert_eq!(deepseek.models[0].id, "deepseek-flash");
        assert_eq!(deepseek.models[0].context_window, Some(128000));
        assert!(deepseek.enabled);
        assert_eq!(deepseek.test_status, "untested");
        assert!(!store.providers[1].enabled);
        assert!(!store.router.enabled);
        assert_eq!(store.router.port, DEFAULT_PORT);
        assert_eq!(
            store.active_provider_id.as_deref(),
            Some(deepseek.id.as_str())
        );
        // The promoted store is persisted; removing the foreign files no longer matters.
        std::fs::remove_file(relay.join("state.json")).unwrap();
        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers.len(), 2);
    }

    #[test]
    fn effective_active_falls_back_to_first_enabled() {
        let mut store = Store::default();
        let provider_a = Provider {
            id: "a".into(),
            enabled: true,
            api_key: "k".into(),
            model: "m".into(),
            ..Default::default()
        };
        let mut provider_b = Provider {
            id: "b".into(),
            ..Default::default()
        };
        store.providers = vec![provider_b.clone(), provider_a.clone()];
        assert_eq!(effective_active(&store).unwrap().id, "a");
        store.active_provider_id = Some("b".into());
        assert_eq!(effective_active(&store).unwrap().id, "a");
        provider_b.enabled = true;
        provider_b.api_key = "k".into();
        provider_b.model = "m".into();
        store.providers = vec![provider_b.clone()];
        assert_eq!(effective_active(&store).unwrap().id, "b");
        let _ = ApiProxyConfigPayload::default();
    }
}
