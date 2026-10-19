//! Client-specific config adapters share one reversible file transaction boundary.
//! This module depends on provider data and the models.dev catalog (read from the
//! app data directory). An optional official snapshot source is supplied by
//! the caller, so model inputs match the Codex adapters without owning accounts.
mod aggregate_models;
mod claude;
mod claude_official;
mod harness;
mod jsonc;
mod mixed_models;
mod model_facts;
mod opencode;
mod pi;
mod options;
mod owned;
mod takeover;
mod version;
pub use claude_official::ClaudeOfficialPreview;
pub use harness::{HarnessInfo, HarnessProfile, HarnessReasoning};
use options::{client_endpoint, validate_model};
pub use options::{ClaudeAuthField, ClaudeModels, ClaudeOneM, ClientAccessMode, ClientOptions};
#[cfg(test)]
mod tests;

use super::{
    model_catalog,
    models::{ApiProxyConfigPayload, ApiProxyMode},
    models_dev,
    router::{ModelProtocol, Provider, Store},
    secret_file, storage,
    targets::TargetId,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CLAUDE: claude::ClaudeCode = claude::ClaudeCode;
static OPENCODE: opencode::OpenCode = opencode::OpenCode;
static PI: pi::Pi = pi::Pi;

#[derive(Clone)]
pub struct ClientPaths {
    pub claude: PathBuf,
    pub claude_user: PathBuf,
    pub opencode: PathBuf,
    pub pi: PathBuf,
    /// Cursor IDE's user directory (`.../Cursor/User`).
    pub cursor: PathBuf,
    /// `cursor-agent`'s directory (`~/.cursor`).
    pub cursor_cli: PathBuf,
    /// DeepSeek Harness's home (`$DSH_HOME`, else `~/.dsh`).
    pub harness: PathBuf,
}

impl ClientPaths {
    pub fn claude_root(&self) -> &Path {
        self.claude.parent().expect("Claude settings has a parent")
    }

    /// Claude uses ~/.claude.json by default; an overridden config directory
    /// stores the user configuration inside that directory as .claude.json.
    pub fn claude_user_config(&self) -> PathBuf {
        self.claude_user.clone()
    }

    pub fn discover() -> Self {
        if let Some(home) = env_path("AGENT_HARBOR_CLIENT_HOME") {
            return Self::from_home(&home);
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config = env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let default_claude_root = home.join(".claude");
        let claude_root =
            env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| default_claude_root.clone());
        Self {
            claude_user: claude_user_path(&home, &claude_root),
            claude: claude_root.join("settings.json"),
            opencode: env_path("OPENCODE_CONFIG")
                .unwrap_or_else(|| opencode_path(&config.join("opencode"))),
            pi: pi::models_path(&home, std::env::var_os("PI_CODING_AGENT_DIR")),
            cursor: super::cursor_inject::ide::user_dir().unwrap_or_else(|| config.join("Cursor").join("User")),
            cursor_cli: home.join(".cursor"),
            harness: harness::home(&home, std::env::var_os("DSH_HOME")),
        }
    }

    pub fn from_home(home: &Path) -> Self {
        Self {
            claude: home.join(".claude/settings.json"),
            claude_user: home.join(".claude.json"),
            opencode: opencode_path(&home.join(".config/opencode")),
            pi: home.join(".pi/agent/models.json"),
            cursor: home.join(".config/Cursor/User"),
            cursor_cli: home.join(".cursor"),
            harness: home.join(".dsh"),
        }
    }
}

fn claude_user_path(home: &Path, root: &Path) -> PathBuf {
    if root == home.join(".claude") {
        home.join(".claude.json")
    } else {
        root.join(".claude.json")
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn opencode_path(dir: &Path) -> PathBuf {
    ["opencode.jsonc", "opencode.json", "config.json"]
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| dir.join("opencode.json"))
}

/// Per-apply input the adapters need beyond the request itself.
pub(super) struct EditContext<'a> {
    /// One-time cleanup of plain `{"model","label"}` picker rows left by older
    /// builds: only possible with no journal and no ownership record, where
    /// nothing else identifies them. Once a record exists it does the
    /// withdrawing, and user rows must never be shape-guessed.
    pub legacy_picker_cleanup: bool,
    /// Non-blocking notices the adapter wants surfaced on the preview.
    pub warnings: &'a mut Vec<String>,
    /// Provider nodes already owned by this session; new external nodes must
    /// never be silently taken over by an additive adapter.
    pub managed_provider_keys: Vec<String>,
    /// The proxy a direct configuration's client reaches its relay through.
    pub proxy: Option<String>,
    /// The `NO_PROXY` list a direct card's client is given while the terminal
    /// proxy reaches it: its relay alone skips that proxy.
    pub bypass: Option<String>,
}

trait ClientAdapter: Sync {
    fn id(&self) -> TargetId;
    fn name(&self) -> &'static str;
    fn protocols(&self) -> &'static [ModelProtocol];
    fn path<'a>(&self, paths: &'a ClientPaths) -> &'a Path;
    /// Whether the adapter writes context/output limits into the client
    /// configuration. Claude Code ignores them, so missing catalog metadata
    /// must not block its injection.
    fn needs_limits(&self) -> bool {
        true
    }
    fn edits(
        &self,
        document: &Value,
        store: &Store,
        request: &ConfigRequest,
        catalog: &models_dev::Catalog,
        official_models: &[Value],
        ctx: &mut EditContext<'_>,
    ) -> Result<Vec<Edit>, String>;
}

