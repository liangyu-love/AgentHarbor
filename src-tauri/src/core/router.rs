use super::{models::ApiProxyConfigPayload, newapi, secret_file, storage};
use crate::platform::paths::CodexPaths;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 25817;
pub const ROUTER_BEARER: &str = "aimami-local-router";
pub const PROVIDER_PREFIX: &str = "aimami_p";
/// Single provider entry injected into Codex, matching the original app's protocol.
pub const CATALOG_PROVIDER_KEY: &str = "aimami_relay";
/// Model-slug prefix used by the original app for relay models (`<provider id>::<model>`).
pub const ORIGINAL_MODEL_PREFIX: &str = "aimami_relay_";
const LEGACY_PROVIDER_KEY: &str = "aimami_newapi";
const MAX_PROVIDERS: usize = 64;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub user_id: Option<u64>,
    pub access_token: String,
    pub api_key: String,
    pub model: String,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
    pub enabled: bool,
    pub models: Vec<newapi::Model>,
    pub models_fetched_at: Option<i64>,
    pub balance: Option<newapi::Balance>,
    pub balance_source: Option<String>,
    pub test_status: String,
    pub test_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RouterSettings {
    pub enabled: bool,
    pub accountless: bool,
    pub port: u16,
}

impl Default for RouterSettings {
    fn default() -> Self {
        Self { enabled: false, accountless: false, port: DEFAULT_PORT }
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
    pub model: String,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
    pub enabled: bool,
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
            model: self.model.clone(),
            context_window: self.context_window,
            auto_compact_limit: self.auto_compact_limit,
            enabled: self.enabled,
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
        self.enabled && !self.api_key.is_empty() && !self.model.is_empty()
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
    pub model: String,
    pub context_window: Option<u64>,
    pub auto_compact_limit: Option<u64>,
}

fn now() -> i64 {
    Utc::now().timestamp()
}

fn store_path(paths: &CodexPaths) -> PathBuf {
    paths.codexmate_dir.join("newapi-providers.dat")
}

pub fn load_store(paths: &CodexPaths) -> Result<Store, String> {
    if let Some(bytes) = secret_file::read(&store_path(paths))? {
        return serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.storage".into());
    }
    let store = match migrate_original_relay(paths)? {
        Some(store) => store,
        None => migrate_legacy_connection(paths)?.unwrap_or_default(),
    };
    // Persist the promoted store so later reads do not depend on the foreign files.
    save_store(paths, &store)?;
    Ok(store)
}

// The original AiMaMi release keeps its relay data in `codexmate/relay/`:
// state.json holds providers/settings, keys/<provider id> holds plaintext API keys.
// Promoting it lets users of the installed app see their real relays immediately.
fn migrate_original_relay(paths: &CodexPaths) -> Result<Option<Store>, String> {
    let relay_dir = paths.codexmate_dir.join("relay");
    let Ok(text) = std::fs::read_to_string(relay_dir.join("state.json")) else { return Ok(None) };
    let Ok(value) = serde_json::from_str::<Value>(&text) else { return Ok(None) };
    let items = value.get("providers").and_then(Value::as_array).cloned().unwrap_or_default();
    let active: Vec<String> = value
        .pointer("/activeByIde/codex")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    let mut providers = Vec::new();
    for item in &items {
        let Some(id) = item.get("id").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 100) else { continue };
        let Ok(base_url) = newapi::base_url(item.get("baseUrl").and_then(Value::as_str).unwrap_or("")) else { continue };
        let name = item.get("name").and_then(Value::as_str).unwrap_or("relay").trim().to_owned();
        let api_key = if item.get("apiKeyStored").and_then(Value::as_bool) == Some(true) {
            std::fs::read_to_string(relay_dir.join("keys").join(id)).map(|k| k.trim().to_owned()).unwrap_or_default()
        } else {
            String::new()
        };
        let mut models = Vec::new();
        for entry in item.get("models").and_then(Value::as_array).into_iter().flatten() {
            let Some(model_id) = entry.get("model").and_then(Value::as_str).filter(|s| !s.trim().is_empty() && s.len() <= 200) else { continue };
            if models.iter().any(|m: &newapi::Model| m.id == model_id) { continue; }
            models.push(newapi::Model {
                id: model_id.to_owned(),
                context_window: entry.get("contextWindow").and_then(Value::as_u64).filter(|v| *v > 0),
                max_output_tokens: None,
                endpoint_types: Vec::new(),
            });
        }
        let updated = item.get("updatedAt").and_then(Value::as_i64).unwrap_or(0) / 1000;
        providers.push(Provider {
            id: id.to_owned(),
            name,
            base_url: base_url.to_string(),
            api_key,
            model: item.get("model").and_then(Value::as_str).unwrap_or("").trim().to_owned(),
            models_fetched_at: (!models.is_empty() && updated > 0).then_some(updated),
            enabled: active.iter().any(|a| a == id),
            test_status: if models.is_empty() { "untested".to_owned() } else { "ok".to_owned() },
            created_at: item.get("createdAt").and_then(Value::as_i64).unwrap_or(0) / 1000,
            updated_at: updated,
            models,
            ..Default::default()
        });
    }
    if providers.is_empty() { return Ok(None) }
    Ok(Some(Store {
        version: 1,
        providers,
        active_provider_id: active.first().cloned(),
        router: RouterSettings {
            enabled: value.get("codexRouterEnabled").and_then(Value::as_bool).unwrap_or(false),
            accountless: value.get("codexRouterNoAccountMode").and_then(Value::as_bool).unwrap_or(false),
            port: value.pointer("/proxy/port").and_then(Value::as_u64)
                .filter(|p| (1..=65535).contains(p)).map(|p| p as u16)
                .unwrap_or(DEFAULT_PORT),
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
    let bytes = match secret_file::read(&paths.codexmate_dir.join("newapi-connection.dat"))? {
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
    Ok(Some(Store { version: 1, providers: vec![provider], ..Default::default() }))
}

fn save_store(paths: &CodexPaths, store: &Store) -> Result<(), String> {
    let bytes = serde_json::to_vec(store).map_err(|_| "newapi.errors.storage")?;
    secret_file::write(&store_path(paths), &bytes)
}

pub fn state(paths: &CodexPaths, proxy_running: bool) -> Result<NewApiState, String> {
    let store = load_store(paths)?;
    Ok(NewApiState {
        providers: store.providers.iter().map(Provider::view).collect(),
        router: RouterView { enabled: store.router.enabled, accountless: store.router.accountless, port: store.router.port },
        proxy_running,
        active_provider_id: store.active_provider_id,
    })
}

/// The provider traffic is routed to: the pinned one, else the first enabled.
pub fn effective_active(store: &Store) -> Option<Provider> {
    store
        .active_provider_id
        .as_ref()
        .and_then(|id| store.providers.iter().find(|p| &p.id == id && p.is_routable()))
        .or_else(|| store.providers.iter().find(|p| p.is_routable()))
        .cloned()
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
    if input.user_id == Some(0) {
        return Err("newapi.errors.userId".into());
    }
    input.access_token = input.access_token.trim().to_owned();
    input.api_key = input.api_key.trim().to_owned();
    input.model = input.model.trim().to_owned();
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

fn apply_routing_if_enabled(paths: &CodexPaths, store: &Store) -> Result<Option<newapi::AppliedConfig>, String> {
    if store.router.enabled {
        Ok(Some(apply_routing_config(paths, store)?))
    } else {
        Ok(None)
    }
}

pub fn save_provider(paths: &CodexPaths, mut input: ProviderInput) -> Result<StateChange, String> {
    validate_input(&mut input)?;
    let mut store = load_store(paths)?;
    let timestamp = now();
    let applied;
    match input.id.as_ref().and_then(|id| store.providers.iter_mut().find(|p| &p.id == id)) {
        Some(provider) => {
            // Never silently send credentials from one site/account to another.
            let same_site = provider.base_url == input.base_url && provider.user_id == input.user_id;
            if same_site {
                if input.access_token.is_empty() {
                    input.access_token = provider.access_token.clone();
                }
                if input.api_key.is_empty() {
                    input.api_key = provider.api_key.clone();
                }
            }
            let site_changed = provider.base_url != input.base_url;
            provider.name = input.name.clone();
            provider.base_url = input.base_url.clone();
            provider.user_id = input.user_id;
            provider.access_token = input.access_token.clone();
            provider.api_key = input.api_key.clone();
            provider.model = input.model.clone();
            provider.context_window = input.context_window;
            provider.auto_compact_limit = input.auto_compact_limit;
            provider.updated_at = timestamp;
            if site_changed {
                provider.models.clear();
                provider.models_fetched_at = None;
                provider.balance = None;
                provider.balance_source = None;
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
                model: input.model.clone(),
                context_window: input.context_window,
                auto_compact_limit: input.auto_compact_limit,
                enabled: false,
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
    save_store(paths, &store)?;
    Ok(StateChange { state: state(paths, super::router_proxy::is_running())?, applied })
}

pub fn delete_provider(paths: &CodexPaths, id: &str) -> Result<StateChange, String> {
    let mut store = load_store(paths)?;
    if !store.providers.iter().any(|p| p.id == id) {
        return Err("newapi.errors.notFound".into());
    }
    store.providers.retain(|p| p.id != id);
    if store.active_provider_id.as_deref() == Some(id) {
        store.active_provider_id = None;
    }
    let applied = apply_routing_if_enabled(paths, &store)?;
    save_store(paths, &store)?;
    Ok(StateChange { state: state(paths, super::router_proxy::is_running())?, applied })
}

pub fn set_enabled(paths: &CodexPaths, id: &str, enabled: bool) -> Result<StateChange, String> {
    let mut store = load_store(paths)?;
    let provider = store.providers.iter_mut().find(|p| p.id == id).ok_or("newapi.errors.notFound")?;
    if enabled && (provider.api_key.is_empty() || provider.model.is_empty()) {
        return Err("newapi.errors.enableRequirements".into());
    }
    provider.enabled = enabled;
    provider.updated_at = now();
    let applied = apply_routing_if_enabled(paths, &store)?;
    save_store(paths, &store)?;
    Ok(StateChange { state: state(paths, super::router_proxy::is_running())?, applied })
}

pub fn set_active(paths: &CodexPaths, id: &str) -> Result<StateChange, String> {
    let mut store = load_store(paths)?;
    let provider = store.providers.iter().find(|p| p.id == id).ok_or("newapi.errors.notFound")?;
    if !provider.is_routable() {
        return Err("newapi.errors.enableRequirements".into());
    }
    store.active_provider_id = Some(id.to_owned());
    let applied = apply_routing_if_enabled(paths, &store)?;
    save_store(paths, &store)?;
    Ok(StateChange { state: state(paths, super::router_proxy::is_running())?, applied })
}

pub fn test_provider(paths: &CodexPaths, id: &str, proxy: &ApiProxyConfigPayload) -> Result<ProviderView, String> {
    let mut store = load_store(paths)?;
    let index = store.providers.iter().position(|p| p.id == id).ok_or("newapi.errors.notFound")?;
    let connection = newapi::Connection {
        base_url: store.providers[index].base_url.clone(),
        api_key: store.providers[index].api_key.clone(),
        ..Default::default()
    };
    if connection.api_key.is_empty() {
        return Err("newapi.errors.apiKeyRequired".into());
    }
    let result = newapi::models(&connection, proxy);
    let provider = &mut store.providers[index];
    let timestamp = now();
    match result {
        Ok(models) => {
            provider.models = models;
            provider.models_fetched_at = Some(timestamp);
            provider.test_status = "ok".into();
            provider.test_at = Some(timestamp);
        }
        Err(error) => {
            provider.test_status = "failed".into();
            provider.test_at = Some(timestamp);
            save_store(paths, &store)?;
            return Err(error);
        }
    }
    save_store(paths, &store)?;
    Ok(store.providers[index].view())
}

// Balance detection order: NewAPI user endpoint, OpenAI-compatible billing endpoints,
// then the DeepSeek official balance endpoint for its known host.
pub fn query_provider_balance(paths: &CodexPaths, id: &str, proxy: &ApiProxyConfigPayload) -> Result<ProviderView, String> {
    let mut store = load_store(paths)?;
    let index = store.providers.iter().position(|p| p.id == id).ok_or("newapi.errors.notFound")?;
    let provider = &store.providers[index];
    let connection = newapi::Connection {
        base_url: provider.base_url.clone(),
        user_id: provider.user_id,
        access_token: provider.access_token.clone(),
        api_key: provider.api_key.clone(),
        ..Default::default()
    };
    let detected = if connection.user_id.is_some() && !connection.access_token.is_empty() {
        newapi::balance(&connection, proxy).ok().map(|balance| (balance, "newapi"))
    } else {
        None
    }
    .or_else(|| (!connection.api_key.is_empty()).then(|| billing_balance(&connection, proxy).ok()).flatten().map(|balance| (balance, "billing")))
    .or_else(|| {
        (!connection.api_key.is_empty()).then(|| deepseek_balance(&connection, proxy).ok()).flatten().map(|balance| (balance, "deepseek"))
    });
    match detected {
        Some((balance, source)) => {
            {
                let provider = &mut store.providers[index];
                provider.balance = Some(balance);
                provider.balance_source = Some(source.into());
            }
            save_store(paths, &store)?;
            Ok(store.providers[index].view())
        }
        None => Err("newapi.errors.balanceDetect".into()),
    }
}

fn parse_first_f64(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| value.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()))
}

// OpenAI-compatible billing endpoints exposed by one-api / new-api style relays.
fn billing_balance(connection: &newapi::Connection, proxy: &ApiProxyConfigPayload) -> Result<newapi::Balance, String> {
    let client = newapi::client(proxy)?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, newapi::auth_header(&connection.api_key)?);
    let root = newapi::base_url(&connection.base_url)?;
    let fetch = |path: &str| -> Result<Value, String> {
        let url = root.join(path).map_err(|_| "newapi.errors.url".to_owned())?;
        newapi::get(&client, url, headers.clone())
    };
    let subscription = fetch("v1/dashboard/billing/subscription")
        .or_else(|_| fetch("dashboard/billing/subscription"))?;
    let hard_limit = parse_first_f64(&subscription, &["hard_limit_usd", "hard_limit", "system_hard_limit_usd"])
        .ok_or("newapi.errors.response")?;
    let usage = fetch("v1/dashboard/billing/usage").or_else(|_| fetch("dashboard/billing/usage")).ok();
    let used_cents = usage.as_ref().and_then(|value| parse_first_f64(value, &["total_usage"])).unwrap_or(0.0);
    let remaining = hard_limit - used_cents / 100.0;
    Ok(newapi::Balance {
        remaining,
        used: Some(used_cents / 100.0),
        unit: "USD".into(),
        raw_quota: remaining,
        quota_per_unit: None,
        request_count: None,
        checked_at: now(),
        conversion_available: true,
    })
}

fn deepseek_balance(connection: &newapi::Connection, proxy: &ApiProxyConfigPayload) -> Result<newapi::Balance, String> {
    let host = newapi::base_url(&connection.base_url)?.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != "api.deepseek.com" {
        return Err("newapi.errors.balanceDetect".into());
    }
    let client = newapi::client(proxy)?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, newapi::auth_header(&connection.api_key)?);
    let url = newapi::base_url(&connection.base_url)?
        .join("api/user/balance")
        .map_err(|_| "newapi.errors.url".to_owned())?;
    let value = newapi::get(&client, url, headers)?;
    let info = value
        .get("balance_infos")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or("newapi.errors.response")?;
    let total = info
        .get("total_balance")
        .and_then(Value::as_str)
        .and_then(|text| text.trim().parse::<f64>().ok())
        .or_else(|| parse_first_f64(info, &["total_balance"]))
        .ok_or("newapi.errors.response")?;
    Ok(newapi::Balance {
        remaining: total,
        used: None,
        unit: info.get("currency").and_then(Value::as_str).unwrap_or("").to_owned(),
        raw_quota: total,
        quota_per_unit: None,
        request_count: None,
        checked_at: now(),
        conversion_available: true,
    })
}

pub fn set_smart_routing(paths: &CodexPaths, enabled: bool) -> Result<StateChange, String> {
    let mut store = load_store(paths)?;
    let applied;
    if enabled {
        if effective_active(&store).is_none() {
            return Err("newapi.errors.noActiveProvider".into());
        }
        applied = Some(apply_routing_config(paths, &store)?);
        store.router.enabled = true;
    } else {
        applied = remove_routing_config(paths, &store)?;
        store.router.enabled = false;
    }
    save_store(paths, &store)?;
    Ok(StateChange { state: state(paths, super::router_proxy::is_running())?, applied })
}

pub fn set_accountless(paths: &CodexPaths, accountless: bool) -> Result<StateChange, String> {
    let mut store = load_store(paths)?;
    store.router.accountless = accountless;
    let applied = apply_routing_if_enabled(paths, &store)?;
    save_store(paths, &store)?;
    Ok(StateChange { state: state(paths, super::router_proxy::is_running())?, applied })
}

fn open_config(paths: &CodexPaths) -> Result<(Option<String>, toml_edit::DocumentMut), String> {
    let original = match std::fs::read_to_string(&paths.config_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("newapi.errors.configRead".into()),
    };
    let document = original
        .as_deref()
        .unwrap_or("")
        .parse::<toml_edit::DocumentMut>()
        .map_err(|_| "newapi.errors.configParse")?;
    Ok((original, document))
}

fn backup_and_write(paths: &CodexPaths, original: Option<String>, document: &toml_edit::DocumentMut) -> Result<newapi::AppliedConfig, String> {
    let backup = match original {
        Some(original) => {
            let path = paths.codexmate_dir.join("config-backups").join(format!("{}.toml", uuid::Uuid::new_v4()));
            storage::write_atomic(&path, original.as_bytes()).map_err(|_| "newapi.errors.storage".to_owned())?;
            Some(path.display().to_string())
        }
        None => None,
    };
    storage::write_atomic(&paths.config_path, document.to_string().as_bytes()).map_err(|_| "newapi.errors.storage")?;
    Ok(newapi::AppliedConfig { path: paths.config_path.display().to_string(), backup_path: backup })
}

fn remove_aimami_provider_entries(document: &mut toml_edit::DocumentMut) {
    if let Some(providers) = document.get_mut("model_providers").and_then(toml_edit::Item::as_table_mut) {
        let ours: Vec<String> = providers
            .iter()
            .map(|(key, _)| key.to_owned())
            .filter(|key| key == LEGACY_PROVIDER_KEY || key == CATALOG_PROVIDER_KEY || key.starts_with(PROVIDER_PREFIX))
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
        .is_some_and(|value| value == LEGACY_PROVIDER_KEY || value == CATALOG_PROVIDER_KEY || value.starts_with(PROVIDER_PREFIX))
}

pub(crate) fn catalog_path(paths: &CodexPaths) -> PathBuf {
    paths.codexmate_dir.join("newapi-router-catalog.json")
}

/// Codex reads relay + official models from this catalog file (`model_catalog_json`),
/// matching the original app's protocol. Official entries are borrowed read-only from
/// the original app's catalog when it is present.
pub(crate) fn write_catalog(paths: &CodexPaths, store: &Store) -> Result<PathBuf, String> {
    let document = catalog_value(&paths.codexmate_dir, store);
    let bytes = serde_json::to_vec_pretty(&document).map_err(|_| "newapi.errors.storage".to_owned())?;
    storage::write_atomic(&catalog_path(paths), &bytes).map_err(|_| "newapi.errors.storage".to_owned())?;
    Ok(catalog_path(paths))
}

/// Pure catalog builder (no disk writes) so the local proxy can serve it per request.
pub(crate) fn catalog_value(codexmate_dir: &std::path::Path, store: &Store) -> Value {
    let mut models: Vec<Value> = Vec::new();
    if let Ok(original) = std::fs::read(codexmate_dir.join("relay").join("codex_router_catalog.json")) {
        if let Ok(value) = serde_json::from_slice::<Value>(&original) {
            if let Some(entries) = value.get("models").and_then(Value::as_array) {
                for entry in entries {
                    let is_relay = entry.get("slug").and_then(Value::as_str)
                        .is_some_and(|slug| slug.starts_with(ORIGINAL_MODEL_PREFIX));
                    if !is_relay {
                        models.push(entry.clone());
                    }
                }
            }
        }
    }
    for provider in routable(store) {
        for model in &provider.models {
            models.push(serde_json::json!({
                "slug": format!("{}::{}", provider.id, model.id),
                "display_name": format!("{} / {}", provider.name, model.id),
                "description": format!("{} → {}", provider.name, model.id),
                "context_window": model.context_window,
                "priority": 100,
            }));
        }
    }
    serde_json::json!({ "models": models })
}

pub fn apply_routing_config(paths: &CodexPaths, store: &Store) -> Result<newapi::AppliedConfig, String> {
    use toml_edit::value;
    let active = effective_active(store).ok_or("newapi.errors.noActiveProvider")?;
    let entries = routable(store);
    if !entries.iter().any(|p| p.id == active.id) {
        return Err("newapi.errors.noActiveProvider".into());
    }
    let (original, mut document) = open_config(paths)?;
    if document.get("model_providers").is_some_and(|item| !item.is_table()) {
        return Err("newapi.errors.configParse".into());
    }
    remove_aimami_provider_entries(&mut document);
    let catalog = write_catalog(paths, store)?;
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
    let _ = entries;
    let entry = &mut document["model_providers"][CATALOG_PROVIDER_KEY];
    *entry = toml_edit::Item::Table(toml_edit::Table::new());
    entry["name"] = value("AiMaMi 智能路由");
    entry["base_url"] = value(format!("http://127.0.0.1:{}/codex/v1", store.router.port));
    entry["wire_api"] = value("responses");
    entry["experimental_bearer_token"] = value(ROUTER_BEARER);
    entry["requires_openai_auth"] = value(false);
    if store.router.accountless {
        document["preferred_auth_method"] = value("apikey");
    } else {
        document.remove("preferred_auth_method");
    }
    backup_and_write(paths, original, &document)
}

pub fn remove_routing_config(paths: &CodexPaths, store: &Store) -> Result<Option<newapi::AppliedConfig>, String> {
    let (original, mut document) = open_config(paths)?;
    let Some(original) = original else { return Ok(None) };
    if document.get("model_providers").is_some_and(|item| !item.is_table()) {
        return Err("newapi.errors.configParse".into());
    }
    remove_aimami_provider_entries(&mut document);
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
    let _ = std::fs::remove_file(catalog_path(paths));
    backup_and_write(paths, Some(original), &document).map(Some)
}

#[derive(Deserialize)]
struct ExportedProvider {
    name: String,
    #[serde(alias = "baseUrl")]
    base_url: String,
    #[serde(default)]
    user_id: Option<u64>,
    #[serde(default)]
    model: String,
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
    let mut store = load_store(paths)?;
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
        if name.is_empty()
            || store.providers.iter().any(|p| p.base_url == base_url && p.name == name)
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
            model: entry.model,
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
    save_store(paths, &store)?;
    Ok(ImportResult { state: state(paths, super::router_proxy::is_running())?, imported, skipped })
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
                "model": p.model,
                "contextWindow": p.context_window,
                "autoCompactLimit": p.auto_compact_limit,
            })
        })
        .collect();
    let document = serde_json::json!({
        "version": 1,
        "exportedAt": now(),
        "providers": providers,
    });
    let mut text = serde_json::to_vec_pretty(&document).map_err(|_| "newapi.errors.storage".to_owned())?;
    text.push(b'\n');
    storage::write_atomic(path, &text).map_err(|_| "newapi.errors.storage".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::ApiProxyConfigPayload;

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
            model: "model-a".into(),
            context_window: None,
            auto_compact_limit: None,
        }
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
            &paths.codexmate_dir.join("newapi-connection.dat"),
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
    fn enable_requires_credentials_and_routing_needs_an_active_provider() {
        let (_dir, paths) = paths();
        let mut bare = input("Relay A", "https://a.example.com");
        bare.api_key.clear();
        bare.model.clear();
        let state = save_provider(&paths, bare).unwrap();
        let id = state.state.providers[0].id.clone();
        assert_eq!(set_enabled(&paths, &id, true).unwrap_err(), "newapi.errors.enableRequirements");
        assert_eq!(set_smart_routing(&paths, true).unwrap_err(), "newapi.errors.noActiveProvider");
        let mut complete = input("Relay A", "https://a.example.com");
        complete.id = Some(id.clone());
        save_provider(&paths, complete).unwrap();
        let state = set_enabled(&paths, &id, true).unwrap();
        assert!(state.state.providers[0].enabled);
        let change = set_smart_routing(&paths, true).unwrap();
        assert!(change.applied.is_some());
        assert!(change.state.router.enabled);
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(text.contains("http://127.0.0.1:25817/codex/v1"));
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
            "# keep\nmodel = 'gpt-x'\nmodel_provider = 'aimami_newapi'\npreferred_auth_method = 'chatgpt'\n[model_providers.other]\nname = 'Other'\n[model_providers.aimami_newapi]\nname = 'legacy'\n",
        )
        .unwrap();
        let state = save_provider(&paths, input("Relay A", "https://a.example.com")).unwrap();
        let id = state.state.providers[0].id.clone();
        set_enabled(&paths, &id, true).unwrap();
        let mut store = load_store(&paths).unwrap();
        store.router.accountless = true;
        // Catalog entries come from the cached model list (populated by 测试连通 in the UI).
        store.providers[0].models.push(newapi::Model { id: "model-a".into(), context_window: Some(128000), max_output_tokens: None, endpoint_types: vec![] });
        save_store(&paths, &store).unwrap();
        apply_routing_config(&paths, &store).unwrap();
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(text.contains("# keep"));
        let parsed: toml::Value = text.parse().unwrap();
        assert_eq!(parsed["model_providers"]["other"]["name"].as_str(), Some("Other"));
        assert!(parsed.get("model_providers").and_then(|p| p.get(LEGACY_PROVIDER_KEY)).is_none());
        assert!(text.contains("preferred_auth_method = \"apikey\""));
        assert!(text.contains(CATALOG_PROVIDER_KEY));
        let catalog = std::fs::read_to_string(catalog_path(&paths)).unwrap();
        assert!(catalog.contains(&format!("{id}::model-a")));
        remove_routing_config(&paths, &store).unwrap();
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(!text.contains("aimami_p_"));
        assert!(!catalog_path(&paths).exists());
        let parsed: toml::Value = text.parse().unwrap();
        // Our routing keys are gone; foreign tables and settings survive.
        assert_eq!(parsed["model_providers"]["other"]["name"].as_str(), Some("Other"));
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("model_provider").is_none());
        assert!(parsed.get("preferred_auth_method").is_none());
    }

