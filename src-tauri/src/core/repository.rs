use crate::core::auth::*;
use crate::core::bootstrap_cache::{self, BootstrapStatePayload};
use crate::core::models::*;
use crate::core::quota_store::{self, QuotaStoreFile};
use crate::platform::paths::CodexPaths;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const REGISTRY_SCHEMA_VERSION: i32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryFile {
    pub schema_version: i32,
    pub updated_at: i64,
    pub active_account_key: Option<String>,
    pub items: Vec<RegistryItem>,
    pub auto_switch: Option<AutoSwitchConfig>,
    pub api: Option<ApiConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegistryItem {
    pub account_key: String,
    pub snapshot_path: String,
    pub email: String,
    pub alias: String,
    pub account_name: Option<String>,
    pub workspace_name: Option<String>,
    pub profile_name: Option<String>,
    pub plan: String,
    pub auth_mode: String,
    pub has_active_subscription: Option<bool>,
    pub subscription_expires_at: Option<i64>,
    pub subscription_will_renew: Option<bool>,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    pub last_usage_at: Option<i64>,
    pub cached_primary_window: Option<RateLimitWindow>,
    pub cached_secondary_window: Option<RateLimitWindow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AutoSwitchConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_5h_threshold")]
    pub threshold_5h_percent: i32,
    #[serde(default = "default_weekly_threshold")]
    pub threshold_weekly_percent: i32,
}

fn default_5h_threshold() -> i32 {
    15
}
fn default_weekly_threshold() -> i32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiConfig {
    #[serde(default = "default_true")]
    pub usage_refresh_enabled: bool,
    #[serde(default = "default_true", alias = "teamNameRefreshEnabled")]
    pub account_metadata_refresh_enabled: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            usage_refresh_enabled: true,
            account_metadata_refresh_enabled: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_usage_refresh_interval() -> String {
    "1m".to_string()
}

fn normalize_usage_refresh_interval(interval: &str) -> Option<&'static str> {
    match interval {
        "30s" => Some("30s"),
        "1m" => Some("1m"),
        "3m" => Some("3m"),
        "5m" => Some("5m"),
        _ => None,
    }
}

pub(crate) fn usage_refresh_interval_seconds(interval: &str) -> u64 {
    match normalize_usage_refresh_interval(interval).unwrap_or("1m") {
        "30s" => 30,
        "1m" => 60,
        "3m" => 180,
        "5m" => 300,
        _ => 60,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HotspotConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexMateSettings {
    #[serde(default)]
    pub hotspot: HotspotConfig,
    #[serde(default = "default_usage_refresh_interval")]
    pub usage_refresh_interval: String,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub api_proxy: ApiProxyConfigPayload,
}

#[derive(Debug, Default)]
pub(crate) struct ApiDiagnostics {
    pub usage_attempt_count: i32,
    pub usage_success_count: i32,
    pub name_attempt_count: i32,
    pub name_success_count: i32,
    pub last_usage_failure: Option<String>,
    pub last_usage_failure_account: Option<String>,
    pub last_name_failure: Option<String>,
    pub last_name_failure_account: Option<String>,
}

pub(crate) struct LoadedState {
    pub paths: AppPathState,
    pub settings: CodexMateSettings,
    pub registry: RegistryFile,
    pub last_scan_at: i64,
    pub warnings: Vec<CoreWarning>,
    pub api_diagnostics: ApiDiagnostics,
    pub active_usage_api_status: ApiReachabilityStatus,
    pub active_usage_api_last_error: Option<String>,
}

pub struct Repository {
    paths: CodexPaths,
}

impl Repository {
    pub fn new() -> Self {
        Self {
            paths: CodexPaths::new(),
        }
    }

    pub fn paths(&self) -> &CodexPaths {
        &self.paths
    }

    pub fn load_snapshot_local(&self) -> Result<CoreEnvelope<CoreSnapshotPayload>, CoreError> {
        let state = self.load_local_state()?;
        let status = self.make_status_payload(&state);
        Ok(CoreEnvelope::ok_with_warnings(
            CoreSnapshotPayload { status },
            state.warnings,
        ))
    }

    pub fn clean(&self) -> Result<CoreEnvelope<CleanPayload>, CoreError> {
        let mut registry = self.load_registry_or_empty()?;
        let original_count = registry.items.len();
        registry
            .items
            .retain(|item| Path::new(&item.snapshot_path).exists());
        let live_account_keys: std::collections::HashSet<_> = registry
            .items
            .iter()
            .map(|item| item.account_key.clone())
            .collect();

        let mut auth_backups_removed = 0i32;
        let mut registry_backups_removed = 0i32;

        if self.paths.auth_backups_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&self.paths.auth_backups_dir) {
                for entry in entries.flatten() {
                    std::fs::remove_file(entry.path())?;
                    auth_backups_removed += 1;
                }
            }
        }

        if self.paths.registry_backups_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&self.paths.registry_backups_dir) {
                for entry in entries.flatten() {
                    std::fs::remove_file(entry.path())?;
                    registry_backups_removed += 1;
                }
            }
        }

        if !registry
            .items
            .iter()
            .any(|i| Some(&i.account_key) == registry.active_account_key.as_ref())
        {
            registry.active_account_key = None;
        }
        registry.updated_at = current_timestamp();
        self.persist_registry(&registry, false)?;

        let mut quota_store = quota_store::load_or_default(&self.paths.quota_store_path);
        let original_quota_count = quota_store.items.len();
        quota_store
            .items
            .retain(|item| live_account_keys.contains(&item.account_key));
        if quota_store.items.len() != original_quota_count {
            quota_store.updated_at = current_timestamp();
            self.save_quota_store(&quota_store)?;
        }

        Ok(CoreEnvelope::ok(CleanPayload {
            auth_backups_removed,
            registry_backups_removed,
            stale_entries_removed: (original_count - registry.items.len()) as i32,
        }))
    }

    pub fn rebuild_registry(&self) -> Result<CoreEnvelope<RebuildRegistryPayload>, CoreError> {
        let registry = self.rebuild_registry_state()?;
        Ok(CoreEnvelope::ok(RebuildRegistryPayload {
            account_count: registry.items.len() as i32,
            active_account_key: registry.active_account_key,
            registry_updated: true,
        }))
    }

    pub fn set_auto_switch(
        &self,
        enabled: bool,
    ) -> Result<CoreEnvelope<AutoSwitchConfigPayload>, CoreError> {
        if enabled {
            return Err(CoreError::OperationFailed(
                "Automatic account switching is not available in this source build".into(),
            ));
        }
        let mut registry = self.load_registry_or_empty()?;
        let mut config = registry.auto_switch.clone().unwrap_or_default();
        config.enabled = enabled;
        registry.auto_switch = Some(config);
        registry.updated_at = current_timestamp();
        self.save_registry(&registry)?;

        if enabled {
            let daemon_binary = self.resolve_daemon_binary()?;
            crate::platform::daemon::install_daemon(
                &self.paths.launch_agent_path,
                &daemon_binary,
                &self.paths.codex_home,
            )?;
        } else {
            crate::platform::daemon::uninstall_daemon(&self.paths.launch_agent_path)?;
            let _ = self.clear_auto_switch_transient_state();
        }

        let payload = self.make_auto_switch_status(&registry);
        Ok(CoreEnvelope::ok(AutoSwitchConfigPayload {
            auto_switch: payload,
        }))
    }

    pub fn configure_auto_switch(
        &self,
        threshold_5h: Option<i32>,
        threshold_weekly: Option<i32>,
    ) -> Result<CoreEnvelope<AutoSwitchConfigPayload>, CoreError> {
        let mut registry = self.load_registry_or_empty()?;
        let mut config = registry.auto_switch.clone().unwrap_or_default();

        if let Some(t) = threshold_5h {
            if !(0..=100).contains(&t) {
                return Err(CoreError::InvalidData(format!(
                    "5h threshold must be 0-100, got {t}"
                )));
            }
            config.threshold_5h_percent = t;
        }
        if let Some(t) = threshold_weekly {
            if !(0..=100).contains(&t) {
                return Err(CoreError::InvalidData(format!(
                    "weekly threshold must be 0-100, got {t}"
                )));
            }
            config.threshold_weekly_percent = t;
        }

        registry.auto_switch = Some(config);
        registry.updated_at = current_timestamp();
        self.save_registry(&registry)?;

        let payload = self.make_auto_switch_status(&registry);
        Ok(CoreEnvelope::ok(AutoSwitchConfigPayload {
            auto_switch: payload,
        }))
    }

    pub fn set_api_proxy_config(
        &self,
        mode: ApiProxyMode,
        url: Option<String>,
    ) -> Result<CoreEnvelope<ApiModePayload>, CoreError> {
        let normalized =
            crate::core::api_client::sanitize_proxy_config(&ApiProxyConfigPayload { mode, url })?;
        let mut settings = self.load_settings();
        settings.api_proxy = normalized.clone();
        self.save_settings(&settings)?;

        Ok(CoreEnvelope::ok(ApiModePayload {
            api: ApiConfigPayload { proxy: normalized },
        }))
    }

    pub fn test_api_proxy_config(
        &self,
        mode: ApiProxyMode,
        url: Option<String>,
    ) -> Result<CoreEnvelope<ApiProxyTestPayload>, CoreError> {
        let payload =
            crate::core::api_client::test_api_connectivity(&ApiProxyConfigPayload { mode, url });
        Ok(CoreEnvelope::ok(payload))
    }

    pub fn detect_api_proxy_config(
        &self,
    ) -> Result<CoreEnvelope<ApiProxyDetectPayload>, CoreError> {
        let payload = crate::core::api_client::detect_api_proxy_config();
        Ok(CoreEnvelope::ok(payload))
    }

    pub fn diagnose(&self) -> Result<CoreEnvelope<DiagnosePayload>, CoreError> {
        let state = self.load_local_state()?;
        Ok(CoreEnvelope::ok(DiagnosePayload {
            paths: state.paths,
            core_version: env!("CARGO_PKG_VERSION").into(),
            platform: DiagnosePlatform {
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
            },
            registry_state: DiagnoseRegistryState {
                account_count: state.registry.items.len() as i32,
            },
            session_state: DiagnoseSessionState {
                latest_rollout_found: false,
            },
            api_state: DiagnoseApiState {
                usage_attempt_count: state.api_diagnostics.usage_attempt_count,
                usage_success_count: state.api_diagnostics.usage_success_count,
                name_attempt_count: state.api_diagnostics.name_attempt_count,
                name_success_count: state.api_diagnostics.name_success_count,
                last_usage_failure: state.api_diagnostics.last_usage_failure,
                last_usage_failure_account: state.api_diagnostics.last_usage_failure_account,
                last_name_failure: state.api_diagnostics.last_name_failure,
                last_name_failure_account: state.api_diagnostics.last_name_failure_account,
            },
        }))
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    pub(crate) fn load_local_state(&self) -> Result<LoadedState, CoreError> {
        let mut warnings: Vec<CoreWarning> = Vec::new();
        let settings = self.load_settings();
        let registry = self.load_registry_or_empty()?;
        if !self.paths.registry_path.exists() {
            warnings.push(CoreWarning {
                code: "REGISTRY_MISSING".into(),
                message: "No local account registry found yet.".into(),
            });
        }

        let last_scan_at = current_timestamp();

        let app_paths = AppPathState {
            codex_home: self.paths.codex_home.display().to_string(),
            accounts_path: self.paths.accounts_dir.display().to_string(),
            auth_path: self.paths.auth_path.display().to_string(),
            registry_path: self.paths.registry_path.display().to_string(),
            sessions_path: self.paths.sessions_dir.display().to_string(),
            launch_agent_path: self.paths.launch_agent_path.display().to_string(),
            auto_switch_log_path: self.paths.auto_switch_log_path.display().to_string(),
            auth_exists: self.paths.auth_path.exists(),
            registry_exists: self.paths.registry_path.exists(),
            sessions_exists: self.paths.sessions_dir.exists(),
        };

        Ok(LoadedState {
            paths: app_paths,
            settings,
            registry,
            last_scan_at,
            warnings,
            api_diagnostics: ApiDiagnostics::default(),
            active_usage_api_status: ApiReachabilityStatus::Unknown,
            active_usage_api_last_error: None,
        })
    }

    pub(crate) fn make_status_payload(&self, state: &LoadedState) -> AppStatusPayload {
        self.make_status_payload_with_service_state(state, None)
    }

    pub(crate) fn make_status_payload_with_service_state(
        &self,
        state: &LoadedState,
        service_state: Option<AutoSwitchRuntimeState>,
    ) -> AppStatusPayload {
        let auto_switch =
            self.make_auto_switch_status_with_service_state(&state.registry, service_state);
        AppStatusPayload {
            paths: state.paths.clone(),
            last_scan_at: state.last_scan_at,
            usage_source: UsageSource::Local,
            auto_switch,
            api: ApiConfigPayload {
                proxy: state.settings.api_proxy.clone(),
            },
            api_connectivity: ApiConnectivityPayload {
                usage_status: state.active_usage_api_status.clone(),
                usage_last_error: state.active_usage_api_last_error.clone(),
            },
        }
    }

    fn make_auto_switch_status(&self, registry: &RegistryFile) -> AutoSwitchStatusPayload {
        self.make_auto_switch_status_with_service_state(registry, None)
    }

    fn make_auto_switch_status_with_service_state(
        &self,
        registry: &RegistryFile,
        service_state: Option<AutoSwitchRuntimeState>,
    ) -> AutoSwitchStatusPayload {
        let config = registry.auto_switch.clone().unwrap_or_default();
        AutoSwitchStatusPayload {
            supported: false,
            enabled: false,
            threshold_5h_percent: config.threshold_5h_percent,
            threshold_weekly_percent: config.threshold_weekly_percent,
            service_state: service_state.unwrap_or_else(|| {
                crate::platform::daemon::check_daemon_state(&self.paths.launch_agent_path)
            }),
            service_label: "dev.agentharbor.auto-switch".into(),
        }
    }

    fn load_registry(&self) -> Result<RegistryFile, CoreError> {
        let data = std::fs::read_to_string(&self.paths.registry_path)?;
        let registry: RegistryFile = serde_json::from_str(&data)?;
        Ok(registry)
    }

    fn load_registry_or_empty(&self) -> Result<RegistryFile, CoreError> {
        if self.paths.registry_path.exists() {
            return self.load_registry();
        }
        Ok(RegistryFile {
            schema_version: REGISTRY_SCHEMA_VERSION,
            updated_at: current_timestamp(),
            active_account_key: None,
            items: vec![],
            auto_switch: Some(AutoSwitchConfig::default()),
            api: Some(ApiConfig::default()),
        })
    }

    pub(crate) fn save_registry(&self, registry: &RegistryFile) -> Result<(), CoreError> {
        self.persist_registry(registry, true)
    }

    fn persist_registry(
        &self,
        registry: &RegistryFile,
        create_backup: bool,
    ) -> Result<(), CoreError> {
        self.paths.ensure_directories()?;

        if create_backup && self.paths.registry_path.exists() {
            let backup_name = format!("registry-{}.json", current_timestamp());
            let backup_path = self.paths.registry_backups_dir.join(backup_name);
            let _ = std::fs::copy(&self.paths.registry_path, &backup_path);
        }

        let mut reg = registry.clone();
        reg.schema_version = reg.schema_version.max(REGISTRY_SCHEMA_VERSION);
        if reg.auto_switch.is_none() {
            reg.auto_switch = Some(AutoSwitchConfig::default());
        }
        if reg.api.is_none() {
            reg.api = Some(ApiConfig::default());
        }

        let data = serde_json::to_string_pretty(&reg)?;
        super::storage::write_atomic(&self.paths.registry_path, data.as_bytes())?;
        Ok(())
    }

    pub(crate) fn save_quota_store(&self, quota_store: &QuotaStoreFile) -> Result<(), CoreError> {
        quota_store::save(&self.paths.quota_store_path, quota_store)
    }

    fn rebuild_registry_state(&self) -> Result<RegistryFile, CoreError> {
        self.paths.ensure_directories()?;
        let prev = self.load_registry().ok();
        let mut registry = RegistryFile {
            schema_version: REGISTRY_SCHEMA_VERSION,
            updated_at: current_timestamp(),
            active_account_key: None,
            items: vec![],
            auto_switch: prev
                .as_ref()
                .and_then(|r| r.auto_switch.clone())
                .or(Some(AutoSwitchConfig::default())),
            api: prev
                .as_ref()
                .and_then(|r| r.api.clone())
                .or(Some(ApiConfig::default())),
        };

        if let Ok(auth_file) = load_auth_file(&self.paths.auth_path) {
            if let Ok(snapshot) = make_auth_snapshot(&auth_file, &self.paths.auth_path) {
                let snapshot_url = self.make_snapshot_path(&snapshot.account_key);
                let _ = std::fs::copy(&self.paths.auth_path, &snapshot_url);
                let mut item = RegistryItem {
                    account_key: snapshot.account_key.clone(),
                    snapshot_path: snapshot_url.display().to_string(),
                    email: snapshot.email,
                    alias: String::new(),
                    account_name: snapshot.account_name,
                    workspace_name: snapshot.workspace_name,
                    profile_name: snapshot.profile_name,
                    plan: format!("{:?}", snapshot.plan).to_lowercase(),
                    auth_mode: format!("{:?}", snapshot.auth_mode).to_lowercase(),
                    has_active_subscription: None,
                    subscription_expires_at: None,
                    subscription_will_renew: None,
                    created_at: snapshot.created_at,
                    last_used_at: Some(current_timestamp()),
                    last_usage_at: None,
                    cached_primary_window: None,
                    cached_secondary_window: None,
                };
                carry_over_registry_state(
                    &mut item,
                    prev_item(prev.as_ref(), &snapshot.account_key),
                );
                registry.items.push(item);
                registry.active_account_key = Some(snapshot.account_key);
            }
        }

        if self.paths.snapshots_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&self.paths.snapshots_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map_or(false, |e| e == "json") {
                        if let Ok(auth_file) = load_auth_file(&path) {
                            if let Ok(snapshot) = make_auth_snapshot(&auth_file, &path) {
                                if !registry
                                    .items
                                    .iter()
                                    .any(|i| i.account_key == snapshot.account_key)
                                {
                                    let account_key = snapshot.account_key;
                                    let mut item = RegistryItem {
                                        account_key: account_key.clone(),
                                        snapshot_path: path.display().to_string(),
                                        email: snapshot.email,
                                        alias: String::new(),
                                        account_name: snapshot.account_name,
                                        workspace_name: snapshot.workspace_name,
                                        profile_name: snapshot.profile_name,
                                        plan: format!("{:?}", snapshot.plan).to_lowercase(),
                                        auth_mode: format!("{:?}", snapshot.auth_mode)
                                            .to_lowercase(),
                                        has_active_subscription: None,
                                        subscription_expires_at: None,
                                        subscription_will_renew: None,
                                        created_at: snapshot.created_at,
                                        last_used_at: None,
                                        last_usage_at: None,
                                        cached_primary_window: None,
                                        cached_secondary_window: None,
                                    };
                                    carry_over_registry_state(
                                        &mut item,
                                        prev_item(prev.as_ref(), &account_key),
                                    );
                                    registry.items.push(item);
                                }
                            }
                        }
                    }
                }
            }
        }

        registry
            .items
            .sort_by(|a, b| a.email.to_lowercase().cmp(&b.email.to_lowercase()));
        self.save_registry(&registry)?;
        Ok(registry)
    }

    fn make_snapshot_path(&self, account_key: &str) -> PathBuf {
        let safe_name = account_key
            .replace('@', "_")
            .replace('/', "_")
            .replace(':', "_");
        self.paths.snapshots_dir.join(format!("{safe_name}.json"))
    }

    fn load_settings(&self) -> CodexMateSettings {
        let raw = match std::fs::read_to_string(&self.paths.settings_path) {
            Ok(s) => s,
            Err(_) => return CodexMateSettings::default(),
        };
        let v: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(j) => j,
            Err(_) => return CodexMateSettings::default(),
        };
        serde_json::from_value(v).unwrap_or_default()
    }

    pub(crate) fn load_bootstrap_state(&self) -> BootstrapStatePayload {
        bootstrap_cache::load(&self.paths.bootstrap_cache_path)
    }

    fn update_bootstrap_state<F>(&self, apply: F) -> Result<(), CoreError>
    where
        F: FnMut(&mut BootstrapStatePayload),
    {
        self.paths.ensure_directories()?;
        bootstrap_cache::update(&self.paths.bootstrap_cache_path, apply)
    }

    pub(crate) fn store_bootstrap_mcp_servers(
        &self,
        payload: &McpServerListPayload,
    ) -> Result<(), CoreError> {
        self.update_bootstrap_state(|cache| {
            cache.written_at = Some(current_timestamp());
            cache.mcp_servers = Some(payload.clone());
        })
    }

    pub(crate) fn store_bootstrap_installed_skills(
        &self,
        payload: &SkillListPayload,
    ) -> Result<(), CoreError> {
        self.update_bootstrap_state(|cache| {
            cache.written_at = Some(current_timestamp());
            cache.installed_skills = Some(payload.clone());
        })
    }

    fn save_settings(&self, settings: &CodexMateSettings) -> Result<(), CoreError> {
        self.paths.ensure_directories()?;
        let data = serde_json::to_string_pretty(settings)?;
        super::storage::write_atomic(&self.paths.settings_path, data.as_bytes())?;
        Ok(())
    }

    pub fn get_hotspot_enabled(&self) -> bool {
        false // The public source does not include the hotspot frontend.
    }

    pub fn set_hotspot_enabled(&self, enabled: bool) -> Result<(), CoreError> {
        if enabled {
            return Err(CoreError::OperationFailed(
                "Hotspot view is not available in this source build".into(),
            ));
        }
        let mut settings = self.load_settings();
        settings.hotspot = HotspotConfig { enabled };
        self.save_settings(&settings)?;
        Ok(())
    }

    pub fn get_usage_refresh_interval(&self) -> String {
        normalize_usage_refresh_interval(&self.load_settings().usage_refresh_interval)
            .unwrap_or("1m")
            .to_string()
    }

    pub fn set_usage_refresh_interval(&self, interval: &str) -> Result<String, CoreError> {
        let normalized = normalize_usage_refresh_interval(interval).ok_or_else(|| {
            CoreError::InvalidData(format!("Unsupported refresh interval: {interval}"))
        })?;
        let mut settings = self.load_settings();
        settings.usage_refresh_interval = normalized.to_string();
        self.save_settings(&settings)?;
        Ok(normalized.to_string())
    }

    pub fn get_or_create_device_id(&self) -> Result<String, CoreError> {
        let mut settings = self.load_settings();
        if let Some(ref id) = settings.device_id {
            return Ok(id.clone());
        }
        let id = uuid::Uuid::new_v4().to_string();
        settings.device_id = Some(id.clone());
        self.save_settings(&settings)?;
        Ok(id)
    }

    fn resolve_daemon_binary(&self) -> Result<PathBuf, CoreError> {
        std::env::current_exe().map_err(|e| {
            CoreError::OperationFailed(format!("Failed to resolve current executable: {e}"))
        })
    }

    fn clear_auto_switch_transient_state(&self) -> Result<(), CoreError> {
        let _ = std::fs::remove_file(&self.paths.auto_switch_pending_path);
        let _ = std::fs::remove_file(&self.paths.auto_switch_snooze_path);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Registry helpers
// ---------------------------------------------------------------------------

fn prev_item<'a>(
    registry: Option<&'a RegistryFile>,
    account_key: &str,
) -> Option<&'a RegistryItem> {
    registry.and_then(|registry| {
        registry
            .items
            .iter()
            .find(|item| item.account_key == account_key)
    })
}