fn adapter(id: TargetId) -> Result<&'static dyn ClientAdapter, String> {
    match id {
        TargetId::ClaudeCode => Ok(&CLAUDE),
        TargetId::OpenCode => Ok(&OPENCODE),
        TargetId::Pi => Ok(&PI),
        // Harness edits two YAML files through its own transaction (harness/).
        TargetId::Codex | TargetId::Harness => Err("clients.errors.unsupported".into()),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequest {
    pub target_id: TargetId,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
    /// Explicit user-entered prompt budget. `None` follows the resolver
    /// (user overrides, upstream declaration, catalog, then the shared
    /// fallback), so a catalog change can move an automatic value while a
    /// user-entered one never moves.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// Explicit user-entered output cap; `None` follows the resolver.
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
    #[serde(flatten)]
    pub options: ClientOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStatus {
    pub target_id: TargetId,
    pub name: String,
    pub path: String,
    pub protocols: Vec<ModelProtocol>,
    pub configured: bool,
    pub conflict: bool,
    pub error: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub request: Option<ConfigRequest>,
    /// Managed paths that no longer match what the last apply wrote.
    pub conflict_fields: Vec<String>,
    /// Claude Code CLI version when it could be detected.
    pub client_version: Option<String>,
    /// A direct configuration whose relay card changed address or key since
    /// it was written (or was deleted): the file still holds the old values.
    pub stale: bool,
    /// DeepSeek Harness's home and profiles, for its card only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessInfo>,
    /// A router configuration withdrawn when AgentHarbor last quit (or could
    /// not take its port) waits to be applied again; `request` then holds it.
    pub resume_pending: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPreview {
    pub path: String,
    pub revision: String,
    /// Only the managed fields are returned; all credential values are redacted.
    pub content: String,
    pub removed_fields: Vec<String>,
    /// Managed fields the user changed since the last apply: applying replaces them.
    pub conflict_fields: Vec<String>,
    /// i18n keys for non-blocking notices: fields the first injection will
    /// overwrite, and higher-priority configuration that would override it.
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigApplied {
    pub path: String,
    pub backup_path: String,
    /// i18n keys for non-blocking notices (takeover, managed settings, router
    /// reconciliation failures). Empty when everything was quiet.
    pub warnings: Vec<String>,
}

struct Edit {
    path: Vec<String>,
    value: Option<Value>,
}

fn edit(path: &[&str], value: Value) -> Edit {
    Edit {
        path: path.iter().map(|s| (*s).to_owned()).collect(),
        value: Some(value),
    }
}

fn remove(path: &[&str]) -> Edit {
    Edit {
        path: path.iter().map(|key| (*key).to_owned()).collect(),
        value: None,
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct FieldChange {
    path: Vec<String>,
    before: Option<Value>,
    after: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    path: PathBuf,
    original: Option<String>,
    installed: Value,
    changes: Vec<FieldChange>,
    request: ConfigRequest,
    /// A direct configuration's cards (address and key) as written; the
    /// router reads them live, so a router configuration keeps none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    connection: Option<String>,
}

/// The network a direct configuration is checked against. A direct client
/// reaches its relay itself, never through this app's proxy.
#[derive(Default)]
pub struct Network {
    /// The app proxy, which relay cards follow unless set to direct.
    pub proxy: ApiProxyConfigPayload,
    /// The terminal proxy while new terminals carry it.
    pub terminal: Option<crate::core::terminal_proxy::ActiveProxy>,
    /// Whether a system proxy is set, for the app proxy's system mode.
    pub system: bool,
}

/// What a direct Claude Code configuration does about its card's proxy.
/// Claude Code reads proxy variables only: never the system proxy, and no
/// SOCKS.
#[derive(Debug, PartialEq)]
enum DirectProxy {
    /// Nothing: router mode, a card set to direct, no app proxy, or the
    /// terminal proxy already carries one into new terminals.
    None,
    /// The app's manual HTTP proxy, written as the client's HTTPS_PROXY.
    Write(String),
    /// A direct card under the terminal proxy: its relay host joins the
    /// terminal proxy's bypass list, written as the client's NO_PROXY.
    Bypass(String),
    /// A manual SOCKS proxy, which Claude Code cannot use.
    Socks,
    /// The system proxy, which Claude Code does not read.
    System,
}

pub struct ConfigManager {
    data_dir: PathBuf,
    /// App data directory, used to read the models.dev catalog snapshot.
    app_data_dir: PathBuf,
    official_home: Option<PathBuf>,
    paths: ClientPaths,
    network: Network,
}

impl ConfigManager {
    pub fn new(data_dir: &Path, paths: ClientPaths) -> Self {
        Self {
            data_dir: data_dir.join("client-config"),
            app_data_dir: data_dir.to_path_buf(),
            official_home: None,
            paths,
            network: Network::default(),
        }
    }

    pub fn with_official_home(mut self, home: &Path) -> Self {
        self.official_home = Some(home.to_path_buf());
        self
    }

    pub fn with_network(mut self, network: Network) -> Self {
        self.network = network;
        self
    }

    fn direct_proxy(&self, store: &Store, request: &ConfigRequest) -> DirectProxy {
        if request.target_id != TargetId::ClaudeCode || request.needs_router() {
            return DirectProxy::None;
        }
        let Ok(provider) = selected_provider(store, request) else {
            return DirectProxy::None;
        };
        if let Some(terminal) = &self.network.terminal {
            // New terminals already carry the proxy; a direct relay skips it.
            return match (provider.network_mode(), provider.relay_host()) {
                ("direct", Some(host)) => DirectProxy::Bypass(
                    terminal.no_proxy.split(',').map(str::trim).filter(|entry| !entry.is_empty())
                        .chain(std::iter::once(host.as_str()))
                        .fold(Vec::<&str>::new(), |mut entries, entry| {
                            if !entries.iter().any(|known| known.eq_ignore_ascii_case(entry)) {
                                entries.push(entry);
                            }
                            entries
                        })
                        .join(","),
                ),
                _ => DirectProxy::None,
            };
        }
        if provider.network_mode() != "proxy" {
            return DirectProxy::None;
        }
        match self.network.proxy.mode {
            ApiProxyMode::Manual => {
                let Some(url) = self.network.proxy.url.as_deref().map(str::trim).filter(|url| !url.is_empty()) else {
                    return DirectProxy::None;
                };
                match reqwest::Url::parse(url) {
                    Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => DirectProxy::Write(url.to_owned()),
                    Ok(_) => DirectProxy::Socks,
                    Err(_) => DirectProxy::None,
                }
            }
            ApiProxyMode::System if self.network.system => DirectProxy::System,
            _ => DirectProxy::None,
        }
    }

    pub fn list(&self) -> Vec<ClientStatus> {
        self.list_with(None)
    }

    /// The clients, each direct configuration checked against `store` for a
    /// card changed since it was written.
    pub fn list_with(&self, store: Option<&Store>) -> Vec<ClientStatus> {
        [TargetId::ClaudeCode, TargetId::OpenCode, TargetId::Pi]
            .into_iter()
            .map(|id| {
                let adapter = adapter(id).expect("registered adapter");
                let mut status = ClientStatus {
                    target_id: adapter.id(),
                    name: adapter.name().into(),
                    path: adapter.path(&self.paths).to_string_lossy().into(),
                    protocols: adapter.protocols().to_vec(),
                    configured: false,
                    conflict: false,
                    error: None,
                    provider_id: None,
                    model: None,
                    request: None,
                    conflict_fields: Vec::new(),
                    client_version: None,
                    stale: false,
                    harness: None,
                    resume_pending: false,
                };
                if id == TargetId::ClaudeCode {
                    status.client_version = version::detected();
                }
                let result = (|| -> Result<(), String> {
                    let session = self.session(id)?;
                    if let Some(session) = &session {
                        status.configured = true;
                        status.path = session.path.to_string_lossy().into();
                        status.provider_id = Some(session.request.provider_id.clone());
                        status.model = Some(session.request.model.clone());
                        status.request = Some(session.request.clone());
                        status.stale = match (store, &session.connection) {
                            (Some(store), Some(written)) => {
                                let proxy = self.direct_proxy(store, &session.request);
                                connection_fingerprint(store, &session.request, &proxy) != *written
                            }
                            _ => false,
                        };
                        let current = parse(read_text(&session.path)?.as_deref(), id)?;
                        let conflict_paths = conflicting_paths(&current, session);
                        status.conflict = !conflict_paths.is_empty();
                        status.conflict_fields = conflict_paths
                            .iter()
                            .map(|path| path.join("."))
                            .collect();
                    } else {
                        if let Some(parked) = self.parked(id) {
                            status.resume_pending = true;
                            status.request = Some(parked);
                        }
                        parse(read_text(adapter.path(&self.paths))?.as_deref(), id)?;
                    }
                    Ok(())
                })();
                status.error = result.err();
                status
            })
            .chain(std::iter::once(self.harness_status(store)))
            .collect()
    }

    fn session_path(&self, id: TargetId) -> PathBuf {
        self.data_dir.join(match id {
            TargetId::ClaudeCode => "claude-code.dat",
            TargetId::OpenCode => "opencode.dat",
            TargetId::Pi => "pi.dat",
            TargetId::Harness => "harness.dat",
            TargetId::Codex => "codex.dat",
        })
    }

    fn parked_path(&self) -> PathBuf {
        self.data_dir.join("router-resume.dat")
    }

    fn parked_requests(&self) -> Vec<ConfigRequest> {
        secret_file::read(&self.parked_path())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// The router configuration of `id` waiting to be applied again.
    pub(super) fn parked(&self, id: TargetId) -> Option<ConfigRequest> {
        self.parked_requests()
            .into_iter()
            .find(|request| request.target_id == id)
    }

    /// Keeps `request` (or nothing) as the configuration `id` resumes with.
    fn park(&self, id: TargetId, request: Option<ConfigRequest>) -> Result<(), String> {
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().map_err(|_| "clients.errors.storage")?;
        let mut parked = self.parked_requests();
        if request.is_none() && !parked.iter().any(|entry| entry.target_id == id) {
            return Ok(());
        }
        parked.retain(|entry| entry.target_id != id);
        parked.extend(request);
        let path = self.parked_path();
        if parked.is_empty() {
            return storage::remove_file_durable(&path).map_err(|_| "clients.errors.storage".into());
        }
        secret_file::write(&path, &serde_json::to_vec(&parked).map_err(|_| "clients.errors.storage")?)
    }

    /// Forgets the configuration `id` would resume with.
    pub fn discard_resume(&self, id: TargetId) -> Result<(), String> {
        self.park(id, None)
    }

    pub fn has_parked(&self) -> bool {
        !self.parked_requests().is_empty()
    }

    /// Applies again what [`Self::restore_router_configs_on_exit`] withdrew,
    /// once this process holds the router's port: a client routed before a
    /// restart is routed after it, as Cursor is. A client configured since
    /// keeps its configuration; one that cannot be applied stays waiting.
    pub fn resume_parked(&self, store: &Store) -> Vec<(TargetId, String)> {
        let mut failures = Vec::new();
        for request in self.parked_requests() {
            let id = request.target_id;
            let result = (|| -> Result<(), String> {
                let _guard = WRITE_LOCK
                    .get_or_init(|| Mutex::new(()))
                    .lock()
                    .map_err(|_| "clients.errors.storage")?;
                if id == TargetId::Harness {
                    if self.harness_stored_request()?.is_none() {
                        self.harness_resume(store, &request)?;
                    }
                    return Ok(());
                }
                if self.session(id)?.is_some() {
                    return Ok(());
                }
                let plan = self.prepare(store, &request, false, false)?;
                let text = render(id, plan.current.as_deref(), &plan.session.installed)?;
                self.commit(id, &plan.session.path, plan.current, Some(text), Some(&plan.session), plan.warnings)
                    .map(|_| ())
            })();
            match result {
                Ok(()) => {
                    if let Err(error) = self.park(id, None) {
                        failures.push((id, error));
                    }
                }
                Err(error) => failures.push((id, error)),
            }
        }
        failures
    }

    fn session(&self, id: TargetId) -> Result<Option<Session>, String> {
        secret_file::read(&self.session_path(id))?
            .map(|bytes| {
                let mut value: Value = serde_json::from_slice(&bytes)
                    .map_err(|_| "clients.errors.storage".to_string())?;
                // Existing installations used the gateway before access mode was explicit.
                if let Some(request) = value.get_mut("request").and_then(Value::as_object_mut) {
                    request
                        .entry("accessMode")
                        .or_insert_with(|| json!("router"));
                }
                serde_json::from_value(value).map_err(|_| "clients.errors.storage".into())
            })
            .transpose()
            .map(Option::flatten)
    }

    pub fn preview(&self, store: &Store, request: &ConfigRequest) -> Result<ConfigPreview, String> {
        self.preview_with_conflicts(store, request, false)
    }

    pub fn preview_with_conflicts(
        &self,
        store: &Store,
        request: &ConfigRequest,
        replace_conflicts: bool,
    ) -> Result<ConfigPreview, String> {
        if request.target_id == TargetId::Harness {
            return self.harness_preview(store, request, replace_conflicts);
        }
        let plan = self.prepare(store, request, false, replace_conflicts)?;
        let mut patch = json!({});
        for change in &plan.session.changes {
            set_field(&mut patch, &change.path, change.after.clone())?;
        }
        redact(&mut patch);
        Ok(ConfigPreview {
            path: plan.session.path.to_string_lossy().into(),
            revision: plan.revision,
            content: serde_json::to_string_pretty(&patch).map_err(|_| "clients.errors.storage")?,
            removed_fields: plan
                .session
                .changes
                .iter()
                .filter(|change| change.after.is_none())
                .map(|change| change.path.join("."))
                .collect(),
            conflict_fields: plan.conflicts,
            warnings: plan.warnings,
        })
    }

    pub fn apply(
        &self,
        store: &Store,
        request: &ConfigRequest,
        expected_revision: &str,
    ) -> Result<ConfigApplied, String> {
        self.apply_with_conflicts(store, request, expected_revision, false)
    }

    /// Explicit, previewed replacement keeps the original rollback point.
    /// The commit backs up the current file before replacing managed fields.
    pub fn apply_with_conflicts(
        &self,
        store: &Store,
        request: &ConfigRequest,
        expected_revision: &str,
        replace_conflicts: bool,
    ) -> Result<ConfigApplied, String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        if request.target_id == TargetId::Harness {
            let applied = self.harness_apply(store, request, expected_revision, replace_conflicts)?;
            let _ = self.park(TargetId::Harness, None);
            return Ok(applied);
        }
        let plan = self.prepare(store, request, false, replace_conflicts)?;
        if plan.revision != expected_revision {
            return Err("clients.errors.changed".into());
        }
        let text = render(request.target_id, plan.current.as_deref(), &plan.session.installed)?;
        let applied = self.commit(
            request.target_id,
            &plan.session.path,
            plan.current,
            Some(text),
            Some(&plan.session),
            plan.warnings,
        )?;
        let _ = self.park(request.target_id, None);
        Ok(applied)
    }

    pub fn restore(&self, id: TargetId) -> Result<ConfigApplied, String> {
        self.restore_confirmed(id, false)
    }

    /// Only an explicit restore confirmation may replace conflicting managed
    /// fields; without one a conflict refuses the restore.
    pub fn restore_confirmed(
        &self,
        id: TargetId,
        replace_conflicts: bool,
    ) -> Result<ConfigApplied, String> {
        // Restoring by hand means the client should not come back on its own.
        let _ = self.park(id, None);
        if id == TargetId::Harness {
            let _guard = WRITE_LOCK.get_or_init(|| Mutex::new(())).lock().map_err(|_| "clients.errors.storage")?;
            return self.harness_restore(replace_conflicts);
        }
        self.restore_with(id, if replace_conflicts { ConflictPolicy::Replace } else { ConflictPolicy::Refuse })
    }

    fn restore_with(&self, id: TargetId, policy: ConflictPolicy) -> Result<ConfigApplied, String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        adapter(id)?;
        let mut session = self.session(id)?.ok_or("clients.errors.notConfigured")?;
        let current = read_text(&session.path)?;
        let document = parse(current.as_deref(), id)?;
        let mut warnings = Vec::new();
        match policy {
            ConflictPolicy::Refuse => verify_fields(&document, &session)?,
            ConflictPolicy::Replace => {}
            ConflictPolicy::KeepEdits => {
                let conflicts = conflicting_paths(&document, &session);
                if !conflicts.is_empty() {
                    // Another tool owns the connection now: the file no longer
                    // reaches this router, so it is left exactly as it is.
                    if session.changes.iter().any(|change| conflicts.contains(&change.path) && carries_connection(change)) {
                        return Err("clients.errors.conflict".into());
                    }
                    // The connection is still ours: withdraw it with every
                    // other field we own, and keep the user's edits.
                    session.changes.retain(|change| !conflicts.contains(&change.path));
                    warnings.push("clients.warnings.keptEdits".into());
                }
            }
        }
        let restored = if document == session.installed {
            // If there were no independent edits, restore exact bytes (including comments).
            session.original.clone()
        } else {
            let mut document = document;
            let mut owned = if owned::path(&self.data_dir, id).exists() {
                owned::load(&self.data_dir, id)
            } else {
                owned::OwnedRecord::seed_from_session(&session)
            };
            owned.recover_from_session(Some(&session));
            // Element arrays may contain independent user edits. Withdraw our
            // exact rows, instead of replacing the array with the old snapshot.
            let arrays: Vec<_> = session
                .changes
                .iter()
                .filter(|change| owned::is_element_array_path(&change.path))
                .map(|change| {
                    let current = field(&document, &change.path).cloned();
                    let restored = match current.as_ref().and_then(Value::as_array) {
                        Some(entries) => {
                            let entries = owned.withdraw(&change.path, entries);
                            (!entries.is_empty() || change.before.is_some()).then(|| json!(entries))
                        }
                        None => current,
                    };
                    (change.path.clone(), restored)
                })
                .collect();
            undo(&mut document, &session)?;
            for (path, restored) in arrays {
                set_field(&mut document, &path, restored)?;
            }
            Some(render(id, current.as_deref(), &document)?)
        };
        self.commit(id, &session.path, current, restored, None, warnings)
    }

    /// Re-inject the stored request without reopening the configuration
    /// dialog. With `rebaseline` the file's current managed values become the
    /// rollback point first — the explicit way out of a conflict, mirroring
    /// the Codex takeover path.
    pub fn reapply(
        &self,
        store: &Store,
        id: TargetId,
        rebaseline: bool,
    ) -> Result<ConfigApplied, String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        if id == TargetId::Harness {
            return self.harness_reapply(store, rebaseline);
        }
        adapter(id)?;
        let session = self.session(id)?.ok_or("clients.errors.notConfigured")?;
        let plan = self.prepare(store, &session.request, rebaseline, false)?;
        let text = render(id, plan.current.as_deref(), &plan.session.installed)?;
        self.commit(
            id,
            &plan.session.path,
            plan.current,
            Some(text),
            Some(&plan.session),
            plan.warnings,
        )
    }

    /// The stored request for a client; reapply uses it to decide whether the
    /// local router must be running.
    pub fn stored_request(&self, id: TargetId) -> Result<Option<ConfigRequest>, String> {
        if id == TargetId::Harness {
            return self.harness_stored_request();
        }
        Ok(self.session(id)?.map(|session| session.request))
    }

    /// Stop managing a client without touching its config file. This is the way out
    /// of a conflict (the user edited managed fields) or an unreadable session record:
    /// the file keeps whatever the user has now, and a later apply starts from it.
    pub fn release(&self, id: TargetId) -> Result<(), String> {
        let _ = self.park(id, None);
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        if id == TargetId::Harness {
            return self.harness_release();
        }
        adapter(id)?;
        let journal = self.session_path(id);
        if journal.exists() {
            storage::remove_file_durable(&journal).map_err(|_| "clients.errors.storage")?;
        }
        Ok(())
    }

    /// Local-router URLs stop working with this process. Direct client
    /// configurations are persistent and remain in place across app restarts.
    /// Attempt each client independently so one conflict cannot strand another.
    /// A user's edit to a field other than the connection must not leave the
    /// client aimed at a dead router with the local token: the connection is
    /// withdrawn and the edit kept. What was withdrawn is kept to resume with
    /// ([`Self::resume_parked`]) once the router listens again.
    pub fn restore_router_configs_on_exit(&self) -> Vec<(TargetId, String)> {
        let harness = self.harness_stored_request().ok().flatten().filter(ConfigRequest::needs_router);
        [TargetId::ClaudeCode, TargetId::OpenCode, TargetId::Pi]
            .into_iter()
            .filter_map(|id| match self.session(id) {
                Ok(Some(session)) if session.request.needs_router() => {
                    match self.restore_with(id, ConflictPolicy::KeepEdits) {
                        Ok(applied) => {
                            if !applied.warnings.is_empty() {
                                log::info!("{id:?} router config withdrawn on exit; the user's own edits were kept");
                            }
                            self.park(id, Some(session.request)).err().map(|error| (id, error))
                        }
                        Err(error) => Some((id, error)),
                    }
                }
                Ok(_) => None,
                Err(error) => Some((id, error)),
            })
            .chain(match self.harness_restore_on_exit() {
                Some(Ok(())) => harness
                    .and_then(|request| self.park(TargetId::Harness, Some(request)).err())
                    .map(|error| (TargetId::Harness, error)),
                Some(Err(error)) => Some((TargetId::Harness, error)),
                None => None,
            })
            .collect()
    }

    fn prepare(
        &self,
        store: &Store,
        request: &ConfigRequest,
        rebaseline: bool,
        replace_conflicts: bool,
    ) -> Result<Plan, String> {
        let canonical = aggregate_models::canonical_request(store, request)?;
        let request = &canonical;
        let adapter = adapter(request.target_id)?;
        if request.aggregate_target().is_none() {
            let provider = selected_provider(store, request)?;
        if !request.accepts_model(provider, &request.model) {
            return Err("clients.errors.protocol".into());
            }
        }
        validate_model(&request.model)?;
        // Only adapters that consume limits require them. Claude Code ignores
        // context/output entirely, so catalog-less models stay injectable.
        if adapter.needs_limits()
            && (request
                .context_window
                .is_some_and(|value| !(1_024..=100_000_000).contains(&value))
                || request
                    .max_output_tokens
                    .is_some_and(|value| value == 0 || value > 100_000_000)
                || matches!(
                    (request.context_window, request.max_output_tokens),
                    (Some(context), Some(output)) if output > context
                ))
        {
            return Err("clients.errors.limits".into());
        }
        if request.needs_router() && store.router.bearer_token.is_empty() {
            return Err("clients.errors.router".into());
        }
        let mut previous = self.session(request.target_id)?;
        let path = previous
            .as_ref()
            .map(|s| s.path.as_path())
            .unwrap_or_else(|| adapter.path(&self.paths))
            .to_path_buf();
        let current = read_text(&path)?;
        let mut document = parse(current.as_deref(), request.target_id)?;
        // Element-keyed arrays are rebuilt from the file as it is now with this
        // app's previous elements withdrawn, so user rows added after the last
        // apply are adopted and rows we wrote are regenerated. The snapshot is
        // taken before undo, which resets arrays to the previous apply's state.
        let owned_recorded = owned::path(&self.data_dir, request.target_id).exists();
        let mut owned = if owned_recorded {
            owned::load(&self.data_dir, request.target_id)
        } else {
            // Pre-upgrade journals carry their additions in the recorded changes.
            previous
                .as_ref()
                .map(owned::OwnedRecord::seed_from_session)
                .unwrap_or_default()
        };
        owned.recover_from_session(previous.as_ref());
        // Each path: whether the file has it, and its baseline. A key holding
        // something that is not an array has no baseline: it is left
        // untouched for the adapter's validation to reject.
        let arrays: Vec<(Vec<String>, bool, Option<Vec<Value>>)> = owned::ELEMENT_PATHS
            .iter()
            .map(|path| {
                let path: Vec<String> = path.iter().map(|key| (*key).to_owned()).collect();
                let values = field(&document, &path);
                let baseline = values
                    .and_then(Value::as_array)
                    .map(|entries| owned.withdraw(&path, entries));
                (path, values.is_some(), baseline)
            })
            .collect();
        let independently_changed = previous.as_ref().is_some_and(|p| document != p.installed);
        // Read before the undo below, which puts the managed fields back.
        let conflicts: Vec<String> = previous
            .as_ref()
            .map(|session| {
                conflicting_paths(&document, session)
                    .iter()
                    .map(|path| path.join("."))
                    .collect()
            })
            .unwrap_or_default();
        let rebaselined = rebaseline && previous.is_some();
        if let Some(previous_session) = previous.as_mut() {
            if rebaselined {
                // Adopt externally changed fields as the new rollback point;
                // unchanged values still belong to our previous injection.
                // Both exact and merged restoration must use this same point.
                let conflicts = conflicting_paths(&document, previous_session);
                for change in &mut previous_session.changes {
                    if conflicts.contains(&change.path) {
                        change.before = field(&document, &change.path).cloned();
                    }
                }
            } else if !replace_conflicts {
                verify_fields(&document, previous_session)?;
            }
            undo(&mut document, previous_session)?;
        }
        let mut original = if independently_changed && !rebaselined {
            Some(render(request.target_id, current.as_deref(), &document)?)
        } else {
            previous
                .as_ref()
                .map(|session| session.original.clone())
                .unwrap_or_else(|| current.clone())
        };
        // A key the user deleted stays deleted; one that exists is rebuilt
        // without this app's previous elements. undo may have re-created a
        // deleted key from the journal, which the None-with-absent branches
        // reverse.
        for (path, present, baseline) in &arrays {
            match baseline {
                Some(entries) => set_field(&mut document, path, Some(json!(entries)))?,
                None if !present && field(&document, path).is_some() => {
                    set_field(&mut document, path, None)?
                }
                None => {}
            }
        }
        if rebaselined {
            original = Some(render(request.target_id, current.as_deref(), &document)?);
        } else if independently_changed {
            // The exact-byte restore path must retain the same adopted user
            // rows as the field-merge path. Build it after withdrawing our rows.
            let mut baseline = parse(original.as_deref(), request.target_id)?;
            for (path, _, _) in arrays.iter().filter(|(_, present, _)| *present) {
                let value = field(&document, path)
                    .filter(|value| {
                        !value.as_array().is_some_and(Vec::is_empty)
                            || field(&baseline, path).is_some()
                    })
                    .cloned();
                set_field(&mut baseline, path, value)?;
            }
            original = Some(render(request.target_id, original.as_deref(), &baseline)?);
        }
        let mut warnings = Vec::new();
        let direct_proxy = self.direct_proxy(store, request);
        match &direct_proxy {
            DirectProxy::Socks => warnings.push("clients.warnings.proxySocks".into()),
            DirectProxy::System => warnings.push("clients.warnings.systemProxy".into()),
            DirectProxy::None | DirectProxy::Write(_) | DirectProxy::Bypass(_) => {}
        }
        let edits = {
            let catalog = models_dev::load(&self.app_data_dir);
            let official = self
                .official_home
                .as_ref()
                .map(|home| model_catalog::official(home, &self.app_data_dir))
                .unwrap_or_default();
            let mut ctx = EditContext {
                // Legacy shape-based cleanup only where nothing else identifies
                // this app's rows: no journal and no ownership record.
                legacy_picker_cleanup: previous.is_none() && !owned_recorded,
                warnings: &mut warnings,
                managed_provider_keys: previous.as_ref().map(|session| {
                    session.changes.iter().filter_map(|change| {
                        (change.path.first().is_some_and(|key| key == "providers"))
                            .then(|| change.path.get(1).cloned()).flatten()
                    }).collect()
                }).unwrap_or_default(),
                proxy: match &direct_proxy {
                    DirectProxy::Write(url) => Some(url.clone()),
                    _ => None,
                },
                bypass: match &direct_proxy {
                    DirectProxy::Bypass(list) => Some(list.clone()),
                    _ => None,
                },
            };
            adapter.edits(
                &document,
                store,
                request,
                &catalog,
                &official.models,
                &mut ctx,
            )?
        };
        // `document` here is the file as it exists right now, before the edits
        // are applied — exactly what the takeover analysis compares against.
        if previous.is_none() {
            takeover::takeover_warnings(&document, &edits, &mut warnings);
        }
        if request.target_id == TargetId::ClaudeCode {
            takeover::managed_settings_conflicts(&edits, &mut warnings);
        }
        warnings.sort_unstable();
        warnings.dedup();
        let mut changes = Vec::new();
        for edit in edits {
            let before = field(&document, &edit.path).cloned();
            set_field(&mut document, &edit.path, edit.value.clone())?;
            changes.push(FieldChange {
                path: edit.path,
                before,
                after: edit.value,
            });
        }
        let session = Session {
            path,
            original,
            installed: document,
            changes,
            request: request.clone(),
            connection: (!request.needs_router())
                .then(|| connection_fingerprint(store, request, &direct_proxy)),
        };
        let mut hasher = Sha256::new();
        hasher.update(
            serde_json::to_vec(&(&current, &session, rebaseline, replace_conflicts))
                .map_err(|_| "clients.errors.storage")?,
        );
        Ok(Plan {
            revision: format!("{:x}", hasher.finalize()),
            current,
            session,
            conflicts,
            warnings,
        })
    }

    fn commit(
        &self,
        id: TargetId,
        path: &Path,
        current: Option<String>,
        next: Option<String>,
        session: Option<&Session>,
        warnings: Vec<String>,
    ) -> Result<ConfigApplied, String> {
        if read_text(path)? != current {
            return Err("clients.errors.changed".into());
        }
        let backup_path = self
            .data_dir
            .join("backups")
            .join(format!("{}.dat", uuid::Uuid::new_v4()));
        secret_file::write(
            &backup_path,
            &serde_json::to_vec(&current).map_err(|_| "clients.errors.storage")?,
        )?;
        storage::prune_backups(&self.data_dir.join("backups"), 20, |path| {
            path.extension().is_some_and(|extension| extension == "dat")
        });
        let previous_session = self.session(id)?;
        let journal = self.session_path(id);
        secret_file::write(
            &journal,
            &serde_json::to_vec(&session).map_err(|_| "clients.errors.storage")?,
        )?;
        let write_result = (|| -> Result<(), String> {
            if read_text(path)? != current {
                return Err("clients.errors.changed".into());
            }
            if let Some(text) = next {
                storage::write_atomic(path, text.as_bytes())
                    .map_err(|_| "clients.errors.storage")?;
            } else if path.exists() {
                std::fs::remove_file(path).map_err(|_| "clients.errors.storage")?;
            }
            Ok(())
        })();
        if let Err(error) = write_result {
            secret_file::write(
                &journal,
                &serde_json::to_vec(&previous_session).map_err(|_| "clients.errors.storage")?,
            )?;
            return Err(error);
        }
        // Update session.installed to match what was actually written and will be read back.
        // This prevents false conflict detection due to JSON serialization/deserialization differences.
        if let Some(mut updated_session) = session.cloned() {
            if let Ok(written_text) = read_text(path) {
                if let Ok(reparsed) = parse(written_text.as_deref(), id) {
                    updated_session.installed = reparsed;
                    // Update the changes' after values to match the reparsed document
                    for change in &mut updated_session.changes {
                        change.after = field(&updated_session.installed, &change.path).cloned();
                    }
                    let _ = secret_file::write(
                        &journal,
                        &serde_json::to_vec(&updated_session)
                            .map_err(|_| "clients.errors.storage")?,
                    );
                }
            }
        }
        // Record what this apply added to the element-keyed arrays so the next
        // apply withdraws exactly those. Restore keeps an empty record: no
        // owned rows remain, but the legacy migration must not run again.
        match session {
            Some(session) => owned::save(
                &self.data_dir,
                id,
                &owned::OwnedRecord::from_changes(&session.changes),
            ),
            None => owned::save(&self.data_dir, id, &owned::OwnedRecord::default()),
        }
        Ok(ConfigApplied {
            path: path.to_string_lossy().into(),
            backup_path: backup_path.to_string_lossy().into(),
            warnings,
        })
    }
}

/// The relay cards a direct configuration points at, as one fingerprint of
/// their addresses and keys and the proxy written for them; a deleted card
/// counts as changed.
fn connection_fingerprint(store: &Store, request: &ConfigRequest, proxy: &DirectProxy) -> String {
    let mut ids: Vec<&str> = std::iter::once(request.provider_id.as_str())
        .chain(request.options.provider_ids.iter().map(String::as_str))
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut hasher = Sha256::new();
    for id in ids {
        hasher.update(id.as_bytes());
        hasher.update([0]);
        match store.providers.iter().find(|provider| provider.id == id) {
            Some(provider) => {
                hasher.update(provider.base_url.trim().as_bytes());
                hasher.update([0]);
                hasher.update(provider.api_key.trim().as_bytes());
            }
            None => hasher.update(b"missing"),
        }
        hasher.update([1]);
    }
    // Absent from fingerprints written before proxies were.
    match proxy {
        DirectProxy::Write(url) => {
            hasher.update(b"proxy");
            hasher.update(url.as_bytes());
        }
        DirectProxy::Bypass(list) => {
            hasher.update(b"bypass");
            hasher.update(list.as_bytes());
        }
        _ => {}
    }
    format!("{:x}", hasher.finalize())
}

/// What a restore does with managed fields the user changed since the apply.
#[derive(Clone, Copy)]
enum ConflictPolicy {
    /// Refuse the restore (an ordinary restore before confirmation).
    Refuse,
    /// Put the baseline back over them (a confirmed restore).
    Replace,
    /// Keep them and restore everything else, unless another tool now owns
    /// the connection (the restore on exit).
    KeepEdits,
}

/// Whether a managed field carries the client's connection: an address on
/// this machine's router, or a credential.
fn carries_connection(change: &FieldChange) -> bool {
    let credential = change.path.last().is_some_and(|key| {
        let key: String = key.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        key.ends_with("apikey") || key.ends_with("token") || key.ends_with("authorization") || key.ends_with("baseurl")
    });
    credential || change.after.as_ref().is_some_and(|value| value.to_string().contains("http://127.0.0.1:"))
}

struct Plan {
    revision: String,
    current: Option<String>,
    session: Session,
    /// Managed paths the user changed since the last apply, by dotted name.
    conflicts: Vec<String>,
    /// Non-blocking notices computed while planning: first-injection takeover
    /// warnings and managed-settings overrides.
    warnings: Vec<String>,
}

pub fn has_router_configs(data_dir: &Path) -> Result<bool, String> {
    let manager = ConfigManager::new(data_dir, ClientPaths::from_home(Path::new(".")));
    for id in [TargetId::ClaudeCode, TargetId::OpenCode, TargetId::Pi] {
        match manager.session(id) {
            Ok(Some(session)) if session.request.needs_router() => return Ok(true),
            Ok(_) => {}
            // One unreadable record must not keep Codex routing down; the client list
            // reports it and offers release.
            Err(error) => log::warn!("skipping unreadable {id:?} client session: {error}"),
        }
    }
    match manager.harness_uses_router() {
        Ok(uses) => Ok(uses),
        Err(error) => {
            log::warn!("skipping unreadable Harness client session: {error}");
            Ok(false)
        }
    }
}

fn selected_provider<'a>(
    store: &'a Store,
    request: &ConfigRequest,
) -> Result<&'a Provider, String> {
    store
        .providers
        .iter()
        .find(|p| p.id == request.provider_id && p.has_route_credentials())
        .ok_or_else(|| "clients.errors.provider".into())
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("clients.errors.read".into()),
    }
}

fn parse(text: Option<&str>, id: TargetId) -> Result<Value, String> {
    let Some(text) = text else {
        return Ok(json!({}));
    };
    let text = text.trim_start_matches('\u{feff}');
    if id == TargetId::Pi && text.len() > 1024 * 1024 {
        return Err("clients.errors.invalidConfig".into());
    }
    let value: Value = if matches!(id, TargetId::OpenCode | TargetId::Pi) {
        json5::from_str(text).map_err(|_| "clients.errors.invalidConfig")?
    } else if id == TargetId::ClaudeCode {
        // Claude's settings.json is JSON, but other tools may have written
        // comments or trailing commas into it. Parse through json5 and write
        // back normalized JSON, same policy as OpenCode; the original text is
        // kept in the backup and restored verbatim when nothing else changed.
        serde_json::from_str(text)
            .or_else(|_| json5::from_str(text))
            .map_err(|_| "clients.errors.invalidConfig")?
    } else {
        serde_json::from_str(text).map_err(|_| "clients.errors.invalidConfig")?
    };
    if !value.is_object() {
        return Err("clients.errors.invalidConfig".into());
    }
    Ok(value)
}

/// The text `next` is written as, over `base` (the text the file held when
/// it was read). An OpenCode or Pi file with comments keeps them and its
/// layout; everything else is written as normalized JSON.
fn render(id: TargetId, base: Option<&str>, next: &Value) -> Result<String, String> {
    let normalized = format!(
        "{}\n",
        serde_json::to_string_pretty(next).map_err(|_| "clients.errors.storage")?
    );
    let syntax = match id {
        TargetId::OpenCode => jsonc::Syntax::OpenCode,
        TargetId::Pi => jsonc::Syntax::Pi,
        _ => return Ok(normalized),
    };
    // Strict JSON has no comments to keep: it stays normalized as before.
    let kept = base
        .filter(|base| serde_json::from_str::<Value>(base.trim_start_matches('\u{feff}')).is_err())
        .and_then(|base| jsonc::patch(base, syntax, next));
    Ok(kept.unwrap_or(normalized))
}

fn field<'a>(document: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(document, |value, key| value.get(key))
}