    #[test]
    fn import_skips_invalid_and_duplicate_entries_and_export_hides_secrets() {
        let (_dir, paths) = paths();
        save_provider(&paths, input("Relay A", "https://a.example.com")).unwrap();
        let import_path = paths.codexmate_dir.join("import.json");
        std::fs::create_dir_all(&paths.codexmate_dir).unwrap();
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
        let export_path = paths.codexmate_dir.join("export.json");
        export_providers(&paths, &export_path).unwrap();
        let text = String::from_utf8(std::fs::read(&export_path).unwrap()).unwrap();
        assert!(!text.contains("sk-a"));
        assert!(!text.contains("token-a"));
        assert!(text.contains("https://b.example.com"));
    }

    #[test]
    fn migrates_providers_from_the_original_app_relay_store() {
        let (_dir, paths) = paths();
        let relay = paths.codexmate_dir.join("relay");
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
        std::fs::write(relay.join("keys").join("aimami_relay_602abf8b50"), "sk-original-key").unwrap();

        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers.len(), 2);
        let deepseek = &store.providers[0];
        assert_eq!(deepseek.id, "aimami_relay_602abf8b50");
        assert_eq!(deepseek.api_key, "sk-original-key");
        assert_eq!(deepseek.base_url, "https://api.deepseek.com/");
        assert_eq!(deepseek.models[0].id, "deepseek-flash");
        assert_eq!(deepseek.models[0].context_window, Some(128000));
        assert!(deepseek.enabled);
        assert_eq!(deepseek.test_status, "ok");
        assert!(!store.providers[1].enabled);
        assert!(store.router.enabled);
        assert_eq!(store.router.port, DEFAULT_PORT);
        assert_eq!(store.active_provider_id.as_deref(), Some("aimami_relay_602abf8b50"));
        // The promoted store is persisted; removing the foreign files no longer matters.
        std::fs::remove_file(relay.join("state.json")).unwrap();
        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers.len(), 2);
    }

    #[test]
    fn effective_active_falls_back_to_first_enabled() {
        let mut store = Store::default();
        let provider_a = Provider { id: "a".into(), enabled: true, api_key: "k".into(), model: "m".into(), ..Default::default() };
        let mut provider_b = Provider { id: "b".into(), ..Default::default() };
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
