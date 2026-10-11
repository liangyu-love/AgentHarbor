use super::{
    codex_config::{CodexConfigRepository, ConfigWriteResult},
    model_defaults, model_metadata, model_policy,
    models::{ApiProxyConfigPayload, CoreError},
    models_dev, newapi, protocol, relay_usage, router_balance, secret_file, storage,
};
use crate::platform::paths::CodexPaths;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};
#[cfg(test)]
#[path = "router_access_mode_tests.rs"]
mod access_mode_tests;
#[path = "router_config_snapshot.rs"]
mod config_snapshot;
#[cfg(test)]
#[path = "router_model_protocol_tests.rs"]
mod model_protocol_tests;
#[path = "codex_native.rs"]
mod native_config;
#[path = "router_transition.rs"]
mod transition;
pub use transition::{
    change_codex_connection, codex_restart_pending, release_codex_connection_on_exit,
    CodexConnectionChange, CodexConnectionTarget,
};
#[path = "codex_subagents.rs"]
pub mod subagents;

pub const DEFAULT_PORT: u16 = 43127;
const PREVIOUS_DEFAULT_PORT: u16 = 25817;
pub const PROVIDER_PREFIX: &str = "agentharbor_p";
pub const CATALOG_PROVIDER_KEY: &str = "agentharbor_relay";
pub const AGGREGATE_PROVIDER_KEY: &str = "agentharbor_aggregate";
pub const NATIVE_PROVIDER_KEY: &str = "agentharbor_native";
pub(crate) const LOCAL_AUTH_HEADER: &str = "x-agentharbor-token";
const DIRECT_PROVIDER_KEY: &str = "agentharbor_newapi";
const PREVIOUS_PROVIDER_PREFIX: &str = "codexharbor_p";
const PREVIOUS_CATALOG_PROVIDER_KEY: &str = "codexharbor_relay";
const PREVIOUS_DIRECT_PROVIDER_KEY: &str = "codexharbor_newapi";
const PREVIOUS_RELAY_MODEL_PREFIX: &str = "aimami_relay_";
const MAX_PROVIDERS: usize = 64;
const MAX_MODELS_PER_PROVIDER: usize = 512;
const STORE_VERSION: u32 = 9;

// Compatibility export for existing provider, injector, and view imports.
// New components use `protocol::ModelProtocol` directly.
pub use super::protocol::ModelProtocol;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CodexAccessMode {
    #[default]
    Router,
    Native,
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
    /// Legacy fixed threshold; migrated into `compression` on load and no
    /// longer read by injection.
    pub auto_compact_limit: Option<u64>,
    /// models.dev provider IDs this relay maps to for capability matching.
    #[serde(default)]
    pub catalog_providers: Vec<String>,
    /// Legacy billing binding, kept for compatibility with saved prices.
    #[serde(default)]
    pub price_catalog_provider: Option<String>,
    #[serde(default)]
    pub price_multiplier: Option<f64>,
    /// Provider-level compression policy; `inherit` follows the global default.
    #[serde(default)]
    pub compression: model_policy::CompressionPolicy,
    pub enabled: bool,
    /// 该供应商的网络出口：proxy = 跟随全局代理设置，direct = 直连不走代理。
    #[serde(default)]
    pub network_mode: String,
    pub models: Vec<newapi::Model>,
    pub models_fetched_at: Option<i64>,
    pub balance: Option<newapi::Balance>,
    pub balance_source: Option<String>,
    pub balance_script: String,
    /// Zero disables periodic balance queries; shared cards use their owner's setting.
    pub balance_refresh_interval_minutes: u32,
    /// Cards on the same site share the first card's balance until customized.
    pub balance_independent: bool,
    pub test_status: String,
    pub test_at: Option<i64>,
    /// Set for the provider an official account source publishes (WorkBuddy,
    /// ChatGPT): its credentials come from the signed-in account at request
    /// time, so it is reachable only through the local router.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_source: Option<super::account_sources::SourceKind>,
    /// The provider's name in gateway model ids (`<slug>/<model>`). Assigned
    /// once from the name and kept through renames, so an agent's configured
    /// model keeps working; editable, unique across providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_slug: Option<String>,
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
    /// Explicit external consumers need the aggregate listener independently
    /// of a managed client injection.
    pub aggregate_external_enabled: bool,
    /// Codex aggregate target. A missing referenced target never selects a substitute.
    pub aggregate_model_id: Option<String>,
    /// Independent search service used when a Chat model calls a hosted tool.
    /// None chooses the current Command card, or the sole available search card.
    pub web_search_provider_id: Option<String>,
    pub accountless: bool,
    pub port: u16,
    pub bearer_token: String,
    /// Keys agents use to reach every model through `/gateway`. Separate from
    /// the bearer token managed client configs carry.
    pub gateway_keys: Vec<GatewayKey>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayKey {
    pub id: String,
    pub name: String,
    pub key: String,
    pub created_at: i64,
}

impl Default for RouterSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            aggregate_external_enabled: false,
            aggregate_model_id: None,
            web_search_provider_id: None,
            accountless: false,
            port: DEFAULT_PORT,
            bearer_token: String::new(),
            gateway_keys: Vec::new(),
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    pub version: u32,
    pub providers: Vec<Provider>,
    pub aggregates: Vec<super::aggregation::AggregateModel>,
    pub router: RouterSettings,
    pub codex_access_mode: CodexAccessMode,
    pub codex_subagents: Option<subagents::Settings>,
    pub active_provider_id: Option<String>,
    /// Global compression policy. `inherit` is never meaningful here and is
    /// read as the 90% product default.
    #[serde(default)]
    pub compression: model_policy::CompressionPolicy,
}

impl Store {
    pub fn global_compression(&self) -> model_policy::CompressionPolicy {
        match self.compression {
            model_policy::CompressionPolicy::Inherit => {
                model_policy::CompressionPolicy::percent(model_policy::DEFAULT_PERCENT)
            }
            ref policy => policy.clone(),
        }
    }
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
    pub web_search_capable: bool,
    pub protocol: ModelProtocol,
    pub model: String,
    pub context_window: Option<u64>,
    pub catalog_providers: Vec<String>,
    pub price_catalog_provider: Option<String>,
    pub price_multiplier: Option<f64>,
    pub compression: model_policy::CompressionPolicy,
    pub enabled: bool,
    pub network_mode: String,
    pub models: Vec<newapi::Model>,
    pub models_fetched_at: Option<i64>,
    pub balance: Option<newapi::Balance>,
    pub balance_source: Option<String>,
    pub balance_script: String,
    pub balance_shared_from: Option<String>,
    pub balance_refresh_interval_minutes: u32,
    pub test_status: String,
    pub test_at: Option<i64>,
    pub account_source: Option<super::account_sources::SourceKind>,
    pub gateway_slug: Option<String>,
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
            web_search_capable: supports_command_search(self),
            protocol: self.protocol,
            model: self.model.clone(),
            context_window: self.context_window,
            catalog_providers: self.catalog_providers.clone(),
            price_catalog_provider: self.price_catalog_provider.clone(),
            price_multiplier: self.price_multiplier,
            compression: self.compression.clone(),
            enabled: self.enabled,
            network_mode: self.network_mode().to_owned(),
            models: self.models.clone(),
            models_fetched_at: self.models_fetched_at,
            balance: self.balance.clone(),
            balance_source: self.balance_source.clone(),
            balance_script: self.balance_script.clone(),
            balance_shared_from: None,
            balance_refresh_interval_minutes: self.balance_refresh_interval_minutes,
            test_status: self.test_status.clone(),
            test_at: self.test_at,
            account_source: self.account_source,
            gateway_slug: self.gateway_slug.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    pub(crate) fn is_routable(&self) -> bool {
        self.can_route_model(&self.model)
    }

    pub(crate) fn has_route_credentials(&self) -> bool {
        self.enabled && (self.account_source.is_some() || !self.api_key.is_empty())
    }

    /// An account source's provider signs each request with its account's
    /// token, so only the local router can reach it.
    pub(crate) fn router_only(&self) -> bool {
        self.account_source.is_some()
    }

    pub(crate) fn can_route_model(&self, model: &str) -> bool {
        self.has_route_credentials() && !model.is_empty() && self.supports_model(model)
    }

    pub fn protocol_for_model(&self, model: &str) -> ModelProtocol {
        let claude_name = model.to_ascii_lowercase().contains("claude");
        self.models
            .iter()
            .find(|entry| entry.id == model)
            .and_then(|entry| {
                entry.protocol.or_else(|| {
                    claude_name
                        .then(|| super::protocol::declared_protocol(&entry.endpoint_types))
                        .flatten()
                })
            })
            .or_else(|| claude_name.then_some(ModelProtocol::AnthropicMessages))
            .unwrap_or(self.protocol)
    }

    pub fn selected_protocol(&self) -> ModelProtocol {
        self.protocol_for_model(&self.model)
    }

    pub(crate) fn is_routable_for(&self, protocol: ModelProtocol) -> bool {
        self.selected_protocol() == protocol && self.is_routable()
    }

    pub fn model_endpoint(&self) -> ModelEndpoint<'_> {
        ModelEndpoint {
            base_url: &self.base_url,
            protocol: self.selected_protocol(),
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
        self.supports_model_protocol(model_id, self.protocol_for_model(model_id))
    }

    pub(crate) fn supports_model_protocol(&self, model_id: &str, protocol: ModelProtocol) -> bool {
        match self.models.iter().find(|model| model.id == model_id) {
            None => true,
            Some(model) => model_supports_protocol(model, protocol),
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
    pub aggregate_model_id: Option<String>,
    pub web_search_provider_id: Option<String>,
    pub access_mode: CodexAccessMode,
    pub accountless: bool,
    pub port: u16,
    /// Whether Codex's config still points at our router.
    ///
    /// Another tool writing the same `config.toml` silently wins, which leaves
    /// routing "enabled" here while Codex keeps using its old provider. The UI
    /// reports this instead of letting the toggle look effective.
    pub config_drifted: bool,
    /// Codex is running with a connection written before its last start.
    pub restart_pending: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewApiState {
    pub providers: Vec<ProviderView>,
    pub router: RouterView,
    pub proxy_running: bool,
    pub active_provider_id: Option<String>,
    /// Global compression policy (default 90%).
    pub compression: model_policy::CompressionPolicy,
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
    /// Capability catalog mapping; omitted keeps the stored mapping.
    #[serde(default)]
    pub catalog_providers: Option<Vec<String>>,
    #[serde(default)]
    pub price_catalog_provider: Option<String>,
    /// Omitted preserves a stored multiplier; 1 restores unchanged rates.
    #[serde(default)]
    pub price_multiplier: Option<f64>,
    /// Compression policy; omitted migrates the legacy fixed threshold.
    #[serde(default)]
    pub compression: Option<model_policy::CompressionPolicy>,
    /// 可选：proxy / direct；编辑时留空表示保持原设置。
    #[serde(default)]
    pub network_mode: String,
    /// Gateway short name; empty keeps the stored one (or derives one).
    #[serde(default)]
    pub gateway_slug: Option<String>,
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
        super::aggregation::config::validate(&store)?;
        let needs_metadata_backup = store.version < 8;
        let needs_aggregation_backup = store.version < 9;
        if store.version > STORE_VERSION {
            return Err("newapi.errors.storage".into());
        }
        let mut changed = false;
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
        // Preserve provenance only when identity migration can change it.
        // Ordinary store reads should not clone every cached model.
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
        if store.version < 6 {
            for provider in &mut store.providers {
                for model in &mut provider.models {
                    // Effective values are re-derived at read time; this
                    // migration only restored identity and a visible name.
                    if model.metadata_source != "manual" {
                        model.display_name = model.id.clone();
                    }
                }
                if !provider.model.trim().is_empty()
                    && !provider
                        .models
                        .iter()
                        .any(|model| model.id == provider.model)
                {
                    provider.models.push(newapi::Model {
                        id: provider.model.clone(),
                        display_name: provider.model.clone(),
                        included: true,
                        metadata_source: "local".into(),
                        ..Default::default()
                    });
                }
            }
            changed = true;
        }
        if store.version < 7 {
            // Existing per-card account credentials/scripts were intentional.
            // Only cards without their own balance setup adopt the first card.
            let mut seen_sites = HashSet::new();
            for provider in &mut store.providers {
                if !seen_sites.insert(provider.base_url.clone())
                    && (provider.user_id.is_some()
                        || !provider.access_token.is_empty()
                        || !provider.balance_script.is_empty())
                {
                    provider.balance_independent = true;
                }
            }
            changed = true;
        }
        // One-time migration into explicit per-field storage. Capability values
        // are derived at read time from user overrides, real upstream
        // declarations and the models.dev catalog; auto-filled legacy values
        // must not survive as if they were saved by the user.
        if store.version < 8 {
            for provider in &mut store.providers {
                if provider.compression == model_policy::CompressionPolicy::Inherit {
                    provider.compression =
                        model_policy::migrate_legacy_limit(provider.auto_compact_limit);
                }
                for model in &mut provider.models {
                    if model.metadata_source == "manual" && model.overrides.is_empty() {
                        model.overrides = newapi::ModelOverrides {
                            context_window: model.context_window,
                            max_output_tokens: model.max_output_tokens,
                            reasoning: model.supported_reasoning_levels.clone(),
                            default_effort: model.default_reasoning_level.clone(),
                            ..Default::default()
                        };
                    }
                    match model.metadata_source.as_str() {
                        // Historical "upstream" values may have been overwritten
                        // by the removed local defaults; the next fetch
                        // re-collects real declarations.
                        "upstream" => model.metadata_source = "legacy".into(),
                        "local" | "catalog" => {
                            model.context_window = None;
                            model.max_output_tokens = None;
                        }
                        _ => {}
                    }
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
            changed = true;
        }
        for provider in &mut store.providers {
            match provider.legacy_wire_api.as_str() {
                "responses" => {}
                "chat" => {
                    provider.protocol = ModelProtocol::ChatCompletions;
                    provider.legacy_wire_api = default_wire_api();
                    if store.router.enabled {
                        // The old Codex route used an incompatible wire API.
                        // Keep the imported provider, but require the user to
                        // enable the new Responses bridge explicitly.
                        store.router.enabled = false;
                    }
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
        if super::gateway::assign_slugs(&mut store) {
            changed = true;
        }
        if store.router.enabled && !codex_target_available(&store)
        {
            store.router.enabled = false;
            changed = true;
        }
        if changed {
            if needs_aggregation_backup {
                let backup = paths.app_data_dir.join("newapi-providers.pre-v9.dat");
                match secret_file::read(&backup)? {
                    None => secret_file::write(&backup, &bytes)?,
                    Some(original) => {
                        serde_json::from_slice::<Store>(&original)
                            .map_err(|_| "newapi.errors.storage".to_owned())?;
                    }
                }
            }
            if needs_metadata_backup {
                let backup = paths.app_data_dir.join("newapi-providers.pre-v8.dat");
                match secret_file::read(&backup)? {
                    None => secret_file::write(&backup, &bytes)?,
                    Some(original) => {
                        // Keep the first valid backup; an unreadable/corrupt
                        // backup must not permit a destructive migration.
                        serde_json::from_slice::<Store>(&original)
                            .map_err(|_| "newapi.errors.storage".to_owned())?;
                    }
                }
            }
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

/// Mutate provider and aggregate references under the existing store write lock.
/// Validation failure never writes the proposed state, and the protected atomic
/// file write must succeed before callers publish any runtime state.
pub(crate) fn mutate_store(
    paths: &CodexPaths,
    mutate: impl FnOnce(&mut Store) -> Result<(), String>,
) -> Result<Store, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    mutate(&mut store)?;
    super::gateway::assign_slugs(&mut store);
    super::aggregation::config::validate(&store)?;
    save_store_unlocked(paths, &store)?;
    Ok(store)
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
        codex_access_mode: CodexAccessMode::Router,
        codex_subagents: None,
        providers,
        aggregates: vec![],
        active_provider_id,
        compression: model_policy::CompressionPolicy::percent(model_policy::DEFAULT_PERCENT),
        router: RouterSettings {
            enabled: false,
            aggregate_external_enabled: false,
            aggregate_model_id: None,
            web_search_provider_id: None,
            accountless: value
                .get("codexRouterNoAccountMode")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            port: DEFAULT_PORT,
            bearer_token: String::new(),
            gateway_keys: Vec::new(),
        },
    }))
}

/// Read a store from an explicit path (used by the local proxy per request).
pub fn read_store(path: &Path) -> Result<Store, String> {
    match secret_file::read(path)? {
        Some(bytes) => {
            let mut store: Store =
                serde_json::from_slice(&bytes).map_err(|_| "newapi.errors.storage")?;
            super::aggregation::config::validate(&store)?;
            // Same assignment the next load persists; deterministic for one store.
            super::gateway::assign_slugs(&mut store);
            Ok(store)
        }
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
        codex_access_mode: CodexAccessMode::Router,
        providers: vec![provider],
        ..Default::default()
    }))
}

fn save_store_unlocked(paths: &CodexPaths, store: &Store) -> Result<(), String> {
    let bytes = serde_json::to_vec(store).map_err(|_| "newapi.errors.storage")?;
    secret_file::write(&store_path(paths), &bytes)
}

/// Builds the UI state, verifying the Codex config is still ours.
///
/// Drift is only meaningful while routing is enabled and a path is known: with
/// routing off, the absence of our keys is the intended state.
pub(crate) fn state_from_store_at(
    store: &Store,
    proxy_running: bool,
    paths: Option<&CodexPaths>,
) -> NewApiState {
    NewApiState {
        providers: store
            .providers
            .iter()
            .map(|provider| balance_view(store, provider))
            .collect(),
        router: RouterView {
            enabled: store.router.enabled,
            aggregate_model_id: store.router.aggregate_model_id.clone(),
            web_search_provider_id: store.router.web_search_provider_id.clone(),
            access_mode: store.codex_access_mode,
            accountless: store.router.accountless,
            port: store.router.port,
            config_drifted: store.router.enabled
                && paths.is_some_and(|paths| match store.codex_access_mode {
                    CodexAccessMode::Router => !routing_config_intact(paths),
                    CodexAccessMode::Native => {
                        !native_config::intact(paths, store).unwrap_or(false)
                    }
                }),
            restart_pending: paths.is_some() && codex_restart_pending(),
        },
        proxy_running,
        active_provider_id: (match store.codex_access_mode {
            CodexAccessMode::Router => effective_active_for(store, ModelProtocol::Responses),
            CodexAccessMode::Native => effective_active(store),
        })
        .map(|provider| provider.id.clone()),
        compression: store.global_compression(),
    }
}

fn balance_owner<'a>(store: &'a Store, provider: &'a Provider) -> &'a Provider {
    if provider.balance_independent {
        return provider;
    }
    store
        .providers
        .iter()
        .find(|candidate| candidate.base_url == provider.base_url)
        .unwrap_or(provider)
}

fn balance_view(store: &Store, provider: &Provider) -> ProviderView {
    let owner = balance_owner(store, provider);
    let mut view = provider.view();
    if owner.id != provider.id {
        view.user_id = owner.user_id;
        view.has_access_token = !owner.access_token.is_empty();
        view.balance = owner.balance.clone();
        view.balance_source = owner.balance_source.clone();
        view.balance_script = owner.balance_script.clone();
        view.balance_shared_from = Some(owner.id.clone());
        view.balance_refresh_interval_minutes = owner.balance_refresh_interval_minutes;
    }
    view
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
                .find(|p| &p.id == id && p.is_routable() && !p.router_only())
        })
        .or_else(|| store.providers.iter().find(|p| p.is_routable() && !p.router_only()))
}

/// Validate the user's selected model before any config or store write. Native
/// Codex cannot translate a non-Responses model or silently pick another relay.
pub(crate) fn validate_native_selection(store: &Store) -> Result<(), String> {
    if store.codex_access_mode != CodexAccessMode::Native {
        return Ok(());
    }
    let selected = match store.active_provider_id.as_deref() {
        Some(id) => store.providers.iter().find(|provider| provider.id == id),
        None => store
            .providers
            .iter()
            .find(|provider| provider.has_route_credentials() && !provider.router_only() && !provider.model.is_empty()),
    };
    if selected.is_some_and(Provider::router_only) {
        return Err("accountSources.errors.routerOnly".into());
    }
    if selected.is_some_and(|provider| provider.selected_protocol() != ModelProtocol::Responses) {
        return Err("newapi.errors.nativeResponsesRequired".into());
    }
    Ok(())
}

fn validate_input(input: &mut ProviderInput) -> Result<(), String> {
    if input
        .price_multiplier
        .is_some_and(|factor| !model_metadata::valid_price_multiplier(factor))
    {
        return Err("newapi.errors.priceMultiplier".into());
    }
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
    if input
        .compression
        .as_ref()
        .is_some_and(|policy| policy.validate().is_err())
    {
        return Err("newapi.errors.context".into());
    }
    if input
        .catalog_providers
        .as_ref()
        .is_some_and(|providers| providers.len() > 8)
    {
        return Err("newapi.errors.context".into());
    }
    if let Some(providers) = input.catalog_providers.as_mut() {
        for provider in providers.iter_mut() {
            *provider = provider.trim().to_lowercase();
        }
        providers.retain(|provider| !provider.is_empty() && provider.len() <= 64);
        providers.dedup();
    }
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

/// Trim and bound a `{provider, model}` catalog reference; invalid input is
/// dropped rather than failing the whole save (the reference is advisory).
fn sanitized_ref(reference: Option<models_dev::ModelRef>) -> Option<models_dev::ModelRef> {
    let reference = reference?;
    let provider_id = reference.provider_id.trim().to_lowercase();
    let model_id = reference.model_id.trim().to_owned();
    let valid = |value: &str, max: usize| {
        !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
    };
    (valid(&provider_id, 64) && valid(&model_id, 256)).then_some(models_dev::ModelRef {
        provider_id,
        model_id,
    })
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
        // Legacy manual values are promoted only by the versioned store
        // migration. An empty override here means the user cleared it.
        let overrides = &model.overrides;
        if overrides
            .context_window
            .is_some_and(|value| !(1_024..=100_000_000).contains(&value))
            || overrides
                .input_limit
                .is_some_and(|value| !(1_024..=100_000_000).contains(&value))
            || overrides
                .max_output_tokens
                .is_some_and(|value| value == 0 || value > 100_000_000)
            || matches!(
                (overrides.context_window, overrides.max_output_tokens),
                (Some(context), Some(output)) if output > context
            )
        {
            return Err("newapi.errors.context".into());
        }
        if overrides.reasoning.as_ref().is_some_and(|levels| {
            levels.len() > 32 || !levels.iter().all(newapi::ReasoningLevel::valid)
        }) {
            return Err("newapi.errors.modelList".into());
        }
        if overrides.price.as_ref().is_some_and(|price| {
            [
                price.input,
                price.output,
                price.cache_read,
                price.cache_write,
            ]
            .iter()
            .any(|rate| {
                rate.is_some_and(|rate| !rate.is_finite() || rate < 0.0 || rate > 1_000_000.0)
            })
        }) {
            return Err("newapi.errors.context".into());
        }
        if matches!((model.context_window, model.max_output_tokens), (Some(context), Some(output)) if output > context)
        {
            return Err("newapi.errors.context".into());
        }
        if !matches!(
            model.metadata_source.as_str(),
            "upstream" | "local" | "catalog" | "manual" | "legacy" | ""
        ) {
            model.metadata_source = "manual".into();
        }
        // Catalog bindings are advisory references; impossible values are
        // dropped instead of failing the whole save.
        model.catalog_ref = sanitized_ref(model.catalog_ref.take());
        model.price_ref = sanitized_ref(model.price_ref.take());
        if model.compression.validate().is_err() {
            return Err("newapi.errors.context".into());
        }
    }
    if !default_model.is_empty() {
        if let Some(model) = models.iter_mut().find(|model| model.id == default_model) {
            model.included = true;
        } else {
            if models.len() >= MAX_MODELS_PER_PROVIDER {
                return Err("newapi.errors.tooManyModels".into());
            }
            models.push(newapi::Model {
                id: default_model.to_owned(),
                display_name: default_model.to_owned(),
                included: true,
                metadata_source: "local".into(),
                ..Default::default()
            });
        }
    }
    Ok(())
}

fn apply_routing_if_enabled(
    paths: &CodexPaths,
    store: &Store,
) -> Result<Option<newapi::AppliedConfig>, String> {
    // Provider edits may refresh an active injection, but must not reclaim
    // config.toml after another tool has selected its own provider.
    if !store.router.enabled || !config_snapshot::path(paths).is_file() {
        return Ok(None);
    }
    let (_, document) = open_config(paths)?;
    if !route_is_ours(&document) {
        return Ok(None);
    }
    Ok(Some(apply_routing_config(paths, store)?))
}

pub(crate) fn active_routing_config_is_ours(paths: &CodexPaths) -> bool {
    if !config_snapshot::path(paths).is_file() {
        return false;
    }
    open_config(paths).is_ok_and(|(_, document)| route_is_ours(&document))
}

pub fn save_provider(paths: &CodexPaths, mut input: ProviderInput) -> Result<StateChange, String> {
    validate_input(&mut input)?;
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    // Checked before anything else changes, so a refused name writes nothing.
    super::gateway::check_slug(&store, input.id.as_deref(), input.gateway_slug.as_deref())?;
    let timestamp = now();
    let previous = store.clone();
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
            if let Some(providers) = input.catalog_providers.clone() {
                provider.catalog_providers = providers;
            }
            provider.price_catalog_provider = input.price_catalog_provider.clone();
            if let Some(factor) = input.price_multiplier {
                provider.price_multiplier = (factor != 1.0).then_some(factor);
            }
            provider.compression = match input.compression.clone() {
                Some(policy) => {
                    // An explicit policy is authoritative; drop the legacy
                    // fixed value so `inherit` cannot resurrect it.
                    provider.auto_compact_limit = None;
                    policy
                }
                None => model_policy::migrate_legacy_limit(input.auto_compact_limit),
            };
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
            validate_native_selection(&store)?;
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
                catalog_providers: input.catalog_providers.clone().unwrap_or_default(),
                price_catalog_provider: input.price_catalog_provider.clone(),
                price_multiplier: input.price_multiplier.filter(|factor| *factor != 1.0),
                compression: match input.compression.clone() {
                    Some(policy) => policy,
                    None => model_policy::migrate_legacy_limit(input.auto_compact_limit),
                },
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
            validate_native_selection(&store)?;
            applied = apply_routing_if_enabled(paths, &store)?;
        }
    }
    let saved_id = input.id.clone().or_else(|| store.providers.last().map(|provider| provider.id.clone()));
    if let Some(id) = saved_id {
        super::gateway::set_slug(&mut store, &id, input.gateway_slug.as_deref())?;
    }
    super::gateway::assign_slugs(&mut store);
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

/// A single explicit value to clear. This is shared by the read-only preview
/// and the guarded mutation, so they cannot disagree about what was removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClearCapabilityField {
    ContextWindow,
    InputLimit,
    OutputLimit,
    Reasoning,
    DefaultEffort,
    Price,
    CatalogRef,
    PriceRef,
    Compression,
    ProviderContextWindow,
    ProviderCatalogProviders,
    ProviderPriceCatalogProvider,
    ProviderCompression,
}