fn set_field(document: &mut Value, path: &[String], value: Option<Value>) -> Result<(), String> {
    let Some((key, rest)) = path.split_first() else {
        return Err("clients.errors.invalidConfig".into());
    };
    let object = document
        .as_object_mut()
        .ok_or("clients.errors.invalidConfig")?;
    if rest.is_empty() {
        if let Some(value) = value {
            object.insert(key.clone(), value);
        } else {
            object.remove(key);
        }
    } else if value.is_some() || object.contains_key(key) {
        let child = object.entry(key.clone()).or_insert_with(|| json!({}));
        set_field(child, rest, value)?;
    }
    Ok(())
}

fn verify_fields(document: &Value, session: &Session) -> Result<(), String> {
    if !conflicting_paths(document, session).is_empty() {
        return Err("clients.errors.conflict".into());
    }
    Ok(())
}

/// Managed paths changed to a value other than our last write or the baseline.
/// Returning a field to its baseline is an external restore, not a conflict.
/// Element-keyed arrays are excluded: additions and deletions by the user are
/// adopted on the next apply instead of being reported as conflicts.
fn conflicting_paths(document: &Value, session: &Session) -> Vec<Vec<String>> {
    session
        .changes
        .iter()
        .filter(|change| {
            !owned::is_element_array_path(&change.path)
                && !is_claude_model_selection(document, session, &change.path)
                && field(document, &change.path) != change.after.as_ref()
                && field(document, &change.path) != change.before.as_ref()
        })
        .map(|change| change.path.clone())
        .collect()
}