fn carry_over_registry_state(item: &mut RegistryItem, previous: Option<&RegistryItem>) {
    let Some(previous) = previous else { return };

    if item.alias.is_empty() && !previous.alias.is_empty() {
        item.alias = previous.alias.clone();
    }
    if item.account_name.is_none() {
        item.account_name = previous.account_name.clone();
    }
    if item.workspace_name.is_none() {
        item.workspace_name = previous.workspace_name.clone();
    }
    if item.profile_name.is_none() {
        item.profile_name = previous.profile_name.clone();
    }
    if previous.has_active_subscription.is_some() {
        item.has_active_subscription = previous.has_active_subscription;
    }
    if previous.subscription_expires_at.is_some() {
        item.subscription_expires_at = previous.subscription_expires_at;
    }
    if previous.subscription_will_renew.is_some() {
        item.subscription_will_renew = previous.subscription_will_renew;
    }
    if let Some(previous_last_used_at) = previous.last_used_at {
        item.last_used_at = Some(
            item.last_used_at
                .unwrap_or(previous_last_used_at)
                .max(previous_last_used_at),
        );
    }
    if let Some(previous_last_usage_at) = previous.last_usage_at {
        item.last_usage_at = Some(
            item.last_usage_at
                .unwrap_or(previous_last_usage_at)
                .max(previous_last_usage_at),
        );
    }
    if previous.cached_primary_window.is_some() {
        item.cached_primary_window = previous.cached_primary_window.clone();
    }
    if previous.cached_secondary_window.is_some() {
        item.cached_secondary_window = previous.cached_secondary_window.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_test_repo(label: &str) -> (Repository, PathBuf) {
        let codex_home = std::env::temp_dir().join(format!(
            "agentharbor-repository-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let repo = Repository {
            paths: CodexPaths::from_home(codex_home.clone()),
        };
        repo.paths.ensure_directories().unwrap();
        (repo, codex_home)
    }

    fn write_test_snapshot(path: &Path) -> AuthSnapshot {
        let auth = AuthFile {
            auth_mode: Some("chatgpt".into()),
            openai_api_key: None,
            tokens: AuthTokens {
                id_token: None,
                access_token: None,
                refresh_token: None,
                account_id: Some("test-account".into()),
            },
            last_refresh: Some("2026-04-06T00:00:00Z".into()),
        };
        fs::write(path, serde_json::to_string_pretty(&auth).unwrap()).unwrap();
        make_auth_snapshot(&auth, path).unwrap()
    }

    fn make_registry_item(snapshot: &AuthSnapshot, snapshot_path: &Path) -> RegistryItem {
        RegistryItem {
            account_key: snapshot.account_key.clone(),
            snapshot_path: snapshot_path.display().to_string(),
            email: snapshot.email.clone(),
            alias: String::new(),
            account_name: snapshot.account_name.clone(),
            workspace_name: snapshot.workspace_name.clone(),
            profile_name: snapshot.profile_name.clone(),
            plan: format!("{:?}", snapshot.plan).to_lowercase(),
            auth_mode: format!("{:?}", snapshot.auth_mode).to_lowercase(),
            has_active_subscription: None,
            subscription_expires_at: None,
            subscription_will_renew: None,
            created_at: snapshot.created_at,
            last_used_at: Some(snapshot.created_at),
            last_usage_at: None,
            cached_primary_window: None,
            cached_secondary_window: None,
        }
    }

    fn count_files(dir: &Path) -> usize {
        fs::read_dir(dir)
            .map(|entries| entries.count())
            .unwrap_or(0)
    }

    #[test]
    fn clean_does_not_recreate_registry_backups() {
        let (repo, codex_home) = make_test_repo("clean");
        let snapshot_path = repo.paths.snapshots_dir.join("clean.json");
        let snapshot = write_test_snapshot(&snapshot_path);

        let registry = RegistryFile {
            schema_version: REGISTRY_SCHEMA_VERSION,
            updated_at: current_timestamp(),
            active_account_key: None,
            items: vec![make_registry_item(&snapshot, &snapshot_path)],
            auto_switch: Some(AutoSwitchConfig::default()),
            api: Some(ApiConfig::default()),
        };
        repo.persist_registry(&registry, false).unwrap();
        fs::write(
            repo.paths.registry_backups_dir.join("old-backup.json"),
            "{}",
        )
        .unwrap();

        let result = repo.clean().unwrap();
        assert_eq!(result.data.registry_backups_removed, 1);
        assert_eq!(count_files(&repo.paths.registry_backups_dir), 0);

        let _ = fs::remove_dir_all(codex_home);
    }

    #[test]
    fn set_api_proxy_config_persists_to_settings_and_status() {
        let (repo, codex_home) = make_test_repo("api-proxy");

        repo.set_api_proxy_config(ApiProxyMode::Manual, Some("socks5://127.0.0.1:7890".into()))
            .unwrap();

        let settings = repo.load_settings();
        assert_eq!(settings.api_proxy.mode, ApiProxyMode::Manual);
        assert_eq!(
            settings.api_proxy.url.as_deref(),
            Some("socks5://127.0.0.1:7890")
        );

        let snapshot = repo.load_snapshot_local().unwrap().data;
        assert_eq!(snapshot.status.api.proxy.mode, ApiProxyMode::Manual);
        assert_eq!(
            snapshot.status.api.proxy.url.as_deref(),
            Some("socks5://127.0.0.1:7890")
        );

        let _ = fs::remove_dir_all(codex_home);
    }
}