/// Stored settings, including provider fallbacks currently shadowed by a model.
/// Resolved provenance alone cannot tell which values are removable.
pub(crate) fn manual_capability_fields(
    provider: &Provider,
    model_id: &str,
) -> Vec<ClearCapabilityField> {
    use ClearCapabilityField::*;
    let mut fields = Vec::new();
    if let Some(model) = provider.models.iter().find(|model| model.id == model_id) {
        let overrides = &model.overrides;
        for (field, present) in [
            (ContextWindow, overrides.context_window.is_some()),
            (InputLimit, overrides.input_limit.is_some()),
            (OutputLimit, overrides.max_output_tokens.is_some()),
            (Reasoning, overrides.reasoning.is_some()),
            (DefaultEffort, overrides.default_effort.is_some()),
            (Price, overrides.price.is_some()),
            (CatalogRef, model.catalog_ref.is_some()),
            (PriceRef, model.price_ref.is_some()),
            (
                Compression,
                model.compression != model_policy::CompressionPolicy::Inherit,
            ),
        ] {
            if present {
                fields.push(field);
            }
        }
    }
    for (field, present) in [
        (ProviderContextWindow, provider.context_window.is_some()),
        (
            ProviderCatalogProviders,
            !provider.catalog_providers.is_empty(),
        ),
        (
            ProviderPriceCatalogProvider,
            provider.price_catalog_provider.is_some(),
        ),
        (
            ProviderCompression,
            provider.compression != model_policy::CompressionPolicy::Inherit
                || provider.auto_compact_limit.is_some(),
        ),
    ] {
        if present {
            fields.push(field);
        }
    }
    fields
}

pub(crate) fn clear_capability_on_store(
    store: &mut Store,
    provider_id: &str,
    model_id: &str,
    field: ClearCapabilityField,
) -> Result<(), String> {
    let provider = store
        .providers
        .iter_mut()
        .find(|p| p.id == provider_id)
        .ok_or("newapi.errors.notFound")?;
    match field {
        ClearCapabilityField::ProviderContextWindow => provider.context_window = None,
        ClearCapabilityField::ProviderCatalogProviders => provider.catalog_providers.clear(),
        ClearCapabilityField::ProviderPriceCatalogProvider => {
            provider.price_catalog_provider = None
        }
        ClearCapabilityField::ProviderCompression => {
            provider.compression = model_policy::CompressionPolicy::Inherit;
            provider.auto_compact_limit = None;
        }
        _ => {
            let model = provider
                .models
                .iter_mut()
                .find(|m| m.id == model_id)
                .ok_or("newapi.errors.notFound")?;
            match field {
                ClearCapabilityField::ContextWindow => model.overrides.context_window = None,
                ClearCapabilityField::InputLimit => model.overrides.input_limit = None,
                ClearCapabilityField::OutputLimit => model.overrides.max_output_tokens = None,
                ClearCapabilityField::Reasoning => model.overrides.reasoning = None,
                ClearCapabilityField::DefaultEffort => model.overrides.default_effort = None,
                ClearCapabilityField::Price => model.overrides.price = None,
                ClearCapabilityField::CatalogRef => model.catalog_ref = None,
                ClearCapabilityField::PriceRef => model.price_ref = None,
                ClearCapabilityField::Compression => {
                    model.compression = model_policy::CompressionPolicy::Inherit
                }
                _ => unreachable!(),
            }
        }
    }
    Ok(())
}

pub(crate) fn config_revision(store: &Store) -> Result<String, String> {
    let bytes = serde_json::to_vec(store).map_err(|_| "newapi.errors.storage")?;
    Ok(super::request_diagnostics::state_revision(&bytes))
}

pub fn clear_capability_if_current(
    paths: &CodexPaths,
    provider_id: &str,
    model_id: &str,
    field: ClearCapabilityField,
    expected_config: &str,
    expected_catalog: &str,
    expected_official: &str,
) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let catalog = super::models_dev::load(&paths.app_data_dir);
    let official = super::model_catalog::official(&paths.codex_home, &paths.app_data_dir);
    let official_bytes =
        serde_json::to_vec(&official.models).map_err(|_| "newapi.errors.storage")?;
    if config_revision(&store)? != expected_config
        || catalog.snapshot.revision != expected_catalog
        || super::request_diagnostics::state_revision(&official_bytes) != expected_official
    {
        return Err("newapi.errors.previewStale".into());
    }
    let previous = store.clone();
    clear_capability_on_store(&mut store, provider_id, model_id, field)?;
    if config_revision(&store)? == expected_config {
        return Ok(StateChange {
            state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
            applied: None,
        });
    }
    if let Some(provider) = store.providers.iter_mut().find(|p| p.id == provider_id) {
        provider.updated_at = now();
    }
    validate_native_selection(&store)?;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

pub fn delete_provider(paths: &CodexPaths, id: &str) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let previous = store.clone();
    if !store.providers.iter().any(|p| p.id == id) {
        return Err("newapi.errors.notFound".into());
    }
    store.providers.retain(|p| p.id != id);
    if store.active_provider_id.as_deref() == Some(id) {
        store.active_provider_id = None;
    }
    validate_native_selection(&store)?;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

/// Relay editing, testing and balance commands never act on an account
/// source's provider: the Accounts page owns it.
pub fn ensure_relay(paths: &CodexPaths, id: &str) -> Result<(), String> {
    if load_store(paths)?.providers.iter().any(|provider| provider.id == id && provider.router_only()) {
        return Err("accountSources.errors.managedByAccounts".into());
    }
    Ok(())
}

/// Publish or refresh the provider an account source serves through the
/// router. Its models follow the source; per-model user settings survive.
pub fn upsert_account_provider(paths: &CodexPaths, mut next: Provider) -> Result<StateChange, String> {
    if next.account_source.is_none() {
        return Err("newapi.errors.notFound".into());
    }
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let previous = store.clone();
    let timestamp = now();
    next.models_fetched_at = Some(timestamp);
    next.updated_at = timestamp;
    match store.providers.iter_mut().find(|provider| provider.id == next.id) {
        Some(provider) => {
            if provider.account_source != next.account_source {
                return Err("newapi.errors.notFound".into());
            }
            for model in &mut next.models {
                if let Some(kept) = provider.models.iter().find(|kept| kept.id == model.id) {
                    model.included = kept.included;
                    model.overrides = kept.overrides.clone();
                    model.catalog_ref = kept.catalog_ref.clone();
                    model.price_ref = kept.price_ref.clone();
                    model.compression = kept.compression.clone();
                }
            }
            if next.models.iter().any(|model| model.id == provider.model) {
                next.model = provider.model.clone();
            }
            provider.name = next.name;
            provider.base_url = next.base_url;
            provider.protocol = next.protocol;
            provider.model = next.model;
            provider.models = next.models;
            provider.models_fetched_at = next.models_fetched_at;
            provider.enabled = true;
            provider.updated_at = timestamp;
        }
        None => {
            if store.providers.len() >= MAX_PROVIDERS {
                return Err("newapi.errors.tooMany".into());
            }
            next.created_at = timestamp;
            store.providers.push(next);
        }
    }
    super::gateway::assign_slugs(&mut store);
    validate_native_selection(&store)?;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

pub fn set_enabled(paths: &CodexPaths, id: &str, enabled: bool) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let previous = store.clone();
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
    if enabled
        && store.codex_access_mode == CodexAccessMode::Native
        && !store.providers.iter().any(|provider| {
            Some(provider.id.as_str()) == store.active_provider_id.as_deref()
                && provider.is_routable()
        })
    {
        store.active_provider_id = Some(id.to_owned());
    }
    validate_native_selection(&store)?;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
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
    let mut saved_protocols = HashMap::new();
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
                saved_protocols = provider
                    .models
                    .iter()
                    .filter_map(|model| model.protocol.map(|protocol| (model.id.clone(), protocol)))
                    .collect();
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
    let models = match protocol {
        ModelProtocol::Responses | ModelProtocol::ChatCompletions => {
            newapi::models(&connection, &provider_proxy)
        }
        ModelProtocol::AnthropicMessages => newapi::anthropic_models(&connection, &provider_proxy),
    }?;
    Ok(models
        .into_iter()
        .map(|mut model| {
            if let Some(protocol) = saved_protocols.get(&model.id) {
                model.protocol = Some(*protocol);
            }
            model
        })
        .collect())
}

pub fn test_provider(
    paths: &CodexPaths,
    id: &str,
    model: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<ProviderView, String> {
    test_provider_with_progress(paths, id, model, proxy, None)
}

pub fn test_provider_with_progress(
    paths: &CodexPaths,
    id: &str,
    model: &str,
    proxy: &ApiProxyConfigPayload,
    on_event: Option<&dyn Fn(super::provider_probe::ProbeEvent)>,
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
    let protocol = snapshot.protocol_for_model(test_model);
    let result = if let Some(emit) = on_event {
        super::provider_probe::run(&connection, test_model, protocol, &provider_proxy, emit)
    } else {
        match protocol {
            ModelProtocol::Responses => {
                newapi::test_responses(&connection, test_model, &provider_proxy)
            }
            ModelProtocol::ChatCompletions => {
                newapi::test_chat(&connection, test_model, &provider_proxy)
            }
            ModelProtocol::AnthropicMessages => {
                newapi::test_anthropic_messages(&connection, test_model, &provider_proxy)
            }
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
        || store.providers[index].protocol_for_model(test_model)
            != snapshot.protocol_for_model(test_model)
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
            state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
            applied: None,
        });
    }
    provider.network_mode = mode.to_owned();
    provider.test_status = "untested".into();
    provider.test_at = None;
    provider.updated_at = now();
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied: None,
    })
}

// 余额适配由 router_balance 隔离：官方域名走官方协议，自定义域名先通过
// 无凭据公开接口识别 NewAPI / Sub2API，再发送对应的账户访问令牌。
pub fn set_balance_credentials(
    paths: &CodexPaths,
    id: &str,
    expected_base_url: &str,
    user_id: Option<u64>,
    access_token: &str,
) -> Result<ProviderView, String> {
    if user_id == Some(0) {
        return Err("newapi.errors.userId".into());
    }
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let owner = store
        .providers
        .iter()
        .find(|provider| provider.id == id)
        .ok_or("newapi.errors.notFound")?;
    if owner.base_url != expected_base_url {
        return Err("newapi.errors.changed".into());
    }
    let owner = balance_owner(&store, owner).clone();
    let provider = store.providers.iter_mut().find(|p| p.id == id).unwrap();
    if !provider.balance_independent && owner.id != provider.id {
        provider.balance_script = owner.balance_script;
        provider.access_token = owner.access_token;
        provider.balance_refresh_interval_minutes = owner.balance_refresh_interval_minutes;
    }
    provider.balance_independent = true;
    provider.user_id = user_id;
    if !access_token.trim().is_empty() || provider.access_token.is_empty() {
        provider.access_token = access_token.trim().to_owned();
    }
    provider.balance = None;
    provider.balance_source = None;
    provider.updated_at = now();
    save_store_unlocked(paths, &store)?;
    Ok(balance_view(
        &store,
        store.providers.iter().find(|p| p.id == id).unwrap(),
    ))
}

pub fn set_balance_script(
    paths: &CodexPaths,
    id: &str,
    expected_base_url: &str,
    script: &str,
) -> Result<ProviderView, String> {
    super::balance_script::validate(script)?;
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let provider = store
        .providers
        .iter()
        .find(|provider| provider.id == id)
        .ok_or("newapi.errors.notFound")?;
    if provider.base_url != expected_base_url {
        return Err("newapi.errors.changed".into());
    }
    let owner = balance_owner(&store, provider).clone();
    let provider = store.providers.iter_mut().find(|p| p.id == id).unwrap();
    if !provider.balance_independent && owner.id != provider.id {
        provider.user_id = owner.user_id;
        provider.access_token = owner.access_token;
        provider.balance_refresh_interval_minutes = owner.balance_refresh_interval_minutes;
    }
    provider.balance_independent = true;
    provider.balance_script = script.trim().to_owned();
    provider.balance = None;
    provider.balance_source = None;
    provider.updated_at = now();
    save_store_unlocked(paths, &store)?;
    Ok(balance_view(
        &store,
        store.providers.iter().find(|p| p.id == id).unwrap(),
    ))
}

pub fn set_balance_sharing(
    paths: &CodexPaths,
    id: &str,
    expected_base_url: &str,
    shared: bool,
) -> Result<ProviderView, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let requested = store
        .providers
        .iter()
        .find(|p| p.id == id)
        .ok_or("newapi.errors.notFound")?;
    if requested.base_url != expected_base_url {
        return Err("newapi.errors.changed".into());
    }
    let owner = balance_owner(&store, requested).clone();
    let provider = store
        .providers
        .iter_mut()
        .find(|provider| provider.id == id)
        .unwrap();
    if !shared && owner.id != provider.id {
        provider.user_id = owner.user_id;
        provider.access_token = owner.access_token;
        provider.balance_script = owner.balance_script;
        provider.balance = owner.balance;
        provider.balance_source = owner.balance_source;
        provider.balance_refresh_interval_minutes = owner.balance_refresh_interval_minutes;
    } else if shared && owner.id != provider.id {
        provider.balance = None;
        provider.balance_source = None;
    }
    provider.balance_independent = !shared;
    provider.updated_at = now();
    save_store_unlocked(paths, &store)?;
    Ok(balance_view(
        &store,
        store.providers.iter().find(|p| p.id == id).unwrap(),
    ))
}

pub fn set_balance_refresh_interval(
    paths: &CodexPaths,
    id: &str,
    expected_base_url: &str,
    interval_minutes: u32,
) -> Result<ProviderView, String> {
    if interval_minutes > super::balance_refresh::MAX_INTERVAL_MINUTES {
        return Err("newapi.errors.balanceInterval".into());
    }
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let provider = store
        .providers
        .iter()
        .find(|provider| provider.id == id)
        .ok_or("newapi.errors.notFound")?;
    if provider.base_url != expected_base_url {
        return Err("newapi.errors.changed".into());
    }
    let owner_id = balance_owner(&store, provider).id.clone();
    let owner = store
        .providers
        .iter_mut()
        .find(|provider| provider.id == owner_id)
        .unwrap();
    owner.balance_refresh_interval_minutes = interval_minutes;
    owner.updated_at = now();
    save_store_unlocked(paths, &store)?;
    Ok(balance_view(
        &store,
        store
            .providers
            .iter()
            .find(|provider| provider.id == id)
            .unwrap(),
    ))
}

pub fn balance_refresh_targets(
    paths: &CodexPaths,
) -> Result<Vec<super::balance_refresh::Target>, String> {
    let store = load_store(paths)?;
    Ok(store
        .providers
        .iter()
        .filter(|provider| {
            provider.balance_refresh_interval_minutes > 0
                && balance_owner(&store, provider).id == provider.id
        })
        .map(|provider| super::balance_refresh::Target {
            id: provider.id.clone(),
            interval_minutes: provider.balance_refresh_interval_minutes,
            checked_at: provider
                .balance
                .as_ref()
                .map(|balance| balance.checked_at)
                .unwrap_or(provider.updated_at),
        })
        .collect())
}

pub fn query_provider_balance(
    paths: &CodexPaths,
    id: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<ProviderView, String> {
    query_balance(paths, id, proxy, false)
        .map(|view| view.expect("manual balance query always returns a provider"))
}

pub fn query_scheduled_provider_balance(
    paths: &CodexPaths,
    id: &str,
    proxy: &ApiProxyConfigPayload,
) -> Result<Option<ProviderView>, String> {
    query_balance(paths, id, proxy, true)
}

fn query_balance(
    paths: &CodexPaths,
    id: &str,
    proxy: &ApiProxyConfigPayload,
    scheduled: bool,
) -> Result<Option<ProviderView>, String> {
    let snapshot = {
        let _guard = store_write_guard()?;
        let store = load_store_unlocked(paths)?;
        let requested = store
            .providers
            .iter()
            .find(|p| p.id == id)
            .ok_or("newapi.errors.notFound")?;
        let owner = balance_owner(&store, requested);
        // Recheck at execution time: another card may have been querying while
        // the user disabled this owner's schedule or changed balance sharing.
        if scheduled && (owner.balance_refresh_interval_minutes == 0 || owner.id != id) {
            return Ok(None);
        }
        owner.clone()
    };
    let connection = newapi::Connection {
        base_url: snapshot.base_url.clone(),
        user_id: snapshot.user_id,
        access_token: snapshot.access_token.clone(),
        api_key: snapshot.api_key.clone(),
        ..Default::default()
    };
    let provider_proxy = newapi::effective_proxy(snapshot.network_mode(), proxy);
    let (balance, source) = if snapshot.balance_script.is_empty() {
        router_balance::query(&connection, &provider_proxy)?
    } else {
        (
            super::balance_script::query(&connection, &provider_proxy, &snapshot.balance_script)?,
            "custom",
        )
    };
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let requested = store
        .providers
        .iter()
        .find(|p| p.id == id)
        .ok_or("newapi.errors.notFound")?;
    if balance_owner(&store, requested).id != snapshot.id {
        return Err("newapi.errors.changed".into());
    }
    let index = store
        .providers
        .iter()
        .position(|p| p.id == snapshot.id)
        .unwrap();
    let provider = &mut store.providers[index];
    if provider.base_url != snapshot.base_url
        || provider.user_id != snapshot.user_id
        || provider.access_token != snapshot.access_token
        || provider.api_key != snapshot.api_key
        || provider.balance_script != snapshot.balance_script
        || provider.network_mode() != snapshot.network_mode()
    {
        return Err("newapi.errors.changed".into());
    }
    provider.balance = Some(balance);
    provider.balance_source = Some(source.into());
    save_store_unlocked(paths, &store)?;
    Ok(Some(balance_view(
        &store,
        store.providers.iter().find(|p| p.id == id).unwrap(),
    )))
}

pub fn set_smart_routing(paths: &CodexPaths, enabled: bool) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    set_smart_routing_unlocked(paths, enabled)
}