/// Claude Code persists /model selections in this field. Selecting one of
/// the installed models is normal client activity, not an ownership conflict.
/// Endpoint, credential and role-mapping fields still use strict checks.
fn is_claude_model_selection(document: &Value, session: &Session, path: &[String]) -> bool {
    if session.request.target_id != TargetId::ClaudeCode || path != ["model"] {
        return false;
    }
    let Some(current) = document.get("model").and_then(Value::as_str) else {
        return false;
    };
    fn unmarked(model: &str) -> &str {
        model
            .strip_suffix("[1m]")
            .or_else(|| model.strip_suffix("[1M]"))
            .unwrap_or(model)
    }
    let matches = |model: &str| unmarked(model) == unmarked(current);
    if matches(&session.request.model) {
        return true;
    }
    if let Some(selected) = &session.request.options.claude_model_ids {
        if selected.iter().any(|model| matches(model)) {
            return true;
        }
    }
    session
        .installed
        .pointer("/modelPicker/options")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry["model"].as_str())
                .any(matches)
        })
}

fn undo(document: &mut Value, session: &Session) -> Result<(), String> {
    let original = parse(session.original.as_deref(), session.request.target_id)?;
    for change in session.changes.iter().rev() {
        set_field(document, &change.path, change.before.clone())?;
        // Remove containers introduced by our leaf edits only when still empty.
        // Existing empty objects and siblings added by the user must survive.
        for depth in (1..change.path.len()).rev() {
            let parent = &change.path[..depth];
            if field(&original, parent).is_none()
                && field(document, parent)
                    .and_then(Value::as_object)
                    .is_some_and(|object| object.is_empty())
            {
                set_field(document, parent, None)?;
            }
        }
    }
    Ok(())
}

fn redact(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let normalized: String = key
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                if normalized.ends_with("apikey")
                    || normalized.ends_with("token")
                    || normalized.ends_with("authorization")
                    || normalized.ends_with("cookie")
                    || normalized.contains("secret")
                    || normalized.contains("password")
                    || normalized.contains("credential")
                    // A proxy address with a user name and password.
                    || (normalized.ends_with("proxy") && value.as_str().is_some_and(|url| url.contains('@')))
                {
                    *value = json!("[managed locally]");
                } else {
                    redact(value);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}
