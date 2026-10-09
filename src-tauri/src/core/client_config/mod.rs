//! Client-specific config adapters share one reversible file transaction boundary.
//! This module depends on provider data and the models.dev catalog (read from the
//! app data directory). An optional official snapshot source is supplied by
//! the caller, so model inputs match the Codex adapters without owning accounts.
mod aggregate_models;
mod claude;
mod claude_official;
mod opencode;
mod pi;
mod options;
mod owned;
mod takeover;
mod version;
pub use claude_official::ClaudeOfficialPreview;
use options::{client_endpoint, validate_model};
pub use options::{ClaudeAuthField, ClaudeModels, ClaudeOneM, ClientAccessMode, ClientOptions};
#[cfg(test)]
mod tests;

use super::{
    model_catalog, models_dev,
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
        }
    }

    pub fn from_home(home: &Path) -> Self {
        Self {
            claude: home.join(".claude/settings.json"),
            claude_user: home.join(".claude.json"),
            opencode: opencode_path(&home.join(".config/opencode")),
            pi: home.join(".pi/agent/models.json"),
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
        TargetId::Codex => Err("clients.errors.unsupported".into()),
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPreview {
    pub path: String,
    pub revision: String,
    /// Only the managed fields are returned; all credential values are redacted.
    pub content: String,
    pub removed_fields: Vec<String>,
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
}

pub struct ConfigManager {
    data_dir: PathBuf,
    /// App data directory, used to read the models.dev catalog snapshot.
    app_data_dir: PathBuf,
    official_home: Option<PathBuf>,
    paths: ClientPaths,
}

impl ConfigManager {
    pub fn new(data_dir: &Path, paths: ClientPaths) -> Self {
        Self {
            data_dir: data_dir.join("client-config"),
            app_data_dir: data_dir.to_path_buf(),
            official_home: None,
            paths,
        }
    }

    pub fn with_official_home(mut self, home: &Path) -> Self {
        self.official_home = Some(home.to_path_buf());
        self
    }

    pub fn list(&self) -> Vec<ClientStatus> {
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
                        let current = parse(read_text(&session.path)?.as_deref(), id)?;
                        let conflict_paths = conflicting_paths(&current, session);
                        status.conflict = !conflict_paths.is_empty();
                        status.conflict_fields = conflict_paths
                            .iter()
                            .map(|path| path.join("."))
                            .collect();
                    } else {
                        parse(read_text(adapter.path(&self.paths))?.as_deref(), id)?;
                    }
                    Ok(())
                })();
                status.error = result.err();
                status
            })
            .collect()
    }

    fn session_path(&self, id: TargetId) -> PathBuf {
        self.data_dir.join(match id {
            TargetId::ClaudeCode => "claude-code.dat",
            TargetId::OpenCode => "opencode.dat",
            TargetId::Pi => "pi.dat",
            TargetId::Codex => "codex.dat",
        })
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
        let plan = self.prepare(store, request, false, replace_conflicts)?;
        if plan.revision != expected_revision {
            return Err("clients.errors.changed".into());
        }
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&plan.session.installed)
                .map_err(|_| "clients.errors.storage")?
        );
        self.commit(
            request.target_id,
            &plan.session.path,
            plan.current,
            Some(text),
            Some(&plan.session),
            plan.warnings,
        )
    }

    pub fn restore(&self, id: TargetId) -> Result<ConfigApplied, String> {
        self.restore_confirmed(id, false)
    }

    /// Only an explicit restore confirmation may replace conflicting managed
    /// fields. Automatic exit restoration always uses the strict wrapper.
    pub fn restore_confirmed(
        &self,
        id: TargetId,
        replace_conflicts: bool,
    ) -> Result<ConfigApplied, String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
        adapter(id)?;
        let session = self.session(id)?.ok_or("clients.errors.notConfigured")?;
        let current = read_text(&session.path)?;
        let document = parse(current.as_deref(), id)?;
        if !replace_conflicts {
            verify_fields(&document, &session)?;
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
            owned.recover_picker_entries(Some(&session));
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
            Some(format!(
                "{}\n",
                serde_json::to_string_pretty(&document).map_err(|_| "clients.errors.storage")?
            ))
        };
        self.commit(id, &session.path, current, restored, None, Vec::new())
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
        adapter(id)?;
        let session = self.session(id)?.ok_or("clients.errors.notConfigured")?;
        let plan = self.prepare(store, &session.request, rebaseline, false)?;
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&plan.session.installed)
                .map_err(|_| "clients.errors.storage")?
        );
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
        Ok(self.session(id)?.map(|session| session.request))
    }

    /// Stop managing a client without touching its config file. This is the way out
    /// of a conflict (the user edited managed fields) or an unreadable session record:
    /// the file keeps whatever the user has now, and a later apply starts from it.
    pub fn release(&self, id: TargetId) -> Result<(), String> {
        let _guard = WRITE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| "clients.errors.storage")?;
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
    pub fn restore_router_configs_on_exit(&self) -> Vec<(TargetId, String)> {
        [TargetId::ClaudeCode, TargetId::OpenCode, TargetId::Pi]
            .into_iter()
            .filter_map(|id| match self.session(id) {
                Ok(Some(session)) if session.request.needs_router() => {
                    self.restore(id).err().map(|error| (id, error))
                }
                Ok(_) => None,
                Err(error) => Some((id, error)),
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
        owned.recover_picker_entries(previous.as_ref());
        let picker_options = document
            .get("modelPicker")
            .and_then(|picker| picker.get("options"));
        let available_values = document.get("availableModels");
        let had_picker = picker_options.is_some();
        let had_available = available_values.is_some();
        // A key holding something that is not an array has no baseline: it is
        // left untouched for the adapter's validation to reject.
        let baseline_picker = picker_options
            .and_then(Value::as_array)
            .map(|entries| owned.withdraw(&["modelPicker".into(), "options".into()], entries));
        let baseline_available = available_values
            .and_then(Value::as_array)
            .map(|values| owned.withdraw(&["availableModels".into()], values));
        let independently_changed = previous.as_ref().is_some_and(|p| document != p.installed);
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
            Some(format!(
                "{}\n",
                serde_json::to_string_pretty(&document).map_err(|_| "clients.errors.storage")?
            ))
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
        match baseline_picker {
            Some(entries) => set_field(
                &mut document,
                &["modelPicker".into(), "options".into()],
                Some(json!(entries)),
            )?,
            None if !had_picker
                && field(&document, &["modelPicker".into(), "options".into()]).is_some() =>
            {
                set_field(
                    &mut document,
                    &["modelPicker".into(), "options".into()],
                    None,
                )?
            }
            None => {}
        }
        match baseline_available {
            Some(values) => set_field(
                &mut document,
                &["availableModels".into()],
                Some(json!(values)),
            )?,
            None if !had_available && field(&document, &["availableModels".into()]).is_some() => {
                set_field(&mut document, &["availableModels".into()], None)?
            }
            None => {}
        }
        if rebaselined {
            original = Some(format!(
                "{}\n",
                serde_json::to_string_pretty(&document).map_err(|_| "clients.errors.storage")?
            ));
        } else if independently_changed {
            // The exact-byte restore path must retain the same adopted user
            // rows as the field-merge path. Build it after withdrawing our rows.
            let mut baseline = parse(original.as_deref(), request.target_id)?;
            for (present, path) in [
                (had_picker, vec!["modelPicker".into(), "options".into()]),
                (had_available, vec!["availableModels".into()]),
            ] {
                if present {
                    let value = field(&document, &path)
                        .filter(|value| {
                            !value.as_array().is_some_and(Vec::is_empty)
                                || field(&baseline, &path).is_some()
                        })
                        .cloned();
                    set_field(&mut baseline, &path, value)?;
                }
            }
            original = Some(format!(
                "{}\n",
                serde_json::to_string_pretty(&baseline).map_err(|_| "clients.errors.storage")?
            ));
        }
        let mut warnings = Vec::new();
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

struct Plan {
    revision: String,
    current: Option<String>,
    session: Session,
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
    Ok(false)
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