fn set_smart_routing_unlocked(paths: &CodexPaths, enabled: bool) -> Result<StateChange, String> {
    let mut store = load_store_unlocked(paths)?;
    let previous = store.clone();
    let previously_owned = active_routing_config_is_ours(paths);
    let applied;
    if enabled {
        validate_native_selection(&store)?;
        if !codex_target_available(&store) {
            return Err("newapi.errors.noActiveProvider".into());
        }
        applied = Some(apply_routing_config(paths, &store)?);
        store.router.enabled = true;
    } else {
        applied = remove_routing_config(paths, &store)?;
        store.router.enabled = false;
    }
    if let Err(error) = save_store_unlocked(paths, &store) {
        if enabled {
            let rollback = if previously_owned {
                apply_routing_config(paths, &previous).map(|_| ())
            } else {
                remove_routing_config(paths, &store).map(|_| ())
            };
            if rollback.is_err() {
                log::warn!("Codex connection rollback failed after store write error");
            }
        }
        return Err(error);
    }
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

pub fn select_aggregate(paths: &CodexPaths, id: Option<String>) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    if id.is_some() && store.codex_access_mode == CodexAccessMode::Native {
        return Err("aggregation.errors.routerOnly".into());
    }
    if let Some(id) = &id {
        let aggregate = store
            .aggregates
            .iter()
            .find(|aggregate| aggregate.id == *id)
            .ok_or("aggregation.errors.notFound")?;
        if !super::aggregation::project(&store, aggregate, &models_dev::load(&paths.app_data_dir))
            .available
        {
            return Err("aggregation.errors.unavailable".into());
        }
    }
    let previous = store.clone();
    store.router.aggregate_model_id = id;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

fn codex_target_available(store: &Store) -> bool {
    if let Some(id) = store.router.aggregate_model_id.as_deref() {
        store.codex_access_mode == CodexAccessMode::Router
            && store.aggregates.iter().any(|aggregate| {
                aggregate.id == id
                    && aggregate.enabled
                    && aggregate.members.iter().any(|member| {
                        member.enabled
                            && store.providers.iter().any(|provider| {
                                provider.id == member.provider_id
                                    && provider.can_route_model(&member.actual_model_id)
                            })
                    })
            })
    } else {
        effective_active_for(store, ModelProtocol::Responses).is_some()
    }
}

/// Only identifies the REST search service; model protocols are independent.
pub(crate) fn supports_command_search(provider: &Provider) -> bool {
    if provider.model_credential().api_key.trim().is_empty() {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(&provider.base_url) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("api.commandcode.ai")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && matches!(url.path().trim_end_matches('/'), "" | "/provider" | "/provider/v1")
}

pub fn set_web_search_provider(
    paths: &CodexPaths,
    provider_id: Option<String>,
) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    if let Some(id) = provider_id.as_deref() {
        let provider = store.providers.iter().find(|provider| provider.id == id)
            .ok_or("newapi.errors.notFound")?;
        if !supports_command_search(provider) {
            return Err("newapi.errors.webSearchProviderUnsupported".into());
        }
    }
    store.router.web_search_provider_id = provider_id;
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied: None,
    })
}

/// Global compression policy. Applying it re-derives every injected threshold,
/// so an active configuration keeps matching what the UI shows.
pub fn set_compression(
    paths: &CodexPaths,
    policy: model_policy::CompressionPolicy,
) -> Result<StateChange, String> {
    policy.validate().map_err(|_| "newapi.errors.context")?;
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let previous = store.clone();
    store.compression = policy;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

/// Explicit per-model price override. Prices never affect routing, so nothing
/// is re-applied; the ledger prices new requests with the updated rates.
pub fn set_model_price_override(
    paths: &CodexPaths,
    provider_id: &str,
    model_id: &str,
    price: Option<newapi::PriceOverrides>,
) -> Result<StateChange, String> {
    let price = price.filter(|rates| {
        [
            rates.input,
            rates.output,
            rates.cache_read,
            rates.cache_write,
        ]
        .iter()
        .any(|rate| rate.is_some())
    });
    if price.as_ref().is_some_and(|rates| {
        [
            rates.input,
            rates.output,
            rates.cache_read,
            rates.cache_write,
        ]
        .iter()
        .any(|rate| rate.is_some_and(|rate| !rate.is_finite() || rate < 0.0 || rate > 1_000_000.0))
    }) {
        return Err("newapi.errors.context".into());
    }
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let provider = store
        .providers
        .iter_mut()
        .find(|provider| provider.id == provider_id)
        .ok_or("newapi.errors.notFound")?;
    let model = provider
        .models
        .iter_mut()
        .find(|model| model.id == model_id)
        .ok_or("newapi.errors.notFound")?;
    model.overrides.price = price;
    provider.updated_at = now();
    save_store_unlocked(paths, &store)?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied: None,
    })
}

/// A clean app exit ends this session's route. A previous run's saved toggle
/// alone does not confer ownership of Codex's current config.
/// Exit and port-conflict cleanup: the router is going away, so Codex must
/// stop pointing at it. Threads keep their provider; nothing blocks exit.
pub fn stop_smart_routing_on_exit(paths: &CodexPaths) -> Result<(), String> {
    let _threads = super::thread_recovery::thread_mutation_guard().map_err(|e| e.to_string())?;
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    if !store.router.enabled && !config_snapshot::path(paths).is_file() {
        return Ok(());
    }
    remove_routing_config(paths, &store)?;
    store.router.enabled = false;
    save_store_unlocked(paths, &store)?;
    // A running Codex may still write its catalog rows; normalize next time.
    if !crate::platform::process::is_codex_app_running() {
        if let Err(error) = transition::normalize_threads(paths) {
            log::warn!("Codex thread catalog normalization skipped: {error}");
        }
    }
    Ok(())
}

/// Best effort whenever Codex is stopped (startup, restart): rows left on an
/// earlier build's router provider become visible and usable again.
pub fn normalize_threads_when_stopped(paths: &CodexPaths) {
    if crate::platform::process::is_codex_app_running() {
        return;
    }
    let (Ok(_threads), Ok(_store)) = (
        super::thread_recovery::thread_mutation_guard(),
        store_write_guard(),
    ) else {
        return;
    };
    transition::clear_restart_pending();
    if let Err(error) = transition::normalize_threads(paths) {
        log::warn!("Codex thread catalog normalization skipped: {error}");
    }
}

pub fn set_access_mode(paths: &CodexPaths, mode: CodexAccessMode) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    set_access_mode_unlocked(paths, mode)
}

fn set_access_mode_unlocked(paths: &CodexPaths, mode: CodexAccessMode) -> Result<StateChange, String> {
    let mut store = load_store_unlocked(paths)?;
    if store.codex_access_mode == mode {
        return Ok(StateChange {
            state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
            applied: None,
        });
    }
    let previous = store.clone();
    if mode == CodexAccessMode::Native {
        if store.router.aggregate_model_id.is_some() {
            return Err("aggregation.errors.routerOnly".into());
        }
        // Pin the provider currently shown/used by Codex before changing modes.
        // Native validation must not choose a different provider by protocol.
        store.active_provider_id = effective_active_for(&store, ModelProtocol::Responses)
            .map(|provider| provider.id.clone());
    }
    store.codex_access_mode = mode;
    validate_native_selection(&store)?;
    let applied = apply_routing_if_enabled(paths, &store)?;
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        applied,
    })
}

fn persist_connection_change(
    paths: &CodexPaths,
    previous: &Store,
    store: &Store,
    config_changed: bool,
) -> Result<(), String> {
    if let Err(error) = save_store_unlocked(paths, store) {
        if config_changed {
            let restored = if previous.router.enabled && active_routing_config_is_ours(paths) {
                apply_routing_config(paths, previous).map(|_| ())
            } else {
                remove_routing_config(paths, previous).map(|_| ())
            };
            if restored.is_err() {
                log::warn!("Codex connection rollback failed; persisted settings will be retried on next load");
            }
        }
        return Err(error);
    }
    Ok(())
}

pub fn set_accountless(paths: &CodexPaths, accountless: bool) -> Result<StateChange, String> {
    let _guard = store_write_guard()?;
    let mut store = load_store_unlocked(paths)?;
    let previous = store.clone();
    store.router.accountless = accountless;
    let applied = if store.codex_access_mode == CodexAccessMode::Router {
        apply_routing_if_enabled(paths, &store)?
    } else {
        None
    };
    persist_connection_change(paths, &previous, &store, applied.is_some())?;
    Ok(StateChange {
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
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
    let exact = |provider: &&Provider| provider.is_routable_for(protocol);
    store
        .active_provider_id
        .as_ref()
        .and_then(|id| {
            store.providers.iter().find(|provider| {
                &provider.id == id
                    && provider.is_routable()
                    && protocol::Translation::can_serve(protocol, provider.selected_protocol())
            })
        })
        .or_else(|| store.providers.iter().find(exact))
        // A provider speaking another protocol is still reachable through the
        // router's translation, so it can carry this protocol's traffic when no
        // native match exists.
        .or_else(|| {
            store.active_provider_id.as_ref().and_then(|id| {
                store
                    .providers
                    .iter()
                    .find(|provider| &provider.id == id && provider.is_routable())
            })
        })
        .or_else(|| {
            store
                .providers
                .iter()
                .find(|provider| provider.is_routable())
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
        .and_then(toml_edit::Item::as_table_like_mut)
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

fn reject_foreign_managed_provider_entries(
    document: &toml_edit::DocumentMut,
) -> Result<(), CoreError> {
    if document
        .get("model_providers")
        .and_then(toml_edit::Item::as_table_like)
        .is_some_and(|providers| providers.iter().any(|(key, _)| managed_provider_key(key)))
    {
        return Err(CoreError::OperationFailed(
            "newapi.errors.routerSnapshot".into(),
        ));
    }
    Ok(())
}

fn model_provider_is_ours(document: &toml_edit::DocumentMut) -> bool {
    document
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(managed_provider_key)
}

/// Account-mode routing: Codex's built-in `openai` provider points at the
/// router, so threads keep their provider while routing is toggled.
pub(crate) fn builtin_router_base(store: &Store) -> String {
    format!(
        "http://127.0.0.1:{}/openai/{}/v1",
        store.router.port, store.router.bearer_token
    )
}

/// The base URL points at this app's router, whichever provider is selected.
fn router_base_is_ours(document: &toml_edit::DocumentMut) -> bool {
    document
        .get("openai_base_url")
        .and_then(toml_edit::Item::as_str)
        .and_then(|url| url.strip_prefix("http://127.0.0.1:"))
        .and_then(|rest| rest.split_once("/openai/"))
        .is_some_and(|(port, _)| port.parse::<u16>().is_ok_and(|port| port != 0))
}

fn builtin_provider_selected(document: &toml_edit::DocumentMut) -> bool {
    document
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .is_none_or(|provider| provider == "openai")
}

/// Whether config.toml currently routes Codex through this app. Another
/// tool selecting its own provider takes over, even if our URL remains.
pub(crate) fn route_is_ours(document: &toml_edit::DocumentMut) -> bool {
    model_provider_is_ours(document)
        || router_base_is_ours(document) && builtin_provider_selected(document)
}

pub(crate) fn managed_provider_key(key: &str) -> bool {
    key == DIRECT_PROVIDER_KEY
        || key == NATIVE_PROVIDER_KEY
        || key == CATALOG_PROVIDER_KEY
        || key == AGGREGATE_PROVIDER_KEY
        || key.starts_with(PROVIDER_PREFIX)
        || key == PREVIOUS_DIRECT_PROVIDER_KEY
        || key == PREVIOUS_CATALOG_PROVIDER_KEY
        || key.starts_with(PREVIOUS_PROVIDER_PREFIX)
}

pub(crate) fn is_gateway_provider_key(key: &str) -> bool {
    managed_provider_key(key) && key != NATIVE_PROVIDER_KEY
}

fn remove_unreferenced_provider_entries(
    paths: &CodexPaths,
    document: &mut toml_edit::DocumentMut,
) -> Result<(), CoreError> {
    // A partially completed Native transition still has usable sessions bound
    // to its prepared provider. A routine Router refresh must not orphan them.
    let native_in_use = !super::thread_recovery::threads_with_provider(paths, NATIVE_PROVIDER_KEY)?.is_empty();
    let native = native_in_use.then(|| document.get("model_providers")?.get(NATIVE_PROVIDER_KEY).cloned()).flatten();
    remove_managed_provider_entries(document);
    if let Some(entry) = native {
        document["model_providers"][NATIVE_PROVIDER_KEY] = entry;
    }
    Ok(())
}

/// Whether Codex's config still carries the router injection we last applied.
///
/// Any tool that rewrites the same `config.toml` replaces our managed keys, and
/// Codex then talks to whatever provider it now names. This is a read-only probe
/// used for reporting. A missing or unparsable config is drift, not a valid
/// active injection.
pub(crate) fn routing_config_intact(paths: &CodexPaths) -> bool {
    let Ok(Some(text)) = CodexConfigRepository::new(paths).read_optional_text() else {
        return false;
    };
    let Ok(document) = text.parse::<toml_edit::DocumentMut>() else {
        return false;
    };
    let key = document
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or_default();
    let builtin = router_base_is_ours(&document) && builtin_provider_selected(&document);
    let expected_catalogs = if builtin {
        vec![catalog_path(paths), aggregate_catalog_path(paths)]
    } else if key == AGGREGATE_PROVIDER_KEY {
        vec![aggregate_catalog_path(paths)]
    } else {
        vec![catalog_path(paths)]
    };
    (builtin
        || document
            .get("model_providers")
            .and_then(toml_edit::Item::as_table_like)
            .and_then(|providers| providers.get(key))
            .is_some()
            && matches!(key, CATALOG_PROVIDER_KEY | AGGREGATE_PROVIDER_KEY))
        && document
            .get("model_catalog_json")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(|reference| {
                let referenced = PathBuf::from(reference);
                let referenced = if referenced.is_absolute() {
                    referenced
                } else {
                    paths.codex_home.join(referenced)
                };
                expected_catalogs.iter().any(|expected| {
                    referenced == *expected
                        || referenced
                            .canonicalize()
                            .ok()
                            .zip(expected.canonicalize().ok())
                            .is_some_and(|(a, b)| a == b)
                })
            })
}

pub(crate) fn catalog_path(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join("newapi-router-catalog.json")
}

fn aggregate_catalog_path(paths: &CodexPaths) -> PathBuf {
    paths.app_data_dir.join("aggregate-router-catalog.json")
}
fn selected_catalog_path(paths: &CodexPaths, store: &Store) -> PathBuf {
    if store.router.aggregate_model_id.is_some() {
        aggregate_catalog_path(paths)
    } else {
        catalog_path(paths)
    }
}
pub(crate) fn aggregate_catalog_value(
    app_data_dir: &std::path::Path,
    codex_home: &std::path::Path,
    store: &Store,
) -> Value {
    let catalog = models_dev::load(app_data_dir);
    let official = super::model_catalog::official(codex_home, app_data_dir);
    let models: Vec<_> = store
        .aggregates
        .iter()
        .filter_map(|aggregate| {
            let view = super::aggregation::project(store, aggregate, &catalog);
            view.projection.filter(|_| view.available).map(|projection| {
                let mut entry = aggregate_catalog_entry(aggregate, &projection);
                let model = matched_official_model(&official.models, &aggregate.public_model_id);
                adopt_codex_behavior(&mut entry, model);
                entry
            })
        })
        .collect();
    serde_json::json!({"models":models})
}

/// Codex reads relay + official models from this catalog file (`model_catalog_json`),
/// matching the original app's protocol. Official entries are borrowed read-only from
/// the original app's catalog when it is present.
pub(crate) fn write_catalog(paths: &CodexPaths, store: &Store) -> Result<PathBuf, String> {
    let document = if store.router.aggregate_model_id.is_some() {
        aggregate_catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    } else {
        catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    };
    let target = selected_catalog_path(paths, store);
    let bytes =
        serde_json::to_vec_pretty(&document).map_err(|_| "newapi.errors.storage".to_owned())?;
    storage::write_atomic(&target, &bytes)
        .map_err(|_| "newapi.errors.storage".to_owned())?;
    Ok(target)
}

/// Codex refreshes its account model cache after login. Keep the injected
/// catalog file in sync without rewriting or reclaiming config.toml.
pub(crate) fn refresh_catalog_if_changed(paths: &CodexPaths, store: &Store) -> Result<(), String> {
    let expected = if store.router.aggregate_model_id.is_some() {
        aggregate_catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    } else {
        catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    };
    let path = selected_catalog_path(paths, store);
    if std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .as_ref()
        == Some(&expected)
    {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&expected)
        .map_err(|_| "newapi.errors.storage".to_owned())?;
    storage::write_atomic(&path, &bytes).map_err(|_| "newapi.errors.storage".to_owned())
}

/// Pure catalog builder (no disk writes) so the local proxy can serve it per request.
pub(crate) fn catalog_value(
    app_data_dir: &std::path::Path,
    codex_home: &std::path::Path,
    store: &Store,
) -> Value {
    let mut models: Vec<Value> = Vec::new();
    let mut slugs = HashSet::new();
    // Retained login credentials/catalogs do not grant official model access
    // while the user explicitly selects accountless routing.
    if !store.router.accountless {
        let snapshot = super::model_catalog::account_catalog(codex_home, app_data_dir);
        for entry in &snapshot.models {
            let Some(slug) = entry.get("slug").and_then(Value::as_str) else {
                continue;
            };
            let is_relay = slug.starts_with(PREVIOUS_RELAY_MODEL_PREFIX) || slug.contains("::");
            if !is_relay && slugs.insert(slug.to_owned()) {
                models.push(entry.clone());
            }
        }
    }
    // Capability history can inform a relay without expanding account access.
    let official_metadata = super::model_catalog::official(codex_home, app_data_dir);
    let models_dev = models_dev::load(app_data_dir);
    let global_compression = store.global_compression();
    // Any routable provider can back Codex: a non-Responses relay is reached
    // through the router's protocol translation, so it belongs in the menu.
    for provider in store
        .providers
        .iter()
        .filter(|provider| provider.has_route_credentials())
    {
        let mut provider_models: Vec<String> = provider
            .models
            .iter()
            .filter(|model| {
                // Models the relay flagged as unusable for Responses are still
                // offered, since the translation layer speaks the relay's own
                // protocol; only excluded models are left out.
                model.included && provider.supports_model(&model.id)
            })
            .map(|model| model.id.clone())
            .collect();
        if !provider.model.trim().is_empty()
            && provider.supports_model(&provider.model)
            && !provider_models.contains(&provider.model)
        {
            provider_models.push(provider.model.clone());
        }
        for model_id in provider_models {
            let slug = format!("{}::{model_id}", provider.id);
            if !slugs.insert(slug.clone()) {
                continue;
            }
            models.push(relay_menu_entry(
                store,
                provider,
                &model_id,
                matched_official_model(&official_metadata.models, &model_id),
                &models_dev,
                &global_compression,
            ));
        }
    }
    serde_json::json!({ "models": models })
}

/// One relay menu entry for `provider` + `model_id`. Shared by the full catalog
/// builder and the single-entry lookup, so the menu and the request
/// compatibility path can never disagree about a model. `official` is the
/// same model's official entry: its limits apply only to the official endpoint,
/// its client behaviour to every endpoint.
fn relay_menu_entry(
    store: &Store,
    provider: &Provider,
    model_id: &str,
    official: Option<&Value>,
    models_dev: &models_dev::Catalog,
    global_compression: &model_policy::CompressionPolicy,
) -> Value {
    let (metadata, projection) = relay_model_projection(
        store,
        provider,
        model_id,
        official.filter(|_| official_endpoint_applies(provider)),
        models_dev,
        global_compression,
    );
    let alias = metadata.display_name.trim();
    let display_name = menu_display_name(&provider.name, alias, model_id);
    // Codex parses every entry in model_catalog_json as a complete model
    // definition during config/read. A short display-only entry makes that RPC
    // fail, which also blocks the desktop sign-in page.
    let mut entry = relay_catalog_entry(
        format!("{}::{model_id}", provider.id),
        display_name,
        format!("{} → {model_id}", provider.name),
        projection.context_window,
        projection.output_limit,
        projection.reasoning,
        projection.default_reasoning,
    );
    // A unified menu must not apply one provider's compaction threshold
    // globally to unrelated official/relay models.
    if let Some(threshold) = projection.compression.threshold {
        entry["auto_compact_token_limit"] = serde_json::json!(threshold);
    }
    adopt_codex_behavior(&mut entry, official);
    entry
}

/// One entry of the menu a request would see, by slug: either a borrowed
/// official entry or a relay entry for a routable provider. Used on the request
/// hot path so a compatibility check never builds the whole menu.
pub(crate) fn catalog_entry_for_slug(
    app_data_dir: &std::path::Path,
    codex_home: &std::path::Path,
    store: &Store,
    slug: &str,
) -> Option<Value> {
    let Some((provider_id, model_id)) = slug.split_once("::") else {
        if store.router.accountless {
            return None;
        }
        // Official entries are borrowed read-only and keep their own shape.
        return super::model_catalog::account_catalog(codex_home, app_data_dir)
            .models
            .iter()
            .find(|entry| entry.get("slug").and_then(Value::as_str) == Some(slug))
            .cloned();
    };
    let official_snapshot = super::model_catalog::official(codex_home, app_data_dir);
    let provider = store
        .providers
        .iter()
        .find(|provider| provider.id == provider_id && provider.has_route_credentials())?;
    // Mirror the menu's inclusion rules; a request for a model the menu does not
    // list must not silently gain an entry.
    let listed =
        provider.models.iter().any(|model| {
            model.id == model_id && model.included && provider.supports_model(&model.id)
        }) || (provider.model == model_id
            && !provider.model.trim().is_empty()
            && provider.supports_model(&provider.model));
    if !listed {
        return None;
    }
    let models_dev = models_dev::load(app_data_dir);
    Some(relay_menu_entry(
        store,
        provider,
        model_id,
        matched_official_model(&official_snapshot.models, model_id),
        &models_dev,
        &store.global_compression(),
    ))
}

pub(super) fn matched_official_model<'a>(models: &'a [Value], model_id: &str) -> Option<&'a Value> {
    let key = model_defaults::model_key(model_id);
    models.iter().find(|entry| {
        entry
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| {
                !model_defaults::channel_entry(slug) && model_defaults::model_key(slug) == key
            })
    })
}

/// The official account snapshot describes the signed-in plan's own models. It
/// may supply values only when the provider actually targets the official
/// endpoint; a third-party relay with the same model name must not inherit
/// account limits just because `model_key` matches.
pub(super) fn official_endpoint_applies(provider: &Provider) -> bool {
    reqwest::Url::parse(provider.base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            matches!(
                host.as_str(),
                "chatgpt.com" | "chat.openai.com" | "api.openai.com"
            )
        })
}

/// Publisher catalogs allowed for automatic metadata references. Hosting and
/// aggregation services remain available through explicit user bindings only.
/// Publisher IDs absent from the bundled snapshot are reserved for future
/// refreshes; recognized families still fail closed when their records are absent.
const OFFICIAL_CATALOGS: &[&str] = &[
    "openai",
    "nova",
    "anthropic",
    "google",
    "deepseek",
    "xiaomi",
    "alibaba",
    "moonshotai",
    "zhipuai",
    "xai",
    "mistral",
    "meta",
    "llama",
    "cohere",
    "microsoft",
    "ai21",
    "minimax",
    "baidu",
    "tencent-tokenhub",
    "stepfun",
    "01-ai",
    "internlm",
    "baichuan",
    "bytedance",
    "perplexity",
    "inception",
    "morph",
    "longcat",
    "sarvam",
    "sensenova",
    "thinkingmachines",
    "poolside",
    "bailing",
];

/// Bind to the publisher first, then an official endpoint for otherwise opaque
/// model IDs. Host matching is exact; a relay cannot claim an official endpoint
/// through a similar domain name.
fn effective_catalog_scope(
    provider: &Provider,
    model_id: &str,
    catalog: &models_dev::Catalog,
) -> (Vec<String>, bool) {
    if !provider.catalog_providers.is_empty() {
        return (provider.catalog_providers.clone(), false);
    }
    // A model's publisher is the authoritative metadata source even when the
    // request itself goes through an OpenAI-compatible relay. Endpoint scope
    // is only the fallback for model IDs without a recognized publisher.
    if let Some(provider_id) = official_catalog_for_model(model_id, catalog) {
        return (vec![provider_id], true);
    }
    let Some(host) = reqwest::Url::parse(provider.base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
    else {
        return (Vec::new(), false);
    };
    let mut matches = catalog.providers_for_host(&host).iter()
        .filter(|id| official_catalog_namespace(id).is_some())
        .cloned().collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    let inferred = !matches.is_empty();
    (matches, inferred)
}

fn official_catalog_for_model(model_id: &str, catalog: &models_dev::Catalog) -> Option<String> {
    // Reuse the resolver's safe normalization for namespaced/channel model IDs.
    // A recognized publisher stays strict even if its catalog is absent, rather
    // than silently borrowing a different service's data.
    for candidate in model_metadata::candidates(model_id) {
        let id = candidate.value.to_ascii_lowercase();
        // Bedrock is a host for many publishers. Only Amazon's own Nova IDs
        // receive this scope; Bedrock's Claude/GPT entries remain excluded
        // from automatic global metadata references.
        // Recognized region prefixes are removed by candidates(); do not
        // accept arbitrary namespaces merely ending in amazon.nova-*.
        let bedrock_id = id.strip_prefix("amazon.");
        if bedrock_id.is_some_and(|name| name.starts_with("nova-")) {
            return Some("amazon-bedrock".to_owned());
        }
        if id.strip_prefix("amazon/").is_some_and(|name| name.starts_with("nova-")) {
            return Some("amazon-bedrock".to_owned());
        }
        let model_name = id.rsplit_once('/').map_or(id.as_str(), |(_, name)| name);
        if let Some(owner) = official_model_family(model_name) {
            return Some(owner.to_owned());
        }
        if let Some(owner) = id
            .split_once('/')
            .and_then(|(namespace, _)| official_catalog_namespace(namespace))
        {
            return Some(owner.to_owned());
        }
        let mut identities = catalog.global(&id).into_iter()
            .filter(|entry| official_catalog_namespace(&entry.provider.id).is_some())
            .collect::<Vec<_>>();
        identities.extend(catalog.canonical_scoped(OFFICIAL_CATALOGS, &id));
        let mut owners = identities
            .into_iter()
            .filter_map(|entry| {
                let namespace = entry
                    .model
                    .canonical_model_id
                    .as_deref()?
                    .split('/')
                    .next()?;
                official_catalog_namespace(namespace)
            })
            .collect::<Vec<_>>();
        owners.sort_unstable();
        owners.dedup();
        if owners.len() == 1 {
            return Some(owners[0].to_owned());
        }
    }
    None
}

fn official_model_family(id: &str) -> Option<&'static str> {
    if ["o1", "o3", "o4"].iter().any(|prefix| {
        id == *prefix
            || id
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('-'))
    }) {
        return Some("openai");
    }
    [
        ("gpt-", "openai"),
        ("chatgpt-", "openai"),
        ("codex-mini", "openai"),
        ("claude-", "anthropic"),
        ("gemini-", "google"),
        ("gemma-", "google"),
        ("deepseek-", "deepseek"),
        ("mimo-", "xiaomi"),
        ("qwen", "alibaba"),
        ("qwq-", "alibaba"),
        ("qvq-", "alibaba"),
        ("kimi-", "moonshotai"),
        ("moonshot-", "moonshotai"),
        ("glm-", "zhipuai"),
        ("grok-", "xai"),
        ("mistral-", "mistral"),
        ("codestral-", "mistral"),
        ("magistral-", "mistral"),
        ("ministral-", "mistral"),
        ("pixtral-", "mistral"),
        ("devstral-", "mistral"),
        ("llama-", "llama"),
        ("muse-", "meta"),
        ("command-", "cohere"),
        ("phi-", "microsoft"),
        ("nova-", "nova"),
        ("jamba-", "ai21"),
        ("minimax-", "minimax"),
        ("ernie-", "baidu"),
        ("hunyuan-", "tencent-tokenhub"),
        ("hy3", "tencent-tokenhub"),
        ("hy4", "tencent-tokenhub"),
        ("step-", "stepfun"),
        ("sonar", "perplexity"),
        ("mercury-", "inception"),
        ("morph-", "morph"),
        ("longcat-", "longcat"),
        ("sarvam-", "sarvam"),
        ("bailing-", "bailing"),
    ]
    .into_iter()
    .find_map(|(prefix, owner)| id.starts_with(prefix).then_some(owner))
}

fn official_catalog_namespace(namespace: &str) -> Option<&'static str> {
    let normalized = namespace.to_ascii_lowercase();
    let namespace = match normalized.as_str() {
        "alibaba-cn"
        | "alibaba-coding-plan"
        | "alibaba-coding-plan-cn"
        | "alibaba-token-plan"
        | "alibaba-token-plan-cn" => "alibaba",
        "xiaomi-token-plan-cn" => "xiaomi",
        "moonshotai-cn" | "kimi-code-plan-cn" | "kimi-code-plan-global" => "moonshotai",
        "minimax-cn" | "minimax-coding-plan" | "minimax-cn-coding-plan" => "minimax",
        "zai" | "zai-coding-plan" | "zhipuai-coding-plan" => "zhipuai",
        "stepfun-ai" | "stepfun-ai-step-plan" | "stepfun-step-plan" => "stepfun",
        "meituan" => "longcat",
        "tencent" => "tencent-tokenhub",
        other => other,
    };
    OFFICIAL_CATALOGS
        .iter()
        .copied()
        .find(|id| *id == namespace)
}

pub(super) fn resolve_model_metadata(
    provider: &Provider,
    model_id: &str,
    official: Option<&Value>,
    models_dev: &models_dev::Catalog,
) -> model_metadata::ResolvedMetadata {
    let saved = provider.models.iter().find(|model| model.id == model_id);
    let (catalog_scope, strict_catalog_scope) =
        effective_catalog_scope(provider, model_id, models_dev);
    // Price the actual channel first. Legacy bindings retain precedence;
    // capability matching supplies a separately labelled reference fallback.
    let price_scope: Vec<String> = match provider.price_catalog_provider.as_deref() {
        Some(binding) if !binding.trim().is_empty() => vec![binding.trim().to_lowercase()],
        _ => reqwest::Url::parse(provider.base_url.trim())
            .ok()
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .map(|host| models_dev.providers_for_host(&host).to_vec())
            .unwrap_or_default(),
    };
    let request = model_metadata::ResolutionRequest {
        raw_model_id: model_id,
        display_name: saved.map(|model| model.display_name.as_str()).unwrap_or(""),
        catalog_providers: &catalog_scope,
        price_providers: &price_scope,
        explicit_ref: saved.and_then(|model| model.catalog_ref.as_ref()),
        price_ref: saved.and_then(|model| model.price_ref.as_ref()),
        user: saved.map(newapi::Model::user_values).unwrap_or_default(),
        provider_context: provider.context_window,
        official: official.and_then(official_values),
        upstream: saved.and_then(newapi::Model::upstream_values),
    };
    let mut resolved = model_metadata::resolve_with_global_fallback(
        models_dev,
        &request,
        !strict_catalog_scope,
        Some(OFFICIAL_CATALOGS),
    );
    resolved.price = resolved.price.with_multiplier(provider.price_multiplier);
    resolved
}

/// 与 [`resolve_model_metadata`] 相同的有效能力解析，供 `core` 内的只读消费者
/// （Claude Desktop 直连的能力投影）复用，不改变任何既有调用方的语义。
pub(super) fn model_metadata_for(
    provider: &Provider,
    model_id: &str,
    models_dev: &models_dev::Catalog,
) -> model_metadata::ResolvedMetadata {
    resolve_model_metadata(provider, model_id, None, models_dev)
}

fn official_values(entry: &Value) -> Option<model_metadata::OfficialValues> {
    let number = |key: &str| {
        entry
            .get(key)
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
    };
    let levels = entry
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .filter(|levels| levels.len() <= 32)
        .map(|levels| {
            levels
                .iter()
                .filter_map(|level| {
                    let effort = level.get("effort")?.as_str()?.trim();
                    let description = level
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or(effort);
                    let level = newapi::ReasoningLevel {
                        effort: effort.to_owned(),
                        description: description.to_owned(),
                    };
                    level.valid().then_some(level)
                })
                .collect::<Vec<_>>()
        });
    let default_effort = entry
        .get("default_reasoning_level")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if number("context_window").is_none()
        && number("max_output_tokens").is_none()
        && levels.is_none()
    {
        return None;
    }
    Some(model_metadata::OfficialValues {
        context_window: number("context_window"),
        max_output_tokens: number("max_output_tokens"),
        reasoning: levels,
        default_effort,
        updated_at: None,
    })
}

/// The provider-level policy, bridging the historical fixed threshold until
/// the field is removed: an explicit policy always wins, and `inherit` reads
/// the legacy value when one exists.
pub(super) fn provider_compression(provider: &Provider) -> model_policy::CompressionPolicy {
    match provider.compression {
        model_policy::CompressionPolicy::Inherit => {
            model_policy::migrate_legacy_limit(provider.auto_compact_limit)
        }
        ref policy => policy.clone(),
    }
}

/// Price snapshot fixed when a request starts. Recorded in the usage ledger so
/// a later catalog refresh or price edit cannot rewrite what already happened.
pub(crate) fn pricing_for(
    provider: &Provider,
    model_id: &str,
    catalog: &models_dev::Catalog,
) -> relay_usage::PricingSnapshot {
    let metadata = resolve_model_metadata(provider, model_id, None, catalog);
    let price = metadata.price;
    // Static preview completeness and request-time evaluability differ:
    // supported conditional tiers can be settled once prompt usage is known.
    let complete = price.source != model_metadata::source::UNKNOWN
        && price.input_audio.is_none()
        && price.output_audio.is_none()
        && price
            .context_over_200k
            .as_ref()
            .is_none_or(|rates| !rates.has_audio())
        && price.tiers.iter().all(|tier| !tier.rates.has_audio())
        && price
            .tiers
            .iter()
            .all(|tier| tier.tier.condition_type == "context" && tier.tier.size.is_some());
    relay_usage::PricingSnapshot {
        input: price.input,
        output: price.output,
        cache_read: price.cache_read,
        cache_write: price.cache_write,
        reasoning: price.reasoning,
        input_audio: price.input_audio,
        output_audio: price.output_audio,
        context_over_200k: price.context_over_200k,
        tiers: price.tiers,
        source: price.source,
        multiplier: price.multiplier,
        revision: catalog.snapshot.revision.clone(),
        match_kind: metadata.match_kind.as_str().into(),
        complete,
        currency: price.currency,
        source_ref: price.source_ref,
    }
}

/// Whether the native Codex configuration still matches what we would apply.
/// Read-only; used by the capability preview to separate "pending" from
/// "applied" thresholds.
pub(crate) fn native_compression_state(
    paths: &CodexPaths,
    store: &Store,
    catalog: &models_dev::Catalog,
    official: &[Value],
) -> Option<native_config::NativeCompressionState> {
    native_config::compression_state(paths, store, catalog, official)
}

/// Auto-compact thresholds recorded in the currently written router catalog,
/// keyed by model slug (`provider::model`). `None` when no catalog is on disk.
pub(crate) fn applied_catalog_thresholds(
    paths: &CodexPaths,
) -> Option<HashMap<String, Option<u64>>> {
    let bytes = std::fs::read(catalog_path(paths)).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let models = value.get("models")?.as_array()?;
    let mut thresholds = HashMap::new();
    for entry in models {
        let Some(slug) = entry.get("slug").and_then(Value::as_str) else {
            continue;
        };
        thresholds.insert(
            slug.to_owned(),
            entry
                .get("auto_compact_token_limit")
                .and_then(Value::as_u64),
        );
    }
    Some(thresholds)
}

/// The one projection every consumer reads: the router catalog, the native
/// catalog and the capability preview cannot disagree about effective values.
pub(super) fn model_projection(
    provider: &Provider,
    model_id: &str,
    official: Option<&Value>,
    models_dev: &models_dev::Catalog,
    global_compression: &model_policy::CompressionPolicy,
) -> (
    model_metadata::ResolvedMetadata,
    model_policy::ClientProjection,
) {
    let metadata = resolve_model_metadata(provider, model_id, official, models_dev);
    let model_policy_ref = provider
        .models
        .iter()
        .find(|model| model.id == model_id)
        .map(|model| &model.compression);
    let provider_policy = provider_compression(provider);
    let (policy, source) =
        model_policy::resolve(model_policy_ref, Some(&provider_policy), global_compression);
    let projection = model_policy::project(&metadata, (policy, source));
    (metadata, projection)
}

/// Ordinary relay budgets depend only on their own saved model. Explicit
/// aggregates project their eligible members in aggregation::projection.
pub(super) fn relay_model_projection(
    _store: &Store,
    provider: &Provider,
    model_id: &str,
    official: Option<&Value>,
    catalog: &models_dev::Catalog,
    global: &model_policy::CompressionPolicy,
) -> (
    model_metadata::ResolvedMetadata,
    model_policy::ClientProjection,
) {
    model_projection(provider, model_id, official, catalog, global)
}

pub(crate) fn aggregate_catalog_entry(
    aggregate: &super::aggregation::AggregateModel,
    projection: &model_policy::ClientProjection,
) -> Value {
    let label = if aggregate.display_name.is_empty() {
        &aggregate.public_model_id
    } else {
        &aggregate.display_name
    };
    let mut entry = relay_catalog_entry(
        aggregate.public_model_id.clone(),
        label.clone(),
        label.clone(),
        projection.context_window,
        projection.output_limit,
        projection.reasoning.clone(),
        projection.default_reasoning.clone(),
    );
    if let Some(threshold) = projection.compression.threshold {
        entry["auto_compact_token_limit"] = serde_json::json!(threshold);
    }
    entry
}

/// Codex client behaviour borrowed from the official entry of the same model.
///
/// `ultra` is a client alias, never a wire value: Codex resolves it to
/// `multi_agent_reasoning_effort`, else `max`, else the highest declared level,
/// and only multi-agent V2 turns it into proactive delegation. Codex also pins a
/// thread's multi-agent version on its first turn, so an entry without one locks
/// the whole thread to the feature default. These keys change what the client
/// does, never what an upstream must accept, so unlike account limits they apply
/// to any endpoint serving the same model.
pub(super) fn adopt_codex_behavior(entry: &mut Value, official: Option<&Value>) {
    let Some(official) = official else { return };
    let Some(version) = official.get("multi_agent_version").and_then(Value::as_str) else {
        return;
    };
    entry["multi_agent_version"] = Value::from(version);
    let declared = |effort: &str| {
        entry["supported_reasoning_levels"]
            .as_array()
            .is_some_and(|levels| levels.iter().any(|level| level["effort"] == effort))
    };
    let preferred = official
        .get("multi_agent_reasoning_effort")
        .and_then(Value::as_str)
        .filter(|effort| *effort != "ultra" && declared(effort))
        .map(str::to_owned);
    // Without declared reasoning there is nothing for Ultra to resolve to.
    let has_levels = entry["supported_reasoning_levels"]
        .as_array()
        .is_some_and(|levels| !levels.is_empty());
    let ultra = (version == "v2" && has_levels && !declared("ultra"))
        .then(|| {
            official["supported_reasoning_levels"]
                .as_array()?
                .iter()
                .find(|level| level["effort"] == "ultra")
                .cloned()
        })
        .flatten();
    if let Some(level) = ultra {
        if let Some(levels) = entry["supported_reasoning_levels"].as_array_mut() {
            levels.push(level);
        }
    }
    if let Some(effort) = preferred {
        entry["multi_agent_reasoning_effort"] = Value::from(effort);
    }
}

fn relay_catalog_entry(
    slug: String,
    display_name: String,
    description: String,
    context_window: u64,
    max_output_tokens: Option<u64>,
    reasoning_levels: Vec<Value>,
    default_reasoning_level: Option<String>,
) -> Value {
    // These fields are required by Codex's model catalog parser. In
    // particular, an entry without instructions is rejected by config/read.
    // The capability declaration comes from the shared resolver, never from a
    // model-name guess.
    serde_json::json!({
        "slug": slug,
        "display_name": display_name,
        "description": description,
        "context_window": context_window,
        "max_output_tokens": max_output_tokens,
        "visibility": "list",
        "supported_in_api": true,
        "priority": 100,
        "supported_reasoning_levels": reasoning_levels,
        "default_reasoning_level": default_reasoning_level,
        "shell_type": "unified_exec",
        "support_verbosity": false,
        "truncation_policy": { "mode": "tokens", "limit": 10000 },
        "experimental_supported_tools": [],
        "base_instructions": "You are a coding assistant. Follow the developer and user instructions, use available tools when needed, and report your changes and verification clearly."
    })
}

fn menu_display_name(provider_name: &str, alias: &str, model_id: &str) -> String {
    let alias = if alias.trim().is_empty() {
        model_id
    } else {
        alias.trim()
    };
    let prefix = format!("{}/", provider_name.trim());
    if alias.starts_with(&prefix) {
        alias.to_owned()
    } else {
        format!("{prefix}{alias}")
    }
}

pub fn apply_routing_config(
    paths: &CodexPaths,
    store: &Store,
) -> Result<newapi::AppliedConfig, String> {
    if store.codex_access_mode == CodexAccessMode::Native {
        return native_config::apply(paths, store);
    }
    subagents::validate(paths, store)?;
    let applied = apply_local_router_config(paths, store)?;
    cleanup_native_credentials(paths);
    Ok(applied)
}

fn cleanup_native_credentials(paths: &CodexPaths) {
    if native_config::cleanup_credentials(paths).is_err() {
        log::warn!("Codex config updated; native credential cleanup will retry on next use");
    }
}

fn apply_local_router_config(
    paths: &CodexPaths,
    store: &Store,
) -> Result<newapi::AppliedConfig, String> {
    use toml_edit::value;
    let _guard = config_snapshot::guard().map_err(config_update_error)?;
    if store.router.bearer_token.is_empty() {
        return Err("newapi.errors.storage".into());
    }
    let (provider_key, default_model, namespace) = codex_connection(store)?;
    let catalog = selected_catalog_path(paths, store);
    let menu = if store.router.aggregate_model_id.is_some() {
        aggregate_catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    } else {
        catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    };
    let mut pending = None;
    let result = CodexConfigRepository::new(paths)
        .update(|original| {
            let mut document = original
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| CoreError::InvalidData("Invalid Codex config TOML".into()))?;
            if document
                .get("model_providers")
                .is_some_and(|item| !item.is_table_like())
            {
                return Err(CoreError::InvalidData(
                    "model_providers must be a TOML table".into(),
                ));
            }
            let mut snapshot = match config_snapshot::load(paths, original)? {
                Some(snapshot) if snapshot.selected_provider_is_ours(&document) => snapshot,
                // An explicit enable after cc-switch took over starts a new
                // session, with cc-switch's live settings as its baseline.
                Some(snapshot) => {
                    snapshot.restore_subagents(&mut document)?;
                    config_snapshot::Snapshot::capture(paths, &document)?
                }
                None if model_provider_is_ours(&document) => {
                    config_snapshot::recover_legacy(paths, &document, store)?
                }
                None => {
                    reject_foreign_managed_provider_entries(&document)?;
                    // A lost sidecar must not record our own URL as the baseline.
                    if router_base_is_ours(&document) {
                        document.remove("openai_base_url");
                    }
                    config_snapshot::Snapshot::capture(paths, &document)?
                }
            };
            // Refreshing a card/catalog must retain the conversation's valid
            // selection. Removing that model falls back to the configured default.
            let selected = document.get("model")
                .and_then(toml_edit::Item::as_str)
                .filter(|selected| {
                    store.router.aggregate_model_id.is_none()
                        && route_is_ours(&document)
                        && menu_contains(&menu, selected)
                })
                .unwrap_or(&default_model)
                .to_owned();
            remove_unreferenced_provider_entries(paths, &mut document)?;
            config_snapshot::set_value(
                &mut document,
                "model",
                Some(selected.into()));
            config_snapshot::set_value(
                &mut document,
                "model_catalog_json",
                Some(catalog.to_string_lossy().to_string().into()),
            );
            // Context and compaction follow the selected catalog model, not the
            // provider that happened to be active when Router was enabled.
            for key in ["model_context_window", "model_auto_compact_token_limit"] {
                config_snapshot::set_value(&mut document, key, None);
            }
            if !store.router.accountless {
                // Every thread keeps Codex's built-in `openai` provider; only its
                // endpoint changes. Toggling routing never migrates threads.
                if !builtin_provider_selected(&document) {
                    // Prefer the user's own value when it already was built-in.
                    snapshot.restore_key(&mut document, "model_provider")?;
                }
                if !builtin_provider_selected(&document) {
                    config_snapshot::set_value(&mut document, "model_provider", Some("openai".into()));
                }
                config_snapshot::set_value(
                    &mut document,
                    "openai_base_url",
                    Some(builtin_router_base(store).into()),
                );
                snapshot.apply_auth(&mut document, false)?;
                snapshot.apply_subagents(&mut document, store.codex_subagents.as_ref())?;
                write_catalog(paths, store).map_err(CoreError::OperationFailed)?;
                snapshot.prepare(paths, original, &document, config_snapshot::Action::Apply)?;
                pending = Some(snapshot);
                return Ok(document.to_string());
            }
            // Without a ChatGPT login the built-in provider is unavailable, so
            // accountless mode keeps a dedicated provider.
            config_snapshot::set_value(&mut document, "openai_base_url", None);
            config_snapshot::set_value(
                &mut document,
                "model_provider",
                Some(provider_key.into()));
            if document.get("model_providers").is_none() {
                let mut providers = toml_edit::Table::new();
                providers.set_implicit(true);
                document["model_providers"] = toml_edit::Item::Table(providers);
            }
            let inline_providers = document["model_providers"]
                .as_value()
                .is_some_and(toml_edit::Value::is_inline_table);
            let entry = &mut document["model_providers"][provider_key];
            *entry = if inline_providers {
                toml_edit::Item::Value(toml_edit::Value::InlineTable(toml_edit::InlineTable::new()))
            } else {
                toml_edit::Item::Table(toml_edit::Table::new())
            };
            entry["name"] = value("AgentHarbor 智能路由");
            entry["base_url"] = value(format!(
                "http://127.0.0.1:{}/{namespace}/v1", store.router.port));
            entry["wire_api"] = value("responses");
            // Keep Codex's first-party auth pipeline (account display and token
            // refresh) independent of authentication to this loopback router.
            entry["requires_openai_auth"] = value(!store.router.accountless);
            let mut headers = toml_edit::InlineTable::new();
            headers.insert(LOCAL_AUTH_HEADER, store.router.bearer_token.clone().into());
            entry["http_headers"] = value(headers);
            if store.router.accountless {
                entry["experimental_bearer_token"] = value(&store.router.bearer_token);
            }
            snapshot.apply_auth(&mut document, store.router.accountless)?;
            snapshot.apply_subagents(&mut document, store.codex_subagents.as_ref())?;
            write_catalog(paths, store).map_err(CoreError::OperationFailed)?;
            snapshot.prepare(paths, original, &document, config_snapshot::Action::Apply)?;
            pending = Some(snapshot);
            Ok(document.to_string())
        })
        .map_err(config_update_error)?;
    if let Some(snapshot) = pending {
        snapshot.finish(paths);
    }
    Ok(applied_config(result))
}

fn codex_connection(store: &Store) -> Result<(&'static str, String, &'static str), String> {
    if let Some(id) = &store.router.aggregate_model_id {
        let aggregate = store
            .aggregates
            .iter()
            .find(|aggregate| aggregate.id == *id && aggregate.enabled)
            .ok_or("aggregation.errors.unavailable")?;
        Ok((
            AGGREGATE_PROVIDER_KEY,
            aggregate.public_model_id.clone(),
            "aggregate",
        ))
    } else {
        let provider = effective_active_for(store, ModelProtocol::Responses)
            .ok_or("newapi.errors.noActiveProvider")?;
        Ok((
            CATALOG_PROVIDER_KEY,
            format!("{}::{}", provider.id, provider.model),
            "codex",
        ))
    }
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
    if store.codex_access_mode == CodexAccessMode::Native {
        return if native_config::intact(paths, store)? {
            Ok(None)
        } else {
            native_config::apply(paths, store).map(Some)
        };
    }
    if routing_config_matches(paths, store)? && config_snapshot::path(paths).is_file() {
        Ok(None)
    } else {
        apply_routing_config(paths, store).map(Some)
    }
}

/// Read-only validation shared by config repair and one-time thread adoption.
pub(crate) fn routing_config_matches(paths: &CodexPaths, store: &Store) -> Result<bool, String> {
    if !store.router.enabled
        || store.codex_access_mode != CodexAccessMode::Router
        || store.router.bearer_token.is_empty()
    {
        return Ok(false);
    }
    let (provider_key, expected_model, namespace) = codex_connection(store)?;
    let (original, document) = open_config(paths)?;
    let ownership_matches = config_snapshot::load(paths, original.as_deref().unwrap_or(""))
        .map_err(config_update_error)?
        .is_some_and(|snapshot| snapshot.subagents_match(store.codex_subagents.as_ref()));
    let expected_base = format!("http://127.0.0.1:{}/{namespace}/v1", store.router.port);
    let expected_catalog = selected_catalog_path(paths, store).to_string_lossy().to_string();
    let expected_menu = if store.router.aggregate_model_id.is_some() {
        aggregate_catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    } else {
        catalog_value(&paths.app_data_dir, &paths.codex_home, store)
    };
    // Codex can persist a model selected during a conversation. A valid menu
    // choice (official or relay) does not invalidate its provider. Explicit
    // aggregate selection retains its existing configured-default behavior.
    let selected_model_valid = document
        .get("model")
        .and_then(toml_edit::Item::as_str)
        .is_some_and(|selected| {
            if store.router.aggregate_model_id.is_some() {
                selected == expected_model
            } else {
                menu_contains(&expected_menu, selected)
            }
        });
    let catalog_current = std::fs::read(selected_catalog_path(paths, store))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|catalog| catalog == expected_menu);
    let shared = selected_model_valid
        && document.get("model_context_window").is_none()
        && document.get("model_auto_compact_token_limit").is_none()
        && document
            .get("model_catalog_json")
            .and_then(toml_edit::Item::as_str)
            == Some(expected_catalog.as_str())
        && catalog_current
        && ownership_matches
        && subagents::matches(&document, store);
    if !store.router.accountless {
        return Ok(shared
            && document.get("openai_base_url").and_then(toml_edit::Item::as_str)
                == Some(builtin_router_base(store).as_str())
            && builtin_provider_selected(&document));
    }
    let provider = document
        .get("model_providers")
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|providers| providers.get(provider_key))
        .and_then(toml_edit::Item::as_table_like);
    let current = shared && !router_base_is_ours(&document) && provider.is_some_and(|provider| {
        provider.get("base_url").and_then(toml_edit::Item::as_str) == Some(expected_base.as_str())
            && provider.get("wire_api").and_then(toml_edit::Item::as_str) == Some("responses")
            && provider
                .get("requires_openai_auth")
                .and_then(toml_edit::Item::as_bool)
                == Some(!store.router.accountless)
            && provider
                .get("http_headers")
                .and_then(|headers| headers.get(LOCAL_AUTH_HEADER))
                .and_then(toml_edit::Item::as_str)
                == Some(store.router.bearer_token.as_str())
            && if store.router.accountless {
                provider
                    .get("experimental_bearer_token")
                    .and_then(toml_edit::Item::as_str)
                    == Some(store.router.bearer_token.as_str())
                    && document
                        .get("preferred_auth_method")
                        .and_then(toml_edit::Item::as_str)
                        == Some("apikey")
            } else {
                ["experimental_bearer_token", "env_key", "auth", "aws"]
                    .iter()
                    .all(|key| provider.get(key).is_none())
            }
    }) && document
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        == Some(provider_key);
    Ok(current)
}

fn menu_contains(menu: &Value, slug: &str) -> bool {
    menu["models"].as_array()
        .is_some_and(|models| models.iter().any(|model| model["slug"] == slug))
}

pub fn remove_routing_config(
    paths: &CodexPaths,
    store: &Store,
) -> Result<Option<newapi::AppliedConfig>, String> {
    let had_snapshot = config_snapshot::path(paths).is_file();
    let applied = remove_connection_config(paths, store)?;
    if had_snapshot || applied.is_some() {
        cleanup_native_credentials(paths);
    }
    Ok(applied)
}

fn remove_connection_config(
    paths: &CodexPaths,
    store: &Store,
) -> Result<Option<newapi::AppliedConfig>, String> {
    let _guard = config_snapshot::guard().map_err(config_update_error)?;
    // No journal means no owned settings to restore. Only legacy installs
    // whose live provider is still ours require the recovery path below.
    if !config_snapshot::path(paths).is_file() {
        let (_, document) = match open_config(paths) {
            Ok(config) => config,
            Err(_) => return Ok(None),
        };
        if !route_is_ours(&document) {
            return Ok(None);
        }
    }
    if !paths.config_path.exists() {
        if config_snapshot::path(paths).is_file() {
            config_snapshot::discard(paths).map_err(config_update_error)?;
            let _ = std::fs::remove_file(catalog_path(paths));
        }
        return Ok(None);
    }
    let mut pending = None;
    let mut keep_catalog = false;
    let result = CodexConfigRepository::new(paths)
        .update(|original| {
            let mut document = original
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| CoreError::InvalidData("Invalid Codex config TOML".into()))?;
            if document
                .get("model_providers")
                .is_some_and(|item| !item.is_table_like())
            {
                return Err(CoreError::InvalidData(
                    "model_providers must be a TOML table".into(),
                ));
            }
            let snapshot = match config_snapshot::load(paths, original)? {
                Some(snapshot) => Some(snapshot),
                None if model_provider_is_ours(&document) => {
                    Some(config_snapshot::recover_legacy(paths, &document, store)?)
                }
                None => None,
            };
            let Some(mut snapshot) = snapshot else {
                // Without ownership records, release only our own endpoint and
                // the relay-only selections that cannot work without it.
                if !router_base_is_ours(&document) {
                    return Ok(original.to_owned());
                }
                document.remove("openai_base_url");
                if document.get("model").and_then(toml_edit::Item::as_str)
                    .is_some_and(|model| model.contains("::")) {
                    document.remove("model");
                }
                let ours = [catalog_path(paths), aggregate_catalog_path(paths)];
                if document.get("model_catalog_json").and_then(toml_edit::Item::as_str)
                    .is_some_and(|path| ours.iter().any(|own| own.to_string_lossy() == path)) {
                    document.remove("model_catalog_json");
                }
                return Ok(document.to_string());
            };
            remove_managed_provider_entries(&mut document);
            snapshot.restore(paths, &mut document)?;
            // After a takeover our endpoint may remain; it would send official
            // threads to a router that is going away.
            if router_base_is_ours(&document) {
                snapshot.restore_key(&mut document, "openai_base_url")?;
            }
            keep_catalog = document
                .get("model_catalog_json")
                .and_then(toml_edit::Item::as_str)
                == Some(catalog_path(paths).to_string_lossy().as_ref());
            snapshot.prepare(paths, original, &document, config_snapshot::Action::Remove)?;
            pending = Some(snapshot);
            Ok(document.to_string())
        })
        .map_err(config_update_error)?;
    let Some(snapshot) = pending else {
        return Ok(None);
    };
    snapshot.finish(paths);
    if !keep_catalog {
        let _ = std::fs::remove_file(catalog_path(paths));
    }
    Ok(Some(applied_config(result)))
}

#[derive(Deserialize)]
struct ExportedProvider {
    #[serde(default, alias = "priceMultiplier")]
    price_multiplier: Option<f64>,
    name: String,
    #[serde(alias = "baseUrl")]
    base_url: String,
    #[serde(default)]
    user_id: Option<u64>,
    /// Absent in older exports, where the relay's own capabilities decide.
    #[serde(default)]
    protocol: Option<ModelProtocol>,
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
        if entry
            .price_multiplier
            .is_some_and(|factor| !model_metadata::valid_price_multiplier(factor))
        {
            skipped += 1;
            continue;
        }
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
            // An export that predates the protocol field, or one from a relay that
            // never declared it, is served by inferring from the models it listed
            // instead of silently assuming the Codex-specific Responses protocol.
            protocol: entry
                .protocol
                .unwrap_or_else(|| protocol::infer_protocol_from_models(&models, &entry.model)),
            model: entry.model,
            models,
            context_window: entry.context_window,
            auto_compact_limit: entry.auto_compact_limit,
            price_multiplier: entry.price_multiplier.filter(|factor| *factor != 1.0),
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
        state: state_from_store_at(&store, super::router_proxy::is_running(), Some(paths)),
        imported,
        skipped,
    })
}

pub fn export_providers(paths: &CodexPaths, path: &Path) -> Result<(), String> {
    let store = load_store(paths)?;
    let providers: Vec<Value> = store
        .providers
        .iter()
        .filter(|p| !p.router_only())
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
                "priceMultiplier": p.price_multiplier,
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

    #[test]
    fn account_model_refresh_updates_only_the_injected_catalog() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        std::fs::write(&paths.auth_path, br#"{"tokens":{"account_id":"current-account"}}"#).unwrap();
        // This is a signed-in Codex refresh; a cache without account identity
        // is now capability metadata only.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let cache = paths.codex_home.join("models_cache.json");
        std::fs::write(&cache, r#"{"models":[{"slug":"first"}]}"#).unwrap();
        let store = Store::default();
        let catalog = write_catalog(&paths, &store).unwrap();
        std::fs::write(&paths.config_path, "model_provider = 'external'\n").unwrap();
        std::fs::write(&cache, r#"{"models":[{"slug":"second"}]}"#).unwrap();
        refresh_catalog_if_changed(&paths, &store).unwrap();
        let refreshed: Value = serde_json::from_slice(&std::fs::read(catalog).unwrap()).unwrap();
        assert_eq!(refreshed["models"][0]["slug"], "second");
        assert_eq!(refreshed["models"].as_array().unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(&paths.config_path).unwrap(),
            "model_provider = 'external'\n"
        );
    }

    #[test]
    fn injected_menu_lists_only_the_active_accounts_fetched_official_models() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let account_key = |id: &str| {
            use sha2::Digest;
            format!("{:x}", sha2::Sha256::digest(id.as_bytes()))
        };
        let login = |id: &str| {
            std::fs::write(
                &paths.auth_path,
                serde_json::json!({"auth_mode":"chatgpt","tokens":{"account_id":id}}).to_string(),
            )
            .unwrap();
        };
        // A stale global cache from an earlier, uninjected Plus session.
        std::fs::write(
            paths.codex_home.join("models_cache.json"),
            r#"{"models":[{"slug":"plus-cache-only"}]}"#,
        )
        .unwrap();
        login("plus");
        for (id, slug) in [("plus", "plus-model"), ("free", "free-model")] {
            crate::core::model_catalog::store_account_catalog(
                &paths.app_data_dir,
                &account_key(id),
                format!(r#"{{"models":[{{"slug":"{slug}"}}]}}"#).as_bytes(),
            )
            .unwrap();
        }
        let mut store = Store::default();
        store.providers.push(Provider {
            id: "relay".into(),
            name: "Relay".into(),
            enabled: true,
            api_key: "synthetic".into(),
            base_url: "https://relay.example".into(),
            model: "gpt-relay".into(),
            ..Default::default()
        });
        let slugs = |store: &Store| -> Vec<String> {
            catalog_value(&paths.app_data_dir, &paths.codex_home, store)["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["slug"].as_str().unwrap().to_owned())
                .collect()
        };
        for (id, official) in [("free", "free-model"), ("plus", "plus-model")] {
            crate::core::model_catalog::record_account_switch(
                &paths.codex_home,
                &paths.app_data_dir,
                &account_key(id),
            )
            .unwrap();
            login(id);
            assert_eq!(slugs(&store), [official, "relay::gpt-relay"]);
        }
    }

    fn input(name: &str, base_url: &str) -> ProviderInput {
        ProviderInput {
            gateway_slug: None,
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
            catalog_providers: None,
            price_catalog_provider: None,
            price_multiplier: None,
            compression: None,
            network_mode: "proxy".into(),
        }
    }

    #[test]
    fn command_search_capability_is_independent_of_model_protocol() {
        for protocol in [ModelProtocol::Responses, ModelProtocol::ChatCompletions, ModelProtocol::AnthropicMessages,
        ] {
            for suffix in ["", "/", "/provider", "/provider/", "/provider/v1", "/provider/v1/",
            ] {
                let provider = Provider { base_url: format!("https://api.commandcode.ai{suffix}"),
                    api_key: "search-fixture".into(), protocol, ..Default::default() };
                assert!(supports_command_search(&provider));
                assert!(provider.view().web_search_capable);
            }
        }
        for base_url in ["http://api.commandcode.ai", "https://api.commandcode.ai.invalid", "https://api.commandcode.ai:8443",
            "https://user@api.commandcode.ai", "https://api.commandcode.ai/provider/v1?secret=1", "https://api.commandcode.ai/#fragment", "https://api.commandcode.ai/v1",
        ] {
            let provider = Provider { base_url: base_url.into(), api_key: "search-fixture".into(), ..Default::default() };
            assert!(!supports_command_search(&provider), "{base_url}");
        }
        assert!(!supports_command_search(&Provider { base_url: "https://api.commandcode.ai".into(), ..Default::default() }));
    }

    #[test]
    fn search_provider_selection_persists_without_reapplying_model_settings() {
        let (_dir, paths) = paths();
        let mut draft = input("Search service", "https://api.commandcode.ai/provider/v1");
        draft.protocol = ModelProtocol::ChatCompletions;
        let state = save_provider(&paths, draft).unwrap();
        let id = state.state.providers[0].id.clone();
        let selected = set_web_search_provider(&paths, Some(id.clone())).unwrap();
        assert_eq!(selected.state.router.web_search_provider_id.as_deref(), Some(id.as_str()));
        assert!(selected.applied.is_none());
        assert_eq!(load_store(&paths).unwrap().router.web_search_provider_id.as_deref(), Some(id.as_str()));
        assert_eq!(set_web_search_provider(&paths, Some("missing".into())).err().as_deref(), Some("newapi.errors.notFound"));
        let other = save_provider(&paths, input("Other", "https://relay.example.com")).unwrap();
        let other_id = other.state.providers.iter().find(|provider| provider.name == "Other").unwrap().id.clone();
        assert_eq!(set_web_search_provider(&paths, Some(other_id)).err().as_deref(), Some("newapi.errors.webSearchProviderUnsupported"));
        assert_eq!(load_store(&paths).unwrap().router.web_search_provider_id.as_deref(), Some(id.as_str()));
        // An explicit selection must not silently switch the search billing account.
        delete_provider(&paths, &id).unwrap();
        assert_eq!(load_store(&paths).unwrap().router.web_search_provider_id.as_deref(), Some(id.as_str()));
        assert!(set_web_search_provider(&paths, None).unwrap().state.router.web_search_provider_id.is_none());
    }

    #[test]
    fn legacy_router_settings_default_to_automatic_search() {
        let router: RouterSettings = serde_json::from_str(r#"{"enabled":false,"port":43127}"#).unwrap();
        assert!(router.web_search_provider_id.is_none());
    }

    #[test]
    fn user_price_overrides_win_over_the_catalog_and_clear_cleanly() {
        let (_dir, paths) = paths();
        let mut draft = input("Relay", "https://relay.example.com");
        draft.model = "gpt-5.2".into();
        draft.price_multiplier = Some(2.0);
        draft.models = vec![newapi::Model {
            id: "gpt-5.2".into(),
            display_name: "GPT-5.2".into(),
            included: true,
            ..Default::default()
        }];
        let state = save_provider(&paths, draft).unwrap();
        let provider_id = state.state.providers[0].id.clone();
        let store = load_store(&paths).unwrap();
        let catalog = models_dev::load(&paths.app_data_dir);
        // Without an override the catalog price applies (or stays unknown).
        let before = model_projection(
            &store.providers[0],
            "gpt-5.2",
            None,
            &catalog,
            &store.global_compression(),
        );
        assert_ne!(before.0.price.source, "manual");

        set_model_price_override(
            &paths,
            &provider_id,
            "gpt-5.2",
            Some(newapi::PriceOverrides {
                input: Some(1.5),
                output: Some(7.5),
                cache_read: Some(0.15),
                cache_write: None,
            }),
        )
        .unwrap();
        let store = load_store(&paths).unwrap();
        let after = model_projection(
            &store.providers[0],
            "gpt-5.2",
            None,
            &catalog,
            &store.global_compression(),
        );
        assert_eq!(after.0.price.source, "manual");
        assert_eq!(after.0.price.input, Some(1.5));
        assert_eq!(after.0.price.output, Some(7.5));
        assert_eq!(after.0.price.cache_read, Some(0.15));
        assert_eq!(
            after.0.price.multiplier, None,
            "custom prices are already final"
        );
        assert_eq!(after.0.price.complete, true);

        // Clearing restores the catalog price.
        set_model_price_override(&paths, &provider_id, "gpt-5.2", None).unwrap();
        let store = load_store(&paths).unwrap();
        assert!(store.providers[0].models[0].overrides.price.is_none());
        let cleared = model_projection(
            &store.providers[0],
            "gpt-5.2",
            None,
            &catalog,
            &store.global_compression(),
        );
        assert_ne!(cleared.0.price.source, "manual");

        // Negative or absurd rates are rejected.
        assert!(set_model_price_override(
            &paths,
            &provider_id,
            "gpt-5.2",
            Some(newapi::PriceOverrides {
                input: Some(-1.0),
                ..Default::default()
            }),
        )
        .is_err());
    }

    #[test]
    fn catalog_references_are_normalized_or_dropped() {
        let normalized = sanitized_ref(Some(models_dev::ModelRef {
            provider_id: " Anthropic ".into(),
            model_id: " claude-opus-4-5 ".into(),
        }))
        .unwrap();
        assert_eq!(normalized.provider_id, "anthropic");
        assert_eq!(normalized.model_id, "claude-opus-4-5");
        for invalid in [
            models_dev::ModelRef {
                provider_id: String::new(),
                model_id: "x".into(),
            },
            models_dev::ModelRef {
                provider_id: "p".into(),
                model_id: String::new(),
            },
            models_dev::ModelRef {
                provider_id: "p".into(),
                model_id: "x".repeat(300),
            },
            models_dev::ModelRef {
                provider_id: "p".into(),
                model_id: "bad\nmodel".into(),
            },
        ] {
            assert!(sanitized_ref(Some(invalid)).is_none());
        }
        assert!(sanitized_ref(None).is_none());
    }

    #[test]
    fn routing_config_intact_detects_another_tool_taking_over() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        let repository = CodexConfigRepository::new(&paths);

        // A missing file cannot be an active route.
        assert!(!routing_config_intact(&paths));

        // Our own injection reads as intact.
        repository
            .update(|_| {
                Ok(r#"
model = "p1::model-a"
model_provider = "agentharbor_relay"
model_catalog_json = 'catalog.json'

[model_providers.agentharbor_relay]
name = "AgentHarbor"
base_url = "http://127.0.0.1:43127/codex/v1"
"#
                .replace("catalog.json", &catalog_path(&paths).to_string_lossy()))
            })
            .unwrap();
        assert!(routing_config_intact(&paths));

        // A different tool replacing the pointer is exactly the reported failure.
        repository
            .update(|original| {
                let replaced = original.replace("agentharbor_relay", "aimai1").replace(
                    &format!(
                        "model_catalog_json = '{}'\n",
                        catalog_path(&paths).display()
                    ),
                    "",
                );
                Ok(replaced)
            })
            .unwrap();
        assert!(!routing_config_intact(&paths));
    }

    #[test]
    fn routing_config_intact_treats_unreadable_config_as_drift() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(&paths.config_path, "this is not = valid toml [[[").unwrap();
        assert!(!routing_config_intact(&paths));
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
            aggregates: vec![],
            codex_access_mode: CodexAccessMode::Router,
            codex_subagents: None,
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
            "model unavailable"
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
    fn provider_price_multipliers_roundtrip_and_validate_without_touching_credentials() {
        let (_dir, paths) = paths();
        let mut draft = input("Relay", "https://relay.example.com");
        draft.price_multiplier = Some(0.5);
        let saved = save_provider(&paths, draft).unwrap();
        let id = saved.state.providers[0].id.clone();
        assert_eq!(saved.state.providers[0].price_multiplier, Some(0.5));
        let edit = |factor| {
            let mut draft = input("Relay", "https://relay.example.com");
            draft.id = Some(id.clone());
            draft.price_multiplier = factor;
            draft.api_key.clear();
            draft
        };
        save_provider(&paths, edit(None)).unwrap(); // Older callers omit the field.
        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers[0].price_multiplier, Some(0.5));
        assert_eq!(store.providers[0].api_key, "sk-a");
        let exported = paths.app_data_dir.join("export.json");
        export_providers(&paths, &exported).unwrap();
        let (_other_dir, other_paths) = self::paths();
        let imported = import_providers(&other_paths, &exported).unwrap();
        assert_eq!(imported.imported, 1);
        assert_eq!(imported.state.providers[0].price_multiplier, Some(0.5));
        for factor in [0.0, 1.0, 1000.0] {
            save_provider(&paths, edit(Some(factor))).unwrap();
            assert_eq!(
                load_store(&paths).unwrap().providers[0].price_multiplier,
                (factor != 1.0).then_some(factor)
            );
        }
        for factor in [-1.0, 1000.1, f64::INFINITY, f64::NAN] {
            assert_eq!(
                save_provider(&paths, edit(Some(factor))).err().as_deref(),
                Some("newapi.errors.priceMultiplier")
            );
        }
        let invalid = paths.app_data_dir.join("invalid.json");
        std::fs::write(
            &invalid,
            r#"[{"name":"Bad","baseUrl":"https://bad.example.com","priceMultiplier":-1}]"#,
        )
        .unwrap();
        assert_eq!(import_providers(&other_paths, &invalid).unwrap().skipped, 1);
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
                protocol: None,
                context_window: Some(200_000),
                max_output_tokens: Some(16_384),
                endpoint_types: vec!["openai-response".into()],
                included: true,
                metadata_source: "manual".into(),
                supported_reasoning_levels: Some(vec![newapi::ReasoningLevel {
                    effort: "low".into(),
                    description: "Provider low".into(),
                }]),
                default_reasoning_level: Some("low".into()),
                ..Default::default()
            },
            newapi::Model {
                id: "model-deep".into(),
                display_name: "Deep reasoning".into(),
                protocol: None,
                context_window: Some(400_000),
                max_output_tokens: Some(128_000),
                endpoint_types: vec!["openai-response".into()],
                included: true,
                metadata_source: "upstream".into(),
                supported_reasoning_levels: Some(vec![]),
                default_reasoning_level: None,
                ..Default::default()
            },
        ];

        let state = save_provider(&paths, draft).unwrap();
        assert_eq!(state.state.providers[0].models.len(), 2);

        let provider = &load_store(&paths).unwrap().providers[0];
        assert_eq!(provider.model, "model-fast");
        assert_eq!(provider.models.len(), 2);
        assert_eq!(
            provider.models[0]
                .supported_reasoning_levels
                .as_ref()
                .unwrap()[0]
                .effort,
            "low"
        );
        assert_eq!(
            provider.models[0].default_reasoning_level.as_deref(),
            Some("low")
        );
        assert_eq!(provider.models[1].supported_reasoning_levels, Some(vec![]));
        assert_eq!(provider.models[1].display_name, "Deep reasoning");
        assert_eq!(provider.models[1].context_window, Some(400_000));
        assert!(provider.models.iter().all(|model| model.included));
    }

    #[test]
    fn cleared_model_defaults_and_pending_models_survive_save_and_reload() {
        let (_dir, paths) = paths();
        let mut draft = input("Relay", "https://relay.invalid");
        let created = save_provider(&paths, input("Relay", "https://relay.invalid")).unwrap();
        draft.id = Some(created.state.providers[0].id.clone());
        draft.model.clear();
        draft.models = vec![newapi::Model {
            id: "pending-model".into(),
            display_name: "Pending".into(),
            included: false,
            ..Default::default()
        }];
        save_provider(&paths, draft).unwrap();
        let store = load_store(&paths).unwrap();
        assert!(store.providers[0].model.is_empty());
        assert_eq!(store.providers[0].models.len(), 1);
        assert!(!store.providers[0].models[0].included);
        let mut draft = input("Relay", "https://relay.invalid");
        draft.id = Some(created.state.providers[0].id.clone());
        draft.model.clear();
        save_provider(&paths, draft).unwrap();
        let store = load_store(&paths).unwrap();
        assert!(store.providers[0].model.is_empty());
        assert!(store.providers[0].models.is_empty());
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
        // Auto-filled legacy values are cleared: the resolver derives them at
        // read time instead of freezing a guess.
        assert_eq!(model.context_window, None);
        assert_eq!(model.max_output_tokens, None);
        // The record keeps no declaration of its own; nothing pretends the
        // model has a known window.
        assert_eq!(model.metadata_source, "");
    }

    #[test]
    fn current_store_migrates_manual_overrides_and_drops_guessed_values() {
        let (_dir, paths) = paths();
        let models = [
            ("claude-haiku-4-5", "local"),
            ("claude-sonnet-4", "upstream"),
            ("claude-opus-4-8", "manual"),
            ("gpt-6-luna", "upstream"),
        ]
        .into_iter()
        .map(|(id, source)| newapi::Model {
            id: id.into(),
            context_window: Some(200_000),
            max_output_tokens: Some(4_096),
            metadata_source: source.into(),
            ..Default::default()
        })
        .collect();
        let store = Store {
            version: 7,
            codex_access_mode: CodexAccessMode::Router,
            providers: vec![Provider {
                id: "relay".into(),
                models,
                ..Default::default()
            }],
            ..Default::default()
        };
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&store).unwrap()).unwrap();
        let loaded = load_store(&paths).unwrap();
        let models = &loaded.providers[0].models;
        // Auto-filled values never survive as saved data.
        assert_eq!(models[0].context_window, None);
        assert_eq!(models[0].metadata_source, "local");
        // Historical upstream values may have been overwritten by the removed
        // local defaults, so they are demoted to legacy.
        assert_eq!(models[1].metadata_source, "legacy");
        assert_eq!(models[3].metadata_source, "legacy");
        // Explicit user edits become per-field overrides.
        assert_eq!(models[2].metadata_source, "manual");
        assert_eq!(models[2].overrides.context_window, Some(200_000));
        assert_eq!(models[2].overrides.max_output_tokens, Some(4_096));
        // Migration is idempotent: a second load changes nothing.
        let again = load_store(&paths).unwrap();
        assert_eq!(again.providers[0].models, loaded.providers[0].models);
    }

    #[test]
    fn saved_local_guess_is_ignored_and_read_time_resolution_is_used() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.codex_home).unwrap();
        std::fs::write(
            paths.codex_home.join("models_cache.json"),
            r#"{"models":[{"slug":"gpt-6-luna","context_window":272000}]}"#,
        )
        .unwrap();
        let store = Store {
            version: 7,
            codex_access_mode: CodexAccessMode::Router,
            providers: vec![Provider {
                id: "relay".into(),
                name: "Relay".into(),
                base_url: "https://chatgpt.com/backend-api/codex".into(),
                api_key: "sk-fixture".into(),
                model: "gpt-6-luna".into(),
                enabled: true,
                models: vec![newapi::Model {
                    id: "gpt-6-luna".into(),
                    display_name: "Upstream display name".into(),
                    context_window: Some(400_000),
                    max_output_tokens: Some(128_000),
                    metadata_source: "local".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            compression: Default::default(),
            ..Default::default()
        };
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&store).unwrap()).unwrap();
        let repaired = load_store(&paths).unwrap();
        assert_eq!(repaired.version, STORE_VERSION);
        // The guessed value is dropped rather than republished as saved data.
        assert_eq!(repaired.providers[0].models[0].context_window, None);
        // The account snapshot is only consulted for the official endpoint and
        // feeds the shared projection, not the stored model record.
        let catalog = catalog_value(&paths.app_data_dir, &paths.codex_home, &repaired);
        let entry = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "relay::gpt-6-luna")
            .unwrap();
        assert_eq!(entry["context_window"], 272_000);
        assert_eq!(
            repaired.providers[0].models[0].display_name,
            "Upstream display name"
        );
    }

    #[test]
    fn previous_router_identity_moves_to_the_isolated_port_and_namespace() {
        let (_dir, paths) = paths();
        let store = Store {
            version: 1,
            aggregates: vec![],
            codex_access_mode: CodexAccessMode::Router,
            codex_subagents: None,
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
            compression: Default::default(),
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
            aggregates: vec![],
            codex_access_mode: CodexAccessMode::Router,
            codex_subagents: None,
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
            compression: Default::default(),
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
        assert!(config["openai_base_url"]
            .as_str()
            .is_some_and(|url| url.ends_with("/openai/agentharbor_test_token/v1")));
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
        assert!(text.contains(&format!(
            "http://127.0.0.1:43127/openai/{}/v1",
            store.router.bearer_token
        )));
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
            aggregates: vec![],
            codex_access_mode: CodexAccessMode::Router,
            codex_subagents: None,
            providers: vec![provider],
            router: RouterSettings {
                enabled: true,
                bearer_token: "codexharbor_fixture_token".into(),
                port: PREVIOUS_DEFAULT_PORT,
                ..Default::default()
            },
            active_provider_id: Some("legacy-chat".into()),
            compression: Default::default(),
        };
        let mut value = serde_json::to_value(&store).unwrap();
        value["providers"][0]["wireApi"] = serde_json::json!("chat");
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::create_dir_all(paths.config_path.parent().unwrap()).unwrap();
        let original = "model = 'original-model'\nmodel_provider = 'original-provider'\npreferred_auth_method = 'chatgpt'\n[model_providers.original-provider]\nname = 'Original'\n[mcp_servers.tools]\ncommand = 'node'\n";
        std::fs::write(&paths.config_path, original).unwrap();
        write_catalog(&paths, &store).unwrap();
        CodexConfigRepository::new(&paths)
            .update(|original| {
                let mut document = original.parse::<toml_edit::DocumentMut>().unwrap();
                document["model"] = toml_edit::value("legacy-chat::chat-model");
                document["model_provider"] = toml_edit::value(PREVIOUS_CATALOG_PROVIDER_KEY);
                document["model_catalog_json"] =
                    toml_edit::value(catalog_path(&paths).to_string_lossy().to_string());
                let provider = &mut document["model_providers"][PREVIOUS_CATALOG_PROVIDER_KEY];
                *provider = toml_edit::Item::Table(toml_edit::Table::new());
                provider["base_url"] =
                    toml_edit::value(format!("http://127.0.0.1:{}/codex/v1", store.router.port));
                provider["wire_api"] = toml_edit::value("chat");
                provider["requires_openai_auth"] = toml_edit::value(false);
                provider["experimental_bearer_token"] =
                    toml_edit::value(&store.router.bearer_token);
                Ok(document.to_string())
            })
            .unwrap();

        let injected_before = std::fs::read(&paths.config_path).unwrap();
        let migrated = load_store(&paths).unwrap();
        assert!(migrated.providers[0].enabled);
        assert_eq!(
            migrated.providers[0].protocol,
            ModelProtocol::ChatCompletions
        );
        assert!(!migrated.router.enabled);
        assert_eq!(migrated.active_provider_id.as_deref(), Some("legacy-chat"));
        assert!(migrated.router.bearer_token.starts_with("agentharbor_"));
        assert_eq!(migrated.router.port, DEFAULT_PORT);
        assert_eq!(std::fs::read(&paths.config_path).unwrap(), injected_before);
    }

    #[test]
    fn unproven_legacy_chat_migration_only_changes_the_store() {
        let (_dir, paths) = paths();
        let store = Store {
            version: 1,
            aggregates: vec![],
            codex_access_mode: CodexAccessMode::Router,
            codex_subagents: None,
            providers: vec![Provider {
                id: "legacy-chat".into(),
                name: "Legacy Chat".into(),
                base_url: "https://chat.example.com/".into(),
                api_key: "fixture-chat-key".into(),
                model: "chat-model".into(),
                enabled: true,
                ..Default::default()
            }],
            router: RouterSettings {
                enabled: true,
                bearer_token: "old-router-token".into(),
                ..Default::default()
            },
            active_provider_id: Some("legacy-chat".into()),
            compression: Default::default(),
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

        let config_before = std::fs::read(&paths.config_path).unwrap();
        let migrated = load_store(&paths).unwrap();
        assert!(!migrated.router.enabled);
        assert_eq!(std::fs::read(&paths.config_path).unwrap(), config_before);
    }

    #[test]
    fn malformed_codex_config_does_not_block_store_migration_or_get_rewritten() {
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
            aggregates: vec![],
            codex_access_mode: CodexAccessMode::Router,
            codex_subagents: None,
            providers: vec![provider],
            router: RouterSettings {
                enabled: true,
                bearer_token: "old-router-token".into(),
                ..Default::default()
            },
            active_provider_id: Some("legacy-chat".into()),
            compression: Default::default(),
        };
        let mut value = serde_json::to_value(store).unwrap();
        value["providers"][0]["wireApi"] = serde_json::json!("chat");
        secret_file::write(&store_path(&paths), &serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::create_dir_all(paths.config_path.parent().unwrap()).unwrap();
        std::fs::write(&paths.config_path, "[invalid").unwrap();

        let migrated = load_store(&paths).unwrap();
        assert!(!migrated.router.enabled);
        assert_eq!(
            std::fs::read_to_string(&paths.config_path).unwrap(),
            "[invalid"
        );

        let stored: Value =
            serde_json::from_slice(&secret_file::read(&store_path(&paths)).unwrap().unwrap())
                .unwrap();
        assert_eq!(stored["version"].as_u64(), Some(STORE_VERSION as u64));
        assert_eq!(stored["router"]["enabled"].as_bool(), Some(false));
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
        let catalog = std::fs::read_to_string(catalog_path(&paths)).unwrap();
        assert!(catalog.contains(&format!("{id}::model-a")));
        remove_routing_config(&paths, &store).unwrap();
        let text = std::fs::read_to_string(&paths.config_path).unwrap();
        assert!(!text.contains("agentharbor_p_"));
        assert!(!catalog_path(&paths).exists());
        let parsed: toml::Value = text.parse().unwrap();
        // The original provider/model/auth preference and foreign tables survive.
        assert_eq!(
            parsed["model_providers"]["other"]["name"].as_str(),
            Some("Other")
        );
        assert_eq!(parsed["model"].as_str(), Some("gpt-x"));
        assert_eq!(parsed["model_provider"].as_str(), Some("aimami_newapi"));
        assert_eq!(parsed["preferred_auth_method"].as_str(), Some("chatgpt"));
    }

    #[test]
    fn codex_catalog_lists_every_selected_responses_model_with_local_metadata() {
        let (_dir, paths) = paths();
        let store = Store {
            version: STORE_VERSION,
            codex_access_mode: CodexAccessMode::Router,
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
                        protocol: None,
                        context_window: Some(128_000),
                        max_output_tokens: Some(8_192),
                        endpoint_types: vec!["openai-response".into()],
                        included: true,
                        metadata_source: "local".into(),
                        ..Default::default()
                    },
                    newapi::Model {
                        id: "model-extra".into(),
                        display_name: "Extra model".into(),
                        protocol: None,
                        context_window: Some(400_000),
                        max_output_tokens: Some(32_768),
                        endpoint_types: vec!["openai-response".into()],
                        included: true,
                        metadata_source: "manual".into(),
                        overrides: newapi::ModelOverrides {
                            context_window: Some(400_000),
                            max_output_tokens: Some(32_768),
                            ..Default::default()
                        },
                        ..Default::default()
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
        assert_eq!(models[0]["display_name"], "Woyao/Default model");
        assert_eq!(models[0]["context_window"], 128_000);
        assert_eq!(models[0]["shell_type"], "unified_exec");
        assert_eq!(
            models[0]["supported_reasoning_levels"],
            serde_json::json!([])
        );
        assert_eq!(models[0]["truncation_policy"]["mode"], "tokens");
        assert!(models[0]["base_instructions"].as_str().is_some());
        assert_eq!(models[1]["slug"], "relay-one::model-extra");
        assert_eq!(models[1]["context_window"], 400_000);
        assert_eq!(models[1]["max_output_tokens"], 32_768);
        assert_eq!(
            menu_display_name("Woyao", "Woyao/model-a", "model-a"),
            "Woyao/model-a"
        );
    }

    #[test]
    fn capability_catalog_is_inferred_from_an_exact_endpoint_host() {
        let (_dir, paths) = paths();
        let catalog = models_dev::load(&paths.app_data_dir);
        let official = Provider {
            base_url: "https://api.deepseek.com/v1".into(),
            ..Default::default()
        };
        let (scope, strict) = effective_catalog_scope(&official, "deepseek-flash", &catalog);
        assert_eq!(scope, vec!["deepseek"]);
        assert!(strict);
        let resolved = resolve_model_metadata(&official, "deepseek-flash", None, &catalog);
        assert_eq!(resolved.context_window.value, Some(1_000_000));
        // The official entry currently declares 393216 output tokens; 302.AI
        // has 384000 for the same model ID, so this also guards source choice.
        assert_eq!(resolved.output_limit.value, Some(393_216));

        let relay = Provider {
            base_url: "https://api.302.ai/v1".into(),
            ..Default::default()
        };
        let (scope, strict) = effective_catalog_scope(&relay, "deepseek-flash", &catalog);
        assert_eq!(scope, vec!["deepseek"]);
        assert!(strict);
        let resolved = resolve_model_metadata(&relay, "deepseek-flash", None, &catalog);
        assert_eq!(resolved.output_limit.value, Some(393_216));
    }

    #[test]
    fn gpt_models_prefer_openai_catalog_even_behind_a_compatible_relay() {
        let (_dir, paths) = paths();
        let catalog = models_dev::load(&paths.app_data_dir);
        let provider = Provider {
            // The endpoint routes requests through a relay; it must not change
            // the authoritative publisher used for GPT capability metadata.
            base_url: "https://api.302.ai/v1".into(),
            ..Default::default()
        };
        let (scope, strict) = effective_catalog_scope(&provider, "gpt-5.2", &catalog);
        assert_eq!(scope, vec!["openai"]);
        assert!(strict);
        let resolved = resolve_model_metadata(&provider, "gpt-5.2", None, &catalog);
        assert_eq!(
            resolved
                .matched
                .as_ref()
                .map(|entry| entry.provider_id.as_str()),
            Some("openai")
        );
        assert_eq!(resolved.context_window.value, Some(400_000));
    }

    #[test]
    fn claude_gemini_and_mimo_models_prefer_their_official_catalogs() {
        let (_dir, paths) = paths();
        let catalog = models_dev::load(&paths.app_data_dir);
        let relay = Provider {
            base_url: "https://relay.example.com/v1".into(),
            ..Default::default()
        };
        for (model_id, expected_owner) in [
            ("claude-sonnet-4-5-20250929", "anthropic"),
            ("anthropic/claude-sonnet-4-5-20250929", "anthropic"),
            ("gemini-2.5-flash", "google"),
            ("channel::gemini-2.5-flash", "google"),
            ("mimo-v2.5-pro", "xiaomi"),
            ("provider__mimo-v2.5-pro", "xiaomi"),
            ("qwen-flash", "alibaba"),
            ("kimi-k2.6", "moonshotai"),
            ("glm-4.5", "zhipuai"),
            ("grok-4.20-0309-reasoning", "xai"),
            ("MiniMax-M2.5", "minimax"),
            ("step-3.5-flash", "stepfun"),
            ("mistral-large-2411", "mistral"),
            // Canonical model IDs also cover families without a simple prefix.
            ("deep-research-max-preview-04-2026", "google"),
        ] {
            let (scope, strict) = effective_catalog_scope(&relay, model_id, &catalog);
            assert_eq!(scope, vec![expected_owner], "{model_id}");
            assert!(strict, "{model_id}");
            let resolved = resolve_model_metadata(&relay, model_id, None, &catalog);
            assert_eq!(
                resolved
                    .matched
                    .as_ref()
                    .map(|entry| entry.provider_id.as_str()),
                Some(expected_owner),
                "{model_id}"
            );
        }
    }

    #[test]
    fn publisher_aliases_resolve_reasoning_suffixes_and_canonical_model_names() {
        let (_dir, paths) = paths();
        let catalog = models_dev::load(&paths.app_data_dir);
        let provider = Provider {
            base_url: "https://relay.example.com/v1".into(),
            ..Default::default()
        };
        for (requested, owner, official_model) in [
            ("claude-opus-4-8-thinking", "anthropic", "claude-opus-4-8"),
            (
                "kiro__claude-opus-4-8-thinking",
                "anthropic",
                "claude-opus-4-8",
            ),
            ("gemini-3.8-flash-high", "google", "gemini-3.8-flash"),
            (
                "gemini-3.8-flash-thinking-high",
                "google",
                "gemini-3.8-flash",
            ),
            ("gemini-3.8-flash_high", "google", "gemini-3.8-flash"),
            ("gemini-3.8-flash:high", "google", "gemini-3.8-flash"),
            ("gemini-3.1-pro-high", "google", "gemini-3.1-pro-preview"),
            ("gemini-3.1-pro", "google", "gemini-3.1-pro-preview"),
            (
                "woyao/gemini-3.1-pro-thinking-high",
                "google",
                "gemini-3.1-pro-preview",
            ),
            ("deepseek-v4.1-flash", "deepseek", "deepseek-flash"),
            ("deepseek/deepseek-v4.1-flash", "deepseek", "deepseek-flash"),
            ("alibaba/deepseek-v4.1-flash", "deepseek", "deepseek-flash"),
            ("deepseek-v4.1-flash-thinking", "deepseek", "deepseek-flash"),
        ] {
            let resolved = resolve_model_metadata(&provider, requested, None, &catalog);
            let matched = resolved
                .matched
                .as_ref()
                .unwrap_or_else(|| panic!("{requested} should match"));
            assert_eq!(matched.provider_id, owner, "{requested}");
            assert_eq!(matched.model_id, official_model, "{requested}");
            assert_eq!(resolved.raw_model_id, requested);
            assert_eq!(resolved.match_kind, model_metadata::MatchKind::Normalized);
            assert!(resolved.conflicts.is_empty(), "{requested}");
        }
        let resolved = resolve_model_metadata(&provider, "deepseek-v4.2-flash", None, &catalog);
        assert!(resolved.matched.is_none(), "versions must never be guessed");
    }

    #[test]
    fn automatic_channel_prices_use_endpoint_hosts_and_fallback_to_model_references() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "providers": [
                {"id": "deepseek", "api": "https://api.deepseek.test/v1", "models": [
                    {"id": "deepseek-flash", "canonicalModelId": "deepseek/deepseek-v4.1-flash",
                     "limit": {"context": 1000000}, "cost": {"input": 1, "output": 2}}
                ]},
                {"id": "relay", "api": "https://relay.test/provider/v1", "models": [
                    {"id": "deepseek/deepseek-v4.1-flash", "limit": {"context": 99999},
                     "cost": {"input": 5, "output": 6, "cacheRead": 0}}
                ]}
            ]
        }))
        .unwrap();
        let catalog = models_dev::Catalog::parse(&bytes, models_dev::Origin::Runtime).unwrap();
        let mut provider = Provider {
            base_url: "https://relay.test/provider/v1".into(),
            ..Default::default()
        };
        let id = "deepseek/deepseek-v4.1-flash";
        let resolved = resolve_model_metadata(&provider, id, None, &catalog);
        assert_eq!(resolved.context_window.value, Some(1000000));
        assert_eq!(resolved.price.input, Some(5.0));
        assert_eq!(resolved.price.cache_read, Some(0.0));
        assert_eq!(resolved.price.source, model_metadata::source::CATALOG);
        let channel_snapshot = pricing_for(&provider, id, &catalog);
        assert_eq!(channel_snapshot.input, Some(5.0));
        for host in ["https://unknown.test/v1", "https://relay.test.evil.test/v1"] {
            provider.base_url = host.into();
            let reference = resolve_model_metadata(&provider, id, None, &catalog).price;
            assert_eq!(reference.input, Some(1.0), "{host}");
            assert_eq!(reference.source, model_metadata::source::CATALOG_REFERENCE);
        }
        provider.base_url = "https://api.deepseek.test/v1".into();
        let canonical = resolve_model_metadata(&provider, id, None, &catalog).price;
        assert_eq!(canonical.input, Some(1.0));
        assert_eq!(canonical.source_ref.unwrap().model_id, "deepseek-flash");
        assert_ne!(canonical.source, model_metadata::source::CATALOG_REFERENCE);
        provider.price_catalog_provider = Some("relay".into());
        let legacy = resolve_model_metadata(&provider, id, None, &catalog).price;
        assert_eq!(legacy.input, Some(5.0));

        // Capture real request prices on either side of a multiplier edit.
        let dir = tempfile::tempdir().unwrap();
        for factor in [0.5, 2.0] {
            provider.price_multiplier = Some(factor);
            let snapshot = pricing_for(&provider, id, &catalog);
            assert_eq!(snapshot.input, Some(5.0 * factor));
            assert_eq!(snapshot.cache_read, Some(0.0));
            assert_eq!(snapshot.multiplier, Some(factor));
            let mut observer = relay_usage::RelayUsageRequest::new(
                dir.path().into(),
                "relay".into(),
                "Relay".into(),
                id.into(),
                "chatCompletions".into(),
                snapshot,
            )
            .observe(200, false);
            observer.observe(br#"{"usage":{"prompt_tokens":1000000,"completion_tokens":0,"prompt_tokens_details":{"cached_tokens":0}}}"#);
        }
        let ledger = relay_usage::load(dir.path()).unwrap();
        assert_eq!(ledger.all_time.cost_micros, 12_500_000);
        assert!(ledger
            .recent
            .iter()
            .any(|record| record.cost_usd == Some(2.5)
                && record.pricing_snapshot.as_ref().unwrap().multiplier == Some(0.5)));
        assert!(ledger
            .recent
            .iter()
            .any(|record| record.cost_usd == Some(10.0)
                && record.pricing_snapshot.as_ref().unwrap().multiplier == Some(2.0)));
        provider.price_catalog_provider = None;
        provider.base_url = "https://unknown.test/v1".into();
        assert_eq!(
            pricing_for(&provider, id, &catalog).input,
            Some(2.0),
            "model reference estimates use the same multiplier"
        );
    }

    #[test]
    fn endpoint_catalog_inference_uses_exact_hosts_and_respects_explicit_binding() {
        let (_dir, paths) = paths();
        let catalog = models_dev::load(&paths.app_data_dir);
        let lookalike = Provider {
            base_url: "https://api.deepseek.com.evil.test/v1".into(),
            ..Default::default()
        };
        let (scope, strict) = effective_catalog_scope(&lookalike, "private-model", &catalog);
        assert!(scope.is_empty());
        assert!(!strict);

        let official = Provider {
            base_url: "https://api.deepseek.com/v1".into(),
            ..Default::default()
        };
        let (scope, strict) = effective_catalog_scope(&official, "private-model", &catalog);
        assert_eq!(scope, vec!["deepseek"]);
        assert!(strict);

        let explicit = Provider {
            base_url: "https://api.deepseek.com/v1".into(),
            catalog_providers: vec!["302ai".into()],
            ..Default::default()
        };
        let (scope, strict) = effective_catalog_scope(&explicit, "private-model", &catalog);
        assert_eq!(scope, vec!["302ai"]);
        assert!(!strict);
    }

    #[test]
    fn inferred_official_scope_does_not_fall_back_to_a_relay_only_model() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "providers": [
                {"id": "deepseek", "api": "https://api.deepseek.com", "models": []},
                {"id": "302ai", "api": "https://api.302.ai/v1", "models": [
                    {"id": "relay-only", "name": "Relay only", "limit": {"context": 1000}}
                ]}
            ]
        }))
        .unwrap();
        let catalog = models_dev::Catalog::parse(&bytes, models_dev::Origin::Runtime).unwrap();
        let scope = vec!["deepseek".to_owned()];
        let request = model_metadata::ResolutionRequest {
            raw_model_id: "relay-only",
            display_name: "",
            catalog_providers: &scope,
            price_providers: &[],
            explicit_ref: None,
            price_ref: None,
            user: model_metadata::UserValues::default(),
            provider_context: None,
            official: None,
            upstream: None,
        };
        let resolved =
            model_metadata::resolve_with_global_fallback(&catalog, &request, false, None);
        assert_eq!(resolved.context_window.value, None);
        assert_eq!(
            resolved.context_window.source,
            model_metadata::source::UNKNOWN
        );
    }

    #[test]
    fn automatic_generic_metadata_uses_only_publishers() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "providers": [
                {"id": "302ai", "models": [
                    {"id": "relay-only", "limit": {"context": 1000}},
                    {"id": "publisher-model", "limit": {"context": 1000}}
                ]},
                {"id": "openrouter", "models": [
                    {"id": "relay-only", "limit": {"context": 9999}},
                    {"id": "publisher-model", "limit": {"context": 9999}}
                ]},
                {"id": "openai", "models": [
                    {"id": "publisher-model", "limit": {"context": 456000}},
                    {"id": "claude-relay-only", "limit": {"context": 5000}}
                ]}
            ]
        }))
        .unwrap();
        let catalog = models_dev::Catalog::parse(&bytes, models_dev::Origin::Runtime).unwrap();
        let provider = Provider {
            base_url: "https://relay.example.com/v1".into(),
            ..Default::default()
        };
        let resolved = resolve_model_metadata(&provider, "relay-only", None, &catalog);
        assert!(resolved.matched.is_none());
        assert_eq!(resolved.context_window.value, None);

        let resolved = resolve_model_metadata(&provider, "publisher-model", None, &catalog);
        assert_eq!(resolved.matched.as_ref().unwrap().provider_id, "openai");
        assert_eq!(resolved.context_window.value, Some(456_000));

        // Missing publisher data stays unknown even if another official service
        // hosts a same-named model. It cannot impersonate the model's origin.
        let resolved = resolve_model_metadata(&provider, "claude-relay-only", None, &catalog);
        assert!(resolved.matched.is_none());
        assert_eq!(resolved.context_window.value, None);

        // A deliberate user binding can still describe that service's own
        // metadata, while automatic references remain limited to publishers.
        let manual = Provider {
            catalog_providers: vec!["openrouter".into()],
            ..provider
        };
        let resolved = resolve_model_metadata(&manual, "publisher-model", None, &catalog);
        assert_eq!(resolved.matched.as_ref().unwrap().provider_id, "openrouter");
        assert_eq!(resolved.context_window.value, Some(9999));
    }

    #[test]
    fn catalog_fills_unknown_relay_model_context() {
        let (_dir, paths) = paths();
        let store = Store {
            providers: vec![Provider {
                id: "relay-one".into(),
                name: "Relay".into(),
                base_url: "https://relay.example.com".into(),
                api_key: "sk-test".into(),
                model: "unknown-relay-model".into(),
                enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let catalog = catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        assert_eq!(catalog["models"][0]["context_window"], 128_000);
    }

    #[test]
    fn relay_catalog_entry_uses_resolved_levels_and_never_invents_them() {
        let levels: Vec<Value> = [
            ("low", "Fast responses with lighter reasoning"),
            ("medium", "Balanced reasoning"),
        ]
        .iter()
        .map(|(effort, description)| serde_json::json!({"effort": effort, "description": description}))
        .collect();
        let claude = relay_catalog_entry(
            "provider::claude-opus-4-8".into(),
            "Relay/Claude Opus 4.8".into(),
            "Claude".into(),
            1_000_000,
            Some(128_000),
            levels.clone(),
            Some("medium".into()),
        );
        assert_eq!(claude["default_reasoning_level"], "medium");
        assert_eq!(
            claude["supported_reasoning_levels"],
            serde_json::json!(levels)
        );
        assert_eq!(claude["context_window"], 1_000_000);

        // An unknown model carries an explicit empty list and no default; the
        // entry builder never guesses a family's levels.
        let plain = relay_catalog_entry(
            "provider::unknown-model".into(),
            "Unknown".into(),
            String::new(),
            128_000,
            Some(8_192),
            Vec::new(),
            None,
        );
        assert!(plain["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(plain["default_reasoning_level"].is_null());
    }

    #[test]
    fn relay_entries_adopt_codex_multi_agent_behavior_but_not_limits() {
        let level = |effort: &str| serde_json::json!({"effort": effort, "description": effort});
        let official = serde_json::json!({
            "slug": "gpt-6.1-sol",
            "context_window": 999_000,
            "multi_agent_version": "v2",
            "multi_agent_reasoning_effort": "xhigh",
            "supported_reasoning_levels": [level("high"), level("xhigh"), level("max"),
                {"effort": "ultra", "description": "Official ultra"}],
        });
        let entry = |levels: Vec<Value>| {
            relay_catalog_entry(
                "relay::gpt-6.1-sol".into(),
                "Relay".into(),
                String::new(),
                128_000,
                None,
                levels,
                None,
            )
        };
        let efforts = |entry: &Value| {
            entry["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["effort"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };

        // The same model gains its multi-agent version, preferred effort and Ultra.
        let mut full = entry(vec![level("high"), level("xhigh"), level("max")]);
        adopt_codex_behavior(&mut full, Some(&official));
        assert_eq!(full["multi_agent_version"], "v2");
        assert_eq!(full["multi_agent_reasoning_effort"], "xhigh");
        assert_eq!(efforts(&full), ["high", "xhigh", "max", "ultra"]);
        assert_eq!(full["supported_reasoning_levels"][3]["description"], "Official ultra");
        assert_eq!(full["context_window"], 128_000);
        // The catalog is rebuilt on every read; adopting twice is stable.
        let once = full.clone();
        adopt_codex_behavior(&mut full, Some(&official));
        assert_eq!(full, once);

        // A preferred effort the relay lacks is dropped; Codex then picks max.
        let mut partial = entry(vec![level("high"), level("max")]);
        adopt_codex_behavior(&mut partial, Some(&official));
        assert!(partial.get("multi_agent_reasoning_effort").is_none());
        assert_eq!(efforts(&partial), ["high", "max", "ultra"]);

        // Without declared reasoning Ultra has nothing to resolve to.
        let mut silent = entry(Vec::new());
        adopt_codex_behavior(&mut silent, Some(&official));
        assert_eq!(silent["multi_agent_version"], "v2");
        assert!(efforts(&silent).is_empty());

        // V1 keeps its version without Ultra; no declaration changes nothing.
        let mut v1_official = official.clone();
        v1_official["multi_agent_version"] = "v1".into();
        let mut v1 = entry(vec![level("high")]);
        adopt_codex_behavior(&mut v1, Some(&v1_official));
        assert_eq!(v1["multi_agent_version"], "v1");
        assert_eq!(efforts(&v1), ["high"]);
        for official in [None, Some(serde_json::json!({"slug": "gpt-5.5"}))] {
            let mut plain = entry(vec![level("high")]);
            let before = plain.clone();
            adopt_codex_behavior(&mut plain, official.as_ref());
            assert_eq!(plain, before);
        }
    }

    #[test]
    fn relay_menu_borrows_codex_ultra_behavior_from_the_same_official_model() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let cache = serde_json::to_vec(&serde_json::json!({"models": [{
            "slug": "account-model-a",
            "context_window": 999_000,
            "multi_agent_version": "v2",
            "multi_agent_reasoning_effort": "xhigh",
            "supported_reasoning_levels": (["high", "xhigh", "ultra"]
                .map(|effort| serde_json::json!({"effort": effort, "description": effort}))),
        }]}))
        .unwrap();
        std::fs::write(paths.codex_home.join("models_cache.json"), &cache).unwrap();
        super::super::model_catalog::seed_test_account_catalog(
            &paths.codex_home,
            &paths.app_data_dir,
            &cache,
        );
        let store = Store {
            providers: vec![Provider {
                id: "relay-one".into(),
                name: "Relay".into(),
                base_url: "https://relay.example.com".into(),
                api_key: "sk-test".into(),
                model: "account-model-a".into(),
                enabled: true,
                models: vec![newapi::Model {
                    id: "account-model-a".into(),
                    included: true,
                    overrides: newapi::ModelOverrides {
                        reasoning: Some(
                            ["high", "xhigh"]
                                .map(|effort| newapi::ReasoningLevel {
                                    effort: effort.into(),
                                    description: effort.into(),
                                })
                                .into(),
                        ),
                        ..Default::default()
                    },
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let catalog = catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        let relay = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "relay-one::account-model-a")
            .unwrap();
        // A third-party endpoint never inherits account limits, but a thread
        // that starts on it gets the same multi-agent version and Ultra.
        assert_ne!(relay["context_window"], 999_000);
        assert_eq!(relay["multi_agent_version"], "v2");
        assert_eq!(relay["multi_agent_reasoning_effort"], "xhigh");
        assert_eq!(
            relay["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|level| level["effort"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["high", "xhigh", "ultra"]
        );
        // The request path sees the same entry as the menu.
        assert_eq!(
            catalog_entry_for_slug(
                &paths.app_data_dir,
                &paths.codex_home,
                &store,
                "relay-one::account-model-a",
            )
            .as_ref(),
            Some(relay)
        );
    }

    #[test]
    fn official_snapshot_values_only_apply_to_the_official_endpoint() {
        let (_dir, paths) = paths();
        std::fs::create_dir_all(&paths.app_data_dir).unwrap();
        let official = serde_json::json!([
            {
                "slug": "account-model-a",
                "display_name": "Official A",
                "default_reasoning_level": "low",
                "supported_reasoning_levels": [
                    {"effort": "low", "description": "Cached low"},
                    {"effort": "medium", "description": "Cached medium"}
                ],
                "context_window": 999_000,
                "base_instructions": "Official-only instructions",
                "shell_type": "official-shell",
                "experimental_supported_tools": ["official-tool"],
                "comp_hash": "official-internal-id",
                "use_responses_lite": true,
                "supports_reasoning_effort_updates": true
            },
            {
                "slug": "account-model-b",
                "default_reasoning_level": "medium",
                "supported_reasoning_levels": [{"effort": "medium", "description": "Cached medium"}]
            }
        ]);
        let cache_bytes = serde_json::to_vec(&serde_json::json!({"models": official})).unwrap();
        std::fs::write(paths.codex_home.join("models_cache.json"), &cache_bytes).unwrap();
        super::super::model_catalog::seed_test_account_catalog(&paths.codex_home, &paths.app_data_dir, &cache_bytes);
        let previous_only =
            serde_json::json!({"slug": "previous-only", "display_name": "Previous"});
        std::fs::write(
            paths.app_data_dir.join("previous-router-catalog.json"),
            serde_json::to_vec(&serde_json::json!({"models": [
                previous_only,
                {"slug": "old-provider::stale-model"},
                {"slug": "aimami_relay_stale-model"}
            ]}))
            .unwrap(),
        )
        .unwrap();
        let ids = ["account-model-a", "account-model-b", "unknown-relay-model"];
        let store_for = |base_url: &str| Store {
            providers: vec![Provider {
                id: "relay-one".into(),
                name: "Relay".into(),
                base_url: base_url.into(),
                api_key: "sk-test".into(),
                model: "account-model-a".into(),
                enabled: true,
                models: ids
                    .iter()
                    .map(|id| newapi::Model {
                        id: (*id).into(),
                        display_name: format!("Alias {id}"),
                        included: true,
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };

        // Official entries pass through unchanged. Historical metadata is not
        // evidence that a model is still available to the current account.
        let catalog = catalog_value(
            &paths.app_data_dir,
            &paths.codex_home,
            &store_for("https://relay.example.com"),
        );
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.len(), 2 + ids.len());
        assert_eq!(&models[..2], official.as_array().unwrap());
        assert!(!models.iter().any(|model| model["slug"] == previous_only["slug"]));
        assert!(catalog_entry_for_slug(
            &paths.app_data_dir,
            &paths.codex_home,
            &store_for("https://relay.example.com"),
            "previous-only",
        )
        .is_none());

        // Relay capabilities remain separate from the signed-in account.
        for (index, id) in ids.iter().enumerate() {
            let relay = &models[2 + index];
            assert_eq!(relay["slug"], format!("relay-one::{id}"));
            assert_eq!(relay["display_name"], format!("Relay/Alias {id}"));
            assert_ne!(
                relay["context_window"], 999_000,
                "account limits leaked into {id}"
            );
            assert_eq!(relay["context_window"], 128_000);
            assert_eq!(relay["supported_reasoning_levels"], serde_json::json!([]));
            assert!(relay["default_reasoning_level"].is_null());
            // Structural guarantees for Codex's catalog parser.
            assert_eq!(relay["shell_type"], "unified_exec");
            assert_eq!(relay["visibility"], "list");
            assert_eq!(relay["supported_in_api"], true);
            assert_eq!(relay["experimental_supported_tools"], serde_json::json!([]));
            assert!(relay["base_instructions"].as_str().is_some());
            assert_ne!(relay["base_instructions"], official[0]["base_instructions"]);
            assert_eq!(relay["truncation_policy"]["mode"], "tokens");
            for key in [
                "comp_hash",
                "use_responses_lite",
                "supports_reasoning_effort_updates",
            ] {
                assert!(relay.get(key).is_none(), "{key} leaked into {id}");
            }
        }

        // A provider that actually targets the official endpoint keeps the
        // account values as its declaration.
        let catalog = catalog_value(
            &paths.app_data_dir,
            &paths.codex_home,
            &store_for("https://chatgpt.com/backend-api/codex"),
        );
        let models = catalog["models"].as_array().unwrap();
        let account = &models[2];
        assert_eq!(account["context_window"], 999_000);
        assert_eq!(account["default_reasoning_level"], "low");
        assert_eq!(
            account["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        assert_eq!(
            std::fs::read(paths.codex_home.join("models_cache.json")).unwrap(),
            cache_bytes
        );
        assert!(!paths.config_path.exists());
        assert!(!catalog_path(&paths).exists());
    }

    #[test]
    fn relay_catalog_never_invents_reasoning_for_unknown_models() {
        // Nothing about a name may manufacture levels or a default.
        for model_id in ["unknown-model", "vendor/unknown-model", "zzz-private-9"] {
            let (_dir, paths) = paths();
            let store = Store {
                providers: vec![Provider {
                    id: "relay".into(),
                    name: "Relay".into(),
                    base_url: "https://relay.example.com".into(),
                    api_key: "sk-test".into(),
                    model: model_id.into(),
                    enabled: true,
                    models: vec![newapi::Model {
                        id: model_id.into(),
                        included: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let catalog = catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
            let relay = &catalog["models"][0];
            let levels = relay["supported_reasoning_levels"].as_array().unwrap();
            assert!(
                levels.is_empty(),
                "invented reasoning support for {model_id}"
            );
            assert!(relay["default_reasoning_level"].is_null());
            // The shared fallback still gives Codex a usable window and the
            // 90% default threshold, clearly derived from the fallback budget.
            assert_eq!(relay["context_window"], 128_000);
            assert_eq!(relay["auto_compact_token_limit"], 115_200);
        }
        // A declared default is only kept when the levels actually support it.
        let levels: Vec<Value> = ["high", "low"]
            .iter()
            .map(|effort| serde_json::json!({"effort": effort, "description": effort}))
            .collect();
        let entry = relay_catalog_entry(
            "relay::x".into(),
            "X".into(),
            String::new(),
            128_000,
            None,
            levels,
            Some("none".into()),
        );
        // The entry builder copies the resolver's decision; the resolver never
        // emits an unsupported default, so a mismatch cannot occur here.
        assert_eq!(entry["default_reasoning_level"], "none");
    }

    #[test]
    fn balance_settings_update_only_the_selected_provider() {
        let (_dir, paths) = paths();
        let state = save_provider(&paths, input("Relay", "https://relay.example.com")).unwrap();
        let id = &state.state.providers[0].id;
        let url = "https://relay.example.com/";
        assert_eq!(
            set_balance_credentials(
                &paths,
                id,
                "https://other.example.com/",
                Some(42),
                "account-token"
            )
            .unwrap_err(),
            "newapi.errors.changed"
        );
        let view = set_balance_credentials(&paths, id, url, Some(42), "account-token").unwrap();
        assert!(view.has_access_token);
        let script = r#"({request:{url:"{{baseUrl}}/v1/usage",method:"GET"},extractor:r=>r})"#;
        let view = set_balance_script(&paths, id, url, script).unwrap();
        assert_eq!(view.balance_script, script);
        let stored = load_store(&paths).unwrap();
        assert_eq!(stored.providers[0].access_token, "account-token");
        assert_eq!(stored.providers[0].balance_script, script);
    }

    #[test]
    fn same_site_balance_defaults_to_first_card_and_can_be_overridden() {
        let (_dir, paths) = paths();
        let first = save_provider(&paths, input("First", "https://relay.example.com")).unwrap();
        let first_id = first.state.providers[0].id.clone();
        let second = save_provider(&paths, input("Second", "https://relay.example.com")).unwrap();
        let second_id = second.state.providers[1].id.clone();
        let mut store = load_store(&paths).unwrap();
        store.providers[0].balance = Some(newapi::Balance {
            remaining: 12.5,
            ..Default::default()
        });
        store.providers[0].balance_source = Some("custom".into());
        store.providers[1].balance = Some(newapi::Balance {
            remaining: 99.0,
            ..Default::default()
        });
        save_store_unlocked(&paths, &store).unwrap();

        let view = state_from_store_at(&store, false, None);
        assert_eq!(
            view.providers[1].balance_shared_from.as_deref(),
            Some(first_id.as_str())
        );
        assert_eq!(view.providers[1].balance.as_ref().unwrap().remaining, 12.5);

        let own = set_balance_credentials(
            &paths,
            &second_id,
            "https://relay.example.com/",
            Some(42),
            "own-token",
        )
        .unwrap();
        assert_eq!(own.balance_shared_from, None);
        assert_eq!(own.user_id, Some(42));
        let store = load_store(&paths).unwrap();
        assert_eq!(store.providers[0].access_token, "token-a");
        assert_eq!(store.providers[1].access_token, "own-token");

        let shared =
            set_balance_sharing(&paths, &second_id, "https://relay.example.com/", true).unwrap();
        assert_eq!(
            shared.balance_shared_from.as_deref(),
            Some(first_id.as_str())
        );
        assert_eq!(shared.user_id, Some(7));
        assert_eq!(shared.balance.unwrap().remaining, 12.5);

        let own =
            set_balance_sharing(&paths, &second_id, "https://relay.example.com/", false).unwrap();
        assert_eq!(own.balance_shared_from, None);
        assert_eq!(own.user_id, Some(7));
        assert_eq!(own.balance.unwrap().remaining, 12.5);
    }

    #[test]
    fn balance_refresh_interval_defaults_persists_and_follows_shared_owner() {
        let legacy: Provider = serde_json::from_value(serde_json::json!({"id":"legacy"})).unwrap();
        assert_eq!(legacy.balance_refresh_interval_minutes, 0);
        let (_dir, paths) = paths();
        let first = save_provider(&paths, input("First", "https://relay.example.com")).unwrap();
        let first_id = first.state.providers[0].id.clone();
        let second = save_provider(&paths, input("Second", "https://relay.example.com")).unwrap();
        let second_id = second.state.providers[1].id.clone();
        let url = "https://relay.example.com/";
        let before = load_store(&paths).unwrap();
        let shared = set_balance_refresh_interval(&paths, &second_id, url, 5).unwrap();
        assert_eq!(shared.balance_refresh_interval_minutes, 5);
        assert_eq!(
            shared.balance_shared_from.as_deref(),
            Some(first_id.as_str())
        );
        let persisted = load_store(&paths).unwrap();
        assert_eq!(persisted.providers[0].balance_refresh_interval_minutes, 5);
        assert_eq!(persisted.providers[1].balance_refresh_interval_minutes, 0);
        assert_eq!(persisted.providers[0].api_key, before.providers[0].api_key);
        assert_eq!(
            persisted.providers[0].access_token,
            before.providers[0].access_token
        );
        let targets = balance_refresh_targets(&paths).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, first_id);
        assert_eq!(
            set_balance_refresh_interval(&paths, &second_id, "https://wrong.example/", 1)
                .unwrap_err(),
            "newapi.errors.changed"
        );
        assert_eq!(
            set_balance_refresh_interval(&paths, &second_id, url, 10_081).unwrap_err(),
            "newapi.errors.balanceInterval"
        );
        let own = set_balance_sharing(&paths, &second_id, url, false).unwrap();
        assert_eq!(own.balance_refresh_interval_minutes, 5);
        assert_eq!(balance_refresh_targets(&paths).unwrap().len(), 2);
        set_balance_refresh_interval(&paths, &second_id, url, 0).unwrap();
        assert_eq!(balance_refresh_targets(&paths).unwrap().len(), 1);
        set_balance_refresh_interval(&paths, &first_id, url, 0).unwrap();
        assert!(balance_refresh_targets(&paths).unwrap().is_empty());
        assert!(query_scheduled_provider_balance(&paths, &first_id, &ApiProxyConfigPayload::default()).unwrap().is_none());
    }

    #[test]
    fn balance_migration_preserves_existing_per_card_credentials() {
        let (_dir, paths) = paths();
        let mut store = Store {
            version: 6,
            providers: vec![
                Provider {
                    id: "first".into(),
                    base_url: "https://relay.example.com/".into(),
                    access_token: "first-token".into(),
                    ..Default::default()
                },
                Provider {
                    id: "custom".into(),
                    base_url: "https://relay.example.com/".into(),
                    access_token: "own-token".into(),
                    ..Default::default()
                },
                Provider {
                    id: "shared".into(),
                    base_url: "https://relay.example.com/".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        store.router.bearer_token = "agentharbor_test".into();
        save_store_unlocked(&paths, &store).unwrap();
        let upgraded = load_store(&paths).unwrap();
        assert_eq!(upgraded.version, STORE_VERSION);
        assert!(upgraded.providers[1].balance_independent);
        assert!(!upgraded.providers[2].balance_independent);
        let view = state_from_store_at(&upgraded, false, None);
        assert_eq!(view.providers[1].balance_shared_from, None);
        assert_eq!(
            view.providers[2].balance_shared_from.as_deref(),
            Some("first")
        );
    }

    #[test]
    fn enabled_route_keeps_chatgpt_sign_in_and_per_model_context() {
        let (_dir, paths) = paths();
        std::fs::write(&paths.config_path, "preferred_auth_method = 'chatgpt'\n").unwrap();
        let mut draft = input("Relay", "https://relay.example.com");
        draft.model = "gpt-6-luna".into();
        draft.context_window = Some(400_000);
        draft.auto_compact_limit = Some(250_000);
        draft.models = vec![newapi::Model {
            id: "gpt-6-luna".into(),
            context_window: Some(272_000),
            metadata_source: "manual".into(),
            overrides: newapi::ModelOverrides {
                context_window: Some(272_000),
                ..Default::default()
            },
            ..Default::default()
        }];
        let state = save_provider(&paths, draft).unwrap();
        let id = state.state.providers[0].id.clone();
        set_enabled(&paths, &id, true).unwrap();
        let store = load_store(&paths).unwrap();
        apply_routing_config(&paths, &store).unwrap();
        let config: toml::Value = std::fs::read_to_string(&paths.config_path)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(config["preferred_auth_method"].as_str(), Some("chatgpt"));
        assert!(config.get("model_context_window").is_none());
        assert!(config.get("model_auto_compact_token_limit").is_none());
        let catalog: Value =
            serde_json::from_slice(&std::fs::read(catalog_path(&paths)).unwrap()).unwrap();
        let relay = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == format!("{id}::gpt-6-luna"))
            .unwrap();
        assert_eq!(relay["context_window"], 272_000);
        assert_eq!(relay["auto_compact_token_limit"], 250_000);
        // The built-in provider keeps Codex's ChatGPT sign-in; only its endpoint
        // moves to the router, whose token is part of that URL.
        assert!(config.get("model_provider").is_none());
        assert!(config.get("model_providers").is_none());
        assert_eq!(
            config["openai_base_url"].as_str(),
            Some(builtin_router_base(&store).as_str())
        );
    }

    #[test]
    fn unified_router_limits_do_not_leak_between_models_and_restore_on_disable() {
        let (_dir, paths) = paths();
        let original = "model_context_window = 65536\nmodel_auto_compact_token_limit = 60000\n";
        std::fs::write(&paths.config_path, original).unwrap();
        let store = Store {
            codex_access_mode: CodexAccessMode::Router,
            router: RouterSettings {
                enabled: true,
                bearer_token: "synthetic-token".into(),
                ..Default::default()
            },
            active_provider_id: Some("astra".into()),
            providers: vec![
                Provider {
                    id: "astra".into(),
                    name: "Astra".into(),
                    api_key: "key-a".into(),
                    model: "gpt-6-astra".into(),
                    enabled: true,
                    context_window: Some(400_000),
                    auto_compact_limit: Some(350_000),
                    ..Default::default()
                },
                Provider {
                    id: "claude".into(),
                    name: "Claude".into(),
                    api_key: "key-b".into(),
                    model: "claude-sonnet-4-6".into(),
                    enabled: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        apply_routing_config(&paths, &store).unwrap();
        let (_, config) = open_config(&paths).unwrap();
        assert!(config.get("model_context_window").is_none());
        assert!(config.get("model_auto_compact_token_limit").is_none());
        let catalog = catalog_value(&paths.app_data_dir, &paths.codex_home, &store);
        let models = catalog["models"].as_array().unwrap();
        let astra = models
            .iter()
            .find(|model| model["slug"] == "astra::gpt-6-astra")
            .unwrap();
        let claude = models
            .iter()
            .find(|model| model["slug"] == "claude::claude-sonnet-4-6")
            .unwrap();
        assert_eq!(astra["context_window"], 400_000);
        // The provider's own fixed threshold stays authoritative for its models.
        assert_eq!(astra["auto_compact_token_limit"], 350_000);
        // Claude metadata follows Anthropic's catalog even without an explicit
        // provider binding; 90% of the official context window is the threshold.
        assert_eq!(claude["context_window"], 1_000_000);
        assert_eq!(claude["auto_compact_token_limit"], 900_000);
        assert!(routing_config_matches(&paths, &store).unwrap());
        remove_routing_config(&paths, &store).unwrap();
        let (_, restored) = open_config(&paths).unwrap();
        assert_eq!(restored["model_context_window"].as_integer(), Some(65_536));
        assert_eq!(
            restored["model_auto_compact_token_limit"].as_integer(),
            Some(60_000)
        );
    }

    #[test]
    fn router_refresh_and_repair_keep_valid_model_switches() {
        let (_dir, paths) = paths();
        let saved = save_provider(&paths, input("Relay A", "https://relay-a.example.com")).unwrap();
        let a = saved.state.providers[0].id.clone();
        set_enabled(&paths, &a, true).unwrap();
        let saved = save_provider(&paths, input("Relay B", "https://relay-b.example.com")).unwrap();
        let b = saved.state.providers.iter().find(|provider| provider.id != a).unwrap().id.clone();
        set_enabled(&paths, &b, true).unwrap();
        set_smart_routing(&paths, true).unwrap();
        let mut store = load_store(&paths).unwrap();
        let mut official = catalog_value(&paths.app_data_dir, &paths.codex_home, &store)["models"][0].clone();
        official["slug"] = serde_json::json!("fixture-official");
        super::super::model_catalog::seed_test_account_catalog(&paths.codex_home, &paths.app_data_dir,
            serde_json::json!({"models":[official]}).to_string().as_bytes());
        std::fs::write(paths.codex_home.join("models_cache.json"),
            serde_json::json!({"models":[official]}).to_string()).unwrap();
        apply_routing_config(&paths, &store).unwrap();
        let model = store.providers.iter().find(|provider| provider.id == a).unwrap().model.clone();
        for selected in ["fixture-official".to_owned(), format!("{a}::{model}"), format!("{b}::{model}"), "fixture-official".to_owned()] {
            let (_, mut document) = open_config(&paths).unwrap();
            document["model"] = toml_edit::value(&selected);
            std::fs::write(&paths.config_path, document.to_string()).unwrap();
            assert!(routing_config_matches(&paths, &store).unwrap());
            assert!(ensure_routing_config(&paths, &store).unwrap().is_none());
            // A real card/global policy update refreshes the whole catalog.
            set_compression(&paths, model_policy::CompressionPolicy::percent(80)).unwrap();
            store = load_store(&paths).unwrap();
            assert_eq!(open_config(&paths).unwrap().1["model"].as_str(), Some(selected.as_str()));
            // Also preserve the choice when a stale critical field needs repair.
            let (_, mut document) = open_config(&paths).unwrap();
            document["openai_base_url"] = toml_edit::value("http://127.0.0.1:1/openai/stale/v1");
            std::fs::write(&paths.config_path, document.to_string()).unwrap();
            assert!(ensure_routing_config(&paths, &store).unwrap().is_some());
            assert_eq!(open_config(&paths).unwrap().1["model"].as_str(), Some(selected.as_str()));
        }
        let (_, mut document) = open_config(&paths).unwrap();
        document["model"] = toml_edit::value("removed-provider::removed-model");
        std::fs::write(&paths.config_path, document.to_string()).unwrap();
        assert!(ensure_routing_config(&paths, &store).unwrap().is_some());
        assert_eq!(open_config(&paths).unwrap().1["model"].as_str(), Some(codex_connection(&store).unwrap().1.as_str()));
    }

    #[test]
    fn routing_auth_switches_modes_without_touching_credentials() {
        let (_dir, paths) = paths();
        let state = save_provider(&paths, input("Relay", "https://relay.example.com")).unwrap();
        set_enabled(&paths, &state.state.providers[0].id, true).unwrap();
        let mut store = load_store(&paths).unwrap();
        store.router.enabled = true;
        let auth_bytes = br#"{"tokens":{"access_token":"fixture-preserve-token"}}"#;
        std::fs::write(&paths.auth_path, auth_bytes).unwrap();
        apply_routing_config(&paths, &store).unwrap();
        assert!(
            ensure_routing_config(&paths, &store).unwrap().is_none(),
            "repair must be idempotent"
        );

        store.router.accountless = true;
        assert!(ensure_routing_config(&paths, &store).unwrap().is_some());
        assert!(ensure_routing_config(&paths, &store).unwrap().is_none());
        let (_, accountless) = open_config(&paths).unwrap();
        assert_eq!(
            accountless["model_providers"][CATALOG_PROVIDER_KEY]["requires_openai_auth"].as_bool(),
            Some(false)
        );
        assert_eq!(
            accountless["preferred_auth_method"].as_str(),
            Some("apikey")
        );
        assert!(accountless.get("openai_base_url").is_none());

        store.router.accountless = false;
        assert!(ensure_routing_config(&paths, &store).unwrap().is_some());
        assert!(ensure_routing_config(&paths, &store).unwrap().is_none());
        let (_, signed_in) = open_config(&paths).unwrap();
        assert!(signed_in
            .get("model_providers")
            .and_then(|providers| providers.get(CATALOG_PROVIDER_KEY))
            .is_none());
        assert!(signed_in.get("model_provider").is_none());
        assert_eq!(
            signed_in["openai_base_url"].as_str(),
            Some(builtin_router_base(&store).as_str())
        );
        assert!(signed_in.get("preferred_auth_method").is_none());
        assert_eq!(std::fs::read(&paths.auth_path).unwrap(), auth_bytes);
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
    #[test]
    fn nova_and_publisher_identity_scopes_remain_strict() {
        let catalog = models_dev::Catalog::parse(serde_json::json!({"schemaVersion":1,"providers":[
            {"id":"nova","models":[{"id":"nova-2-lite-v1","name":"Nova","limit":{"context":1000}}]},
            {"id":"302ai","models":[{"id":"opaque-alias","name":"Alias","limit":{"context":1000},"canonicalModelId":"google/gemini-fake"}]}
        ]}).to_string().as_bytes(), models_dev::Origin::Runtime).unwrap();
        assert_eq!(official_catalog_for_model("nova-2-lite-v1", &catalog).as_deref(), Some("nova"));
        for id in ["amazon.nova-lite-v1:0", "us.amazon.nova-lite-v1:0", "amazon/nova-lite",
        ] {
            assert_eq!(official_catalog_for_model(id, &catalog).as_deref(), Some("amazon-bedrock"));
        }
        assert_eq!(official_catalog_for_model("opaque-alias", &catalog), None);
        assert_ne!(official_catalog_for_model("anthropic.claude-sonnet-4", &catalog).as_deref(), Some("amazon-bedrock"));
    }

    #[test]
    fn official_reasoning_values_use_shared_validation() {
        let value = serde_json::json!({"context_window":1000,"supported_reasoning_levels":[
            {"effort":"high","description":"Valid"},{"effort":"bad effort"},
            {"effort":"low","description":"x".repeat(1025)}]});
        assert_eq!(official_values(&value).unwrap().reasoning.unwrap().len(), 1);
        let value = serde_json::json!({"context_window":1000,"supported_reasoning_levels":vec![serde_json::json!({"effort":"high"});33]});
        assert!(official_values(&value).unwrap().reasoning.is_none());
    }

}
